//! Render one verified report, without a repository or source tree.
mod document;
mod validate;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// UTF-8 bytes, including markup and the marker; below GitHub's 65,536 characters.
pub const COMMENT_BUDGET: usize = 60_000;
const ARTIFACT: &str = "\nFull evidence: impact.json artifact.\n";

pub fn render(input: &[u8], format: &str) -> Result<String, String> {
    let doc =
        validate::read(input).map_err(|e| format!("impact.json: {e}; re-run sf-impact-tests"))?;
    match format {
        "md" => Ok(markdown(&doc)),
        "gh-step" => step(&doc),
        _ => Err(format!("unknown format {format:?}; use md or gh-step")),
    }
}
fn rows<'a>(doc: &'a Value, key: &str) -> &'a [Value] {
    doc[key].as_array().expect("validated array")
}
fn text(value: &Value) -> &str {
    value.as_str().expect("validated string")
}
/// GitHub does not expand mentions, references, emoji or HTML in code spans.
/// A longer fence prevents arbitrary input from ending the span. Replace line
/// breaks and directional controls, which could misrepresent the surrounding text.
fn code(value: &str) -> String {
    let value: String = value.chars().map(|c| {
        if c.is_control() || matches!(c, '\u{061c}' | '\u{200e}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}') { '�' } else { c }
    }).collect();
    let longest = value.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest + 1);
    let pad = if value.starts_with('`') || value.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{value}{pad}{fence}")
}
fn coded(value: &Value) -> String {
    code(text(value))
}
fn location(node: &Value) -> String {
    let line = node["line"]
        .as_u64()
        .map_or_else(|| "unknown".into(), |n| n.to_string());
    code(&format!("{}:{line}", text(&node["file"])))
}
fn test_line(test: &Value) -> String {
    let reasons = rows(test, "reasons")
        .iter()
        .map(coded)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "- {} ({}): {reasons}; seed {}\n",
        coded(&test["symbol"]),
        location(test),
        coded(&test["seed"]["symbol"])
    )
}
fn heading(title: &str, count: usize) -> String {
    format!("\n### {title} ({count})\n\n")
}

