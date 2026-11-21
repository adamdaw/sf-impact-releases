use crate::{
    Config, eligible,
    graph::{Graph, Link, Step, Via},
};
use sf_impact_common::rows::{RefKind, Tier, fold};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone)]
pub struct Selection {
    pub reasons: BTreeSet<&'static str>,
    /// The inner test classes whose names matched a seed's class by
    /// convention. Only one witness is kept, and a convention witness has no
    /// path, so nothing else would name them.
    pub convention: BTreeSet<usize>,
    pub seed: usize,
    pub path: Vec<Step>,
}
#[derive(Default)]
pub struct Walk {
    pub selected: BTreeMap<usize, Selection>,
    pub possible: BTreeMap<usize, Selection>,
    pub reached: BTreeSet<usize>,
    pub tested_classes: BTreeSet<usize>,
    pub untested: BTreeSet<usize>,
    pub triggers: BTreeSet<usize>,
    pub tested_triggers: BTreeSet<usize>,
    pub bindings: BTreeSet<usize>,
    pub tested_bindings: BTreeSet<usize>,
}
fn record<'a>(
    selections: &'a mut BTreeMap<usize, Selection>,
    test: usize,
    seed: usize,
    reason: &'static str,
    path: &[Step],
) -> &'a mut Selection {
    let found = selections.entry(test).or_insert_with(|| Selection {
        reasons: BTreeSet::new(),
        convention: BTreeSet::new(),
        seed,
        path: path.to_vec(),
    });
    found.reasons.insert(reason);
    if (path.len(), seed, path) < (found.path.len(), found.seed, found.path.as_slice()) {
        found.seed = seed;
        found.path = path.to_vec();
    }
    found
}
impl Walk {
    pub fn seed(
        &mut self,
        graph: &Graph,
        seed: usize,
        direct: &[Link],
        depth: usize,
        config: &Config,
    ) {
        let class = graph.class_owner(seed);
        let mut tested = false;
        if let Some(test) = graph.test_owner(seed)
            && eligible(&graph.nodes[test], config)
        {
            record(&mut self.selected, test, seed, "changed", &[]);
            tested = true;
        }
        if let Some(class) = class {
            for &test in &graph.tests {
                if eligible(&graph.nodes[test], config)
                    && config.test_suffixes.iter().any(|suffix| {
                        fold(&graph.nodes[test].symbol)
                            == fold(&format!("{}{suffix}", graph.nodes[class].symbol))
                    })
                {
                    let run = graph.top_level(test);
                    let selection = record(&mut self.selected, run, seed, "convention", &[]);
                    if run != test {
                        selection.convention.insert(test);
                    }
                    tested = true;
                }
            }
        }
        for link in direct {
            if binding(graph, link) {
                self.bindings.insert(link.edge);
            }
        }
        // A strong and a weak route to the same node are distinct states.
        // Weak routes are walked only to make possible tests visible.
        let mut visited = BTreeSet::from([(seed, false)]);
        let mut queue = VecDeque::from([(seed, Vec::<Step>::new(), false)]);
        while let Some((node, path, weak)) = queue.pop_front() {
            if !weak {
                self.reached.insert(node);
                if graph.nodes[node].kind == "trigger" {
                    self.triggers.insert(node);
                }
            }
            if path.len() >= depth {
                continue;
            }
            let links = if path.is_empty() {
                direct.to_vec()
            } else {
                graph.incoming_to(node, false)
            };
            for link in links {
                let edge = &graph.edges[link.edge];
                let textual = edge.reference.tier == Tier::Textual;
                let next_weak = weak || textual;
                for &source in &edge.sources {
                    let mut next = path.clone();
                    next.push(Step {
                        link: link.clone(),
                        source,
                    });
                    if !next_weak {
                        self.reached.insert(source);
                        self.reached.insert(link.target);
                        if graph.nodes[source].kind == "trigger" {
                            self.triggers.insert(source);
                        }
                    }
                    if let Some(test) = graph.test_owner(source)
                        && eligible(&graph.nodes[test], config)
                    {
                        let lwc_textual = next_weak && graph.nodes[seed].kind == "lwc";
                        let member_name_only =
                            textual && next.len() == 1 && graph.member_name_only(seed, &link);
                        if !next_weak || (next.len() == 1 && !lwc_textual && !member_name_only) {
                            let trigger_dml = next.iter().any(|s| s.link.via == Via::TriggerDml);
                            let reason = if trigger_dml {
                                "trigger_dml"
                            } else if next.iter().any(|s| s.link.via == Via::PlatformEntry) {
                                "platform_entry"
                            } else if next.len() == 1 {
                                "direct"
                            } else {
                                "caller"
                            };
                            record(&mut self.selected, test, seed, reason, &next);
                            tested = true;
                            for step in &next {
                                if step.link.via == Via::TriggerDml {
                                    self.tested_triggers.insert(step.link.target);
                                }
                                if binding(graph, &step.link) {
                                    self.tested_bindings.insert(step.link.edge);
                                }
                            }
                        } else {
                            record(
                                &mut self.possible,
                                test,
                                seed,
                                if lwc_textual {
                                    "lwc_textual"
                                } else if member_name_only {
                                    "member_name_only"
                                } else {
                                    "textual_path"
                                },
                                &next,
                            );
                        }
                    }
                    // @IsTest utility classes are callable helpers as well as
                    // selection candidates. They do not terminate traversal.
                    if visited.insert((source, next_weak)) {
                        queue.push_back((source, next, next_weak));
                    }
                }
            }
        }
        if let Some(class) = class {
            if tested {
                self.tested_classes.insert(class);
            } else if !graph.tests.contains(&class) {
                self.untested.insert(class);
            }
        }
    }
    pub fn finish(&mut self) {
        self.possible
            .retain(|id, _| !self.selected.contains_key(id));
        self.untested.retain(|id| !self.tested_classes.contains(id));
        self.untested
            .extend(self.triggers.difference(&self.tested_triggers));
    }
}
fn binding(graph: &Graph, link: &Link) -> bool {
    link.via == Via::OwnerBinding
        || matches!(
            graph.edges[link.edge].reference.kind,
            RefKind::Cmdt | RefKind::CmdtField | RefKind::FlowAction
        )
}