struct Section {
    heading: String,
    // The weight counts original records, even where one row groups many reasons.
    rows: Vec<(String, usize)>,
    footer: &'static str,
}
impl Section {
    fn render(&self, cap: usize) -> String {
        let mut out = self.heading.clone();
        let mut remaining: usize = self.rows.iter().map(|(_, n)| n).sum();
        let notice = |n| {
            if n == 0 {
                String::new()
            } else {
                format!("\n**{n} more not shown**; see impact.json artifact.\n")
            }
        };
        for (row, count) in &self.rows {
            if out.len() + row.len() + notice(remaining - count).len() + self.footer.len() > cap {
                break;
            }
            out.push_str(row);
            remaining -= count;
        }
        if self.rows.is_empty() {
            out.push_str("None.\n");
        }
        out.push_str(&notice(remaining));
        out.push_str(self.footer);
        out
    }
}
fn reason_section(doc: &Value) -> Section {
    let mut groups: BTreeMap<(&str, &str, &str), Vec<Option<u64>>> = BTreeMap::new();
    for gap in rows(doc, "gaps") {
        groups
            .entry((
                text(&gap["reason"]),
                text(&gap["source"]),
                text(&gap["file"]),
            ))
            .or_default()
            .push(gap["line"].as_u64());
    }
    Section {
        heading: format!(
            "\n### Partial confidence ({} reasons)\n\nGrouped by kind, input and file. Full list: impact.json artifact.\n\n",
            rows(doc, "gaps").len()
        ),
        rows: groups
            .into_iter()
            .map(|((kind, source, file), mut lines)| {
                lines.sort();
                let first = lines
                    .iter()
                    .take(5)
                    .map(|line| line.map_or_else(|| "unknown".into(), |n| n.to_string()))
                    .collect::<Vec<_>>()
                    .join(", ");
                (
                    format!(
                        "- {} ({source}), {}: {} reasons; first lines: {first}\n",
                        code(kind),
                        code(file),
                        lines.len()
                    ),
                    lines.len(),
                )
            })
            .collect(),
        footer: "",
    }
}
fn markdown(doc: &Value) -> String {
    let head = doc["provenance"]["inputs"]["changed"]["head"]
        .as_str()
        .or_else(|| doc["provenance"]["inputs"]["refs"]["tree"]["commit"].as_str())
        .map_or_else(|| "unknown (no commit in input)".into(), code);
    let summary = format!(
        "<!-- sf-impact -->\n## Salesforce impact\n\n| Head commit | Confidence | Selected tests | Untested symbols | Possible tests | Changed symbols |\n| --- | --- | ---: | ---: | ---: | ---: |\n| {head} | {} | {} | {} | {} | {} |\n",
        text(&doc["confidence"]),
        rows(doc, "tests").len(),
        rows(doc, "untested").len(),
        rows(doc, "possible").len(),
        rows(doc, "seeds").len()
    );
    let reasons = reason_section(doc);
    let tests = Section {
        heading: heading("Selected tests", rows(doc, "tests").len()),
        rows: rows(doc, "tests")
            .iter()
            .map(|t| (test_line(t), 1))
            .collect(),
        footer: if rows(doc, "tests").is_empty() {
            "No tests selected; do not pass an empty tests value to RunSpecifiedTests.\n"
        } else {
            ""
        },
    };
    // Independent caps preserve the review order without starving later sections.
    // Each is a ceiling: these sections make room for selected tests, below.
    let sections = [
        (
            Section {
                heading: heading("Untested changed symbols", rows(doc, "untested").len()),
                rows: rows(doc, "untested")
                    .iter()
                    .map(|n| (format!("- {} ({})\n", coded(&n["symbol"]), location(n)), 1))
                    .collect(),
                footer: "",
            },
            4_000,
        ),
        (
            Section {
                heading: heading("Changed symbols", rows(doc, "seeds").len()),
                rows: rows(doc, "seeds")
                    .iter()
                    .map(|s| {
                        (
                            format!(
                                "- {} ({}): {} parsed, {} textual\n",
                                coded(&s["symbol"]),
                                location(s),
                                s["counts"]["parsed"],
                                s["counts"]["textual"]
                            ),
                            1,
                        )
                    })
                    .collect(),
                footer: "",
            },
            6_000,
        ),
        (
            Section {
                heading: format!(
                    "\n<details>\n<summary>Possible tests ({}) — not selected</summary>\n\n",
                    rows(doc, "possible").len()
                ),
                rows: rows(doc, "possible")
                    .iter()
                    .map(|t| (test_line(t), 1))
                    .collect(),
                footer: "\n</details>\n",
            },
            6_000,
        ),
        (
            Section {
                heading: "\n<details>\n<summary>Direct referrers (parsed first)</summary>\n\n"
                    .into(),
                rows: referrers(doc),
                footer: "\n</details>\n",
            },
            6_000,
        ),
    ];
    let mut secondary: Vec<String> = sections.iter().map(|(s, cap)| s.render(*cap)).collect();
    let fixed = summary.len() + ARTIFACT.len();
    let available = COMMENT_BUDGET - fixed - secondary.iter().map(String::len).sum::<usize>();
    let mut reason_text = if doc["confidence"] == "partial" {
        reasons.render(usize::MAX)
    } else {
        String::new()
    };
    let mut test_text = tests.render(usize::MAX);
    if reason_text.len() + test_text.len() > available {
        // Even unusually long names or thousands of distinct reason files must
        // yield a fresh comment. Whole omitted records remain in the artifact.
        if reason_text.len() > available / 2 {
            reason_text = reasons.render(available / 2);
        }
        // The sections below the selected tests give up their rows before a
        // selected test is left out, the last section first. Rendered with no
        // room, a section is its heading and the count of what it does not
        // show: that much is kept for each of them.
        let floors: Vec<usize> = sections.iter().map(|(s, _)| s.render(0).len()).collect();
        let mut reserved: usize = floors.iter().sum();
        let room = COMMENT_BUDGET - fixed - reason_text.len() - reserved;
        if test_text.len() > room {
            test_text = tests.render(room);
        }
        let mut left = COMMENT_BUDGET - fixed - reason_text.len() - test_text.len();
        for (i, (section, _)) in sections.iter().enumerate() {
            reserved -= floors[i];
            if secondary[i].len() > left - reserved {
                secondary[i] = section.render(left - reserved);
            }
            left -= secondary[i].len();
        }
    }
    let secondary = secondary.concat();
    format!("{summary}{reason_text}{test_text}{secondary}{ARTIFACT}")
}
fn referrers(doc: &Value) -> Vec<(String, usize)> {
    let mut result = Vec::new();
    for tier in ["parsed", "textual"] {
        for seed in rows(doc, "seeds") {
            for group in rows(seed, "referrers") {
                for hit in rows(group, tier) {
                    result.push((
                        format!(
                            "- {} ← {tier} {} {} ({})\n",
                            coded(&seed["symbol"]),
                            coded(&group["type"]),
                            coded(&hit["referrer"]["symbol"]),
                            coded(&hit["referrer"]["file"])
                        ),
                        1,
                    ));
                }
            }
        }
    }
    result
}
fn names<'a>(doc: &'a Value, key: &str) -> BTreeSet<&'a str> {
    rows(doc, key)
        .iter()
        .map(|test| text(&test["symbol"]))
        .collect()
}
fn step(doc: &Value) -> Result<String, String> {
    let names = names(doc, "tests");
    for name in &names {
        // Apex identifiers start with an ASCII letter; underscores may occur
        // between alphanumerics, never consecutively or at the end.
        if !name.starts_with(|c: char| c.is_ascii_alphabetic())
            || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            || name.ends_with('_')
            || name.contains("__")
        {
            return Err(format!(
                "invalid Apex class name {name:?} for RunSpecifiedTests"
            ));
        }
    }
    let mut out = String::new();
    output_value(
        &mut out,
        "tests",
        &names.iter().copied().collect::<Vec<_>>().join(","),
    );
    output_value(&mut out, "confidence", text(&doc["confidence"]));
    output_value(&mut out, "selected_count", &names.len().to_string());
    output_value(
        &mut out,
        "possible_count",
        &self::names(doc, "possible").len().to_string(),
    );
    output_value(
        &mut out,
        "untested_count",
        &rows(doc, "untested").len().to_string(),
    );
    output_value(
        &mut out,
        "selection",
        if names.is_empty() {
            "empty"
        } else {
            "selected"
        },
    );
    Ok(out)
}
fn output_value(out: &mut String, key: &str, value: &str) {
    if value.contains(['\n', '\r']) {
        let mut delimiter = "SF_IMPACT_OUTPUT".to_string();
        let mut suffix = 0;
        while value.split(['\n', '\r']).any(|line| line == delimiter) {
            suffix += 1;
            delimiter = format!("SF_IMPACT_OUTPUT_{suffix}");
        }
        out.push_str(&format!("{key}<<{delimiter}\n{value}\n{delimiter}\n"));
    } else {
        out.push_str(&format!("{key}={value}\n"));
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn multiline_values_use_a_collision_free_delimiter() {
        let mut out = String::new();
        super::output_value(
            &mut out,
            "value",
            "A\r\nSF_IMPACT_OUTPUT\nSF_IMPACT_OUTPUT_1\nB",
        );
        assert_eq!(
            out,
            "value<<SF_IMPACT_OUTPUT_2\nA\r\nSF_IMPACT_OUTPUT\nSF_IMPACT_OUTPUT_1\nB\nSF_IMPACT_OUTPUT_2\n"
        );
    }
}
