//! The files that differ, as the rows of `changed.jsonl`: each file itself,
//! the symbols its change adds, modifies and removes, and the names in the
//! entries it touches.
//!
//! A symbol is what `refs.jsonl` declares at head, or what the base version
//! of a file declares. The same declaration at base and at head is one
//! that has the same package, kind of thing and name. The delta pairs no
//! two files that differ, so a file that moved and changed is there as one
//! deleted and one added: where the two declare the same symbol, they are
//! read here as one file, and only what differs between them is changed.

use crate::delta::{Blob, Change, Content, Delta};
use crate::head::Head;
use sf_impact_common::changed::Change::{Added, Modified, Removed};
use sf_impact_common::changed::{
    self, ChangedStatus, Confidence, File, Gap, Hunk, Reason, Row, Side, Symbol, Touched,
};
use sf_impact_common::lines;
use sf_impact_common::rows::{self, DeclType, Declaration, RefKind, Reference, Resolution, fold};
use sf_impact_common::scope::{PROJECT_FILE, ScopeFile, package_of};
use sf_impact_extract::declarative;
use std::collections::{BTreeSet, HashMap, HashSet};

pub struct Mapper<'a> {
    pub delta: &'a Delta,
    /// The scope directories at base and at head.
    pub base_dirs: &'a [String],
    pub head_dirs: &'a [String],
    /// The two are not the same directories.
    pub scope_changed: bool,
    pub head: &'a Head,
}

/// One version of a changed file that is in scope.
#[derive(Clone, Copy)]
struct Version<'a> {
    path: &'a str,
    blob: &'a Blob,
    package: &'a str,
}

fn version<'a>(blob: &'a Option<Blob>, dirs: &'a [String]) -> Option<Version<'a>> {
    let blob = blob.as_ref()?;
    Some(Version {
        path: &blob.path,
        blob,
        package: package_of(dirs, &blob.path)?,
    })
}

/// What a changed file comes to, apart from its own row.
#[derive(Default)]
struct Found {
    rows: Vec<Row>,
    gaps: BTreeSet<Gap>,
}

/// What a file is, by its path in its package: the symbols it declares at
/// the top, or the Apex file it is the companion of. A deleted file and an
/// added file that are the same by it are one file that moved.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key {
    Symbol(String, DeclType, String),
    CompanionOf(String, String),
}

impl Mapper<'_> {
    /// The rows of every changed file: its `file` row and what it comes to.
    pub fn rows(&self) -> Vec<Row> {
        let changes = &self.delta.changes;
        let partners = self.partners(changes);
        let mut rows = Vec::new();
        for (change, partner) in changes.iter().zip(&partners) {
            match partner {
                // Its rows come with those of the file it went on in.
                Some(_) if change.status == ChangedStatus::Deleted => {}
                Some(deleted) => rows.extend(self.moved_and_changed(&changes[*deleted], change)),
                None => {
                    let found = self.found(change);
                    rows.push(self.file_row(change, &found.gaps));
                    rows.extend(found.rows);
                }
            }
        }
        rows
    }

    /// For each change, the other half of it: for a deleted file the added
    /// file that it went on in, and the reverse. They are one file if they
    /// are the same thing by their keys, and nothing else is.
    fn partners(&self, changes: &[Change]) -> Vec<Option<usize>> {
        let mut halves: HashMap<Key, (Vec<usize>, Vec<usize>)> = HashMap::new();
        for (at, change) in changes.iter().enumerate() {
            match change.status {
                ChangedStatus::Deleted => {
                    let old = version(&change.old, self.base_dirs);
                    for key in old.into_iter().flat_map(|old| self.keys(old, Side::Base)) {
                        halves.entry(key).or_default().0.push(at);
                    }
                }
                ChangedStatus::Added => {
                    let new = version(&change.new, self.head_dirs);
                    for key in new.into_iter().flat_map(|new| self.keys(new, Side::Head)) {
                        halves.entry(key).or_default().1.push(at);
                    }
                }
                _ => {}
            }
        }
        // One deleted and one added: a pair. More of either, or a file in
        // two pairs: which went on in which is not known, and none is
        // chosen. They are then removed and added, which leaves nothing out.
        let mut partners: Vec<Option<usize>> = vec![None; changes.len()];
        let mut unsure: HashSet<usize> = HashSet::new();
        for (deleted, added) in halves.values() {
            match (&deleted[..], &added[..]) {
                ([], _) | (_, []) => {}
                ([deleted], [added]) => {
                    for (one, other) in [(*deleted, *added), (*added, *deleted)] {
                        match partners[one].replace(other) {
                            Some(before) if before != other => unsure.extend([one, other, before]),
                            _ => {}
                        }
                    }
                }
                _ => unsure.extend(deleted.iter().chain(added).copied()),
            }
        }
        let sure = |at: usize| !unsure.contains(&at);
        let paired = partners.iter().enumerate().map(|(one, other)| {
            other.filter(|other| sure(one) && sure(*other) && partners[*other] == Some(one))
        });
        paired.collect()
    }

    /// What a version of a file is, by its path in its package.
    fn keys(&self, version: Version, side: Side) -> Vec<Key> {
        let package = version.package.to_string();
        match Kind::of(version.path) {
            Kind::Companion => vec![Key::CompanionOf(package, owner(version).1)],
            Kind::Declares => {
                let declarations = match side {
                    Side::Head => self.head.declarations(version.path).to_vec(),
                    Side::Base => {
                        let read = self.declared_at_base(version, &mut Found::default());
                        read.map_or(Vec::new(), |(declarations, _)| declarations)
                    }
                };
                let top = outermost(&declarations).into_iter();
                top.map(|at| &declarations[at])
                    .map(|d| Key::Symbol(package.clone(), d.kind, fold(&d.symbol)))
                    .collect()
            }
            Kind::Bundle | Kind::Unmodelled => Vec::new(),
        }
    }

    /// The rows of a deleted file and of the added file it went on in:
    /// their own, and those of the one file they are, compared line by
    /// line here, since the delta compares no two files.
    fn moved_and_changed(&self, deleted: &Change, added: &Change) -> Vec<Row> {
        let (old, new) = (deleted.old.as_ref(), added.new.as_ref());
        let texts = old.zip(new).and_then(|(old, new)| {
            let is_text = !old.binary && !new.binary;
            let texts = self.delta.text_of(old).zip(self.delta.text_of(new));
            texts.filter(|_| is_text)
        });
        let joined = Change {
            status: ChangedStatus::Modified,
            old: deleted.old.clone(),
            new: added.new.clone(),
            content: match texts {
                Some((old, new)) => Content::Lines(lines::hunks(old.as_bytes(), new.as_bytes())),
                None => Content::Binary,
            },
            new_directory: added.new_directory.clone(),
        };
        let mut found = self.found(&joined);
        // A symbol that went on says where it was: its base lines are
        // lines of the deleted file.
        let (from, to) = (old.map(|old| &old.path), new.map(|new| &new.path));
        for row in &mut found.rows {
            if let Row::Symbol(symbol) = row
                && symbol.change == Modified
                && Some(&symbol.file) == to
            {
                symbol.previous_file = from.cloned();
            }
        }
        // Each reading bears on the rows of both: what is removed from the
        // one is what is not found in the other.
        let files = [added, deleted].map(|change| self.file_row(change, &found.gaps));
        files.into_iter().chain(found.rows).collect()
    }

    /// What one changed file comes to.
    fn found(&self, change: &Change) -> Found {
        let old = version(&change.old, self.base_dirs);
        let new = version(&change.new, self.head_dirs);
        let mut found = Found::default();
        match (old, new) {
            // A change of mode changes nothing that is read.
            _ if change.status == ChangedStatus::ModeChanged => {}
            // Moved from one kind of file to another: the two versions
            // are read in different ways and have nothing to compare. What
            // the old one was is gone, and what the new one is has come.
            (Some(was), Some(now)) if Kind::of(was.path) != Kind::of(now.path) => {
                self.map(change, old, None, &mut found);
                self.map(change, None, new, &mut found);
            }
            _ => self.map(change, old, new, &mut found),
        }
        found
    }

    /// The row of a changed file itself.
    fn file_row(&self, change: &Change, gaps: &BTreeSet<Gap>) -> Row {
        let old = version(&change.old, self.base_dirs);
        let new = version(&change.new, self.head_dirs);
        let at = |blob: &Option<Blob>| blob.as_ref().map(|blob| blob.path.clone());
        let mut gaps = gaps.clone();
        // Files that did not change are in scope, out of it or in another
        // package from here on, and no row can say which: only the file
        // that moved the scope is in the delta.
        let is_project_file = [&change.old, &change.new]
            .iter()
            .any(|blob| at(blob).as_deref() == Some(PROJECT_FILE));
        if self.scope_changed && is_project_file {
            gaps.insert(Gap::ScopeChanged);
        }
        Row::File(File {
            file: at(&change.new)
                .or_else(|| at(&change.old))
                .unwrap_or_default(),
            previous_file: at(&change.old).filter(|_| change.status == ChangedStatus::Moved),
            status: change.status,
            binary: change.content == Content::Binary,
            package: new.or(old).map(|version| version.package.to_string()),
            confidence: match gaps.is_empty() {
                true => Confidence::Full,
                false => Confidence::Partial,
            },
            gaps: gaps.into_iter().collect(),
        })
    }

    /// A changed file, by what kind of file it is. `old` and `new` are its
    /// versions that are in scope: one, or two of the same kind.
    fn map(&self, change: &Change, old: Option<Version>, new: Option<Version>, found: &mut Found) {
        let Some(either) = new.or(old) else {
            return;
        };
        match Kind::of(either.path) {
            Kind::Bundle => self.bundle(change, old, new, found),
            Kind::Companion => self.companion(change, old, new, found),
            Kind::Unmodelled => {
                found.gaps.insert(Gap::UnmodelledFile);
            }
            Kind::Declares => {
                self.declared(change, old, new, found);
                self.touched(change, old, new, found);
            }
        }
    }

    /// A file that declares what is in it: Apex, or a metadata file that
    /// is one component, or the labels. What the base version declares and
    /// the head does not is removed; what the head declares is added if the
    /// base did not declare it, and modified if the change marks it.
    fn declared(
        &self,
        change: &Change,
        old: Option<Version>,
        new: Option<Version>,
        found: &mut Found,
    ) {
        // A base version that could not be read declared what is not
        // known. Nothing at head is then taken to be new.
        let base = match old {
            Some(old) => self.declared_at_base(old, found),
            None => Some((Vec::new(), Vec::new())),
        };
        let is_base_known = base.is_some();
        let (was, was_alone) = base.unwrap_or_default();
        let now = new.map_or(&[][..], |new| self.head.declarations(new.path));
        if new.is_some_and(|new| self.head.has_parse_error(new.path)) {
            found.gaps.insert(Gap::ParseError);
        }
        // What the Flow names is not all known: what it depends on may be
        // what changed.
        if new.is_some_and(|new| self.head.is_flow_not_read(new.path)) {
            found.gaps.insert(Gap::UnsupportedElement);
        }
        if was.is_empty() && now.is_empty() {
            found.gaps.insert(Gap::NoDeclaration);
        }

        let identity = |d: &Declaration| (d.package.clone(), d.kind, fold(&d.symbol));
        let (at_base, at_head): (HashSet<_>, HashSet<_>) = (
            was.iter().map(identity).collect(),
            now.iter().map(identity).collect(),
        );
        for declaration in was.iter().filter(|d| !at_head.contains(&identity(d))) {
            // Removed from a file that is still there, wherever it is now
            // and whatever it has become, or removed with its file.
            let reason = match change.new {
                Some(_) => Reason::Removed,
                None => Reason::WholeFile,
            };
            found.rows.push(Row::Symbol(Symbol {
                collision: self.head.declares(declaration.kind, &declaration.symbol),
                ..symbol(declaration, Removed, reason)
            }));
        }
        // Lines are mapped where there are lines on both sides to map.
        let hunks = match (&change.content, old) {
            (Content::Lines(hunks), Some(_)) => &hunks[..],
            _ => &[],
        };
        let now_alone = match new {
            Some(new) if !hunks.is_empty() => self.alone_at_head(new, now),
            _ => Vec::new(),
        };
        let mut marks = marked(now, &now_alone, hunks, Side::Head);
        // A line that is gone was a line of something at base. If that is
        // still declared, it changed: an annotation taken from above a
        // method is a change to the method, and at head only a gap above it.
        let by_base_lines = marked(&was, &was_alone, hunks, Side::Base);
        for (declaration, hunks) in was.iter().zip(by_base_lines) {
            if let Some(at) = now
                .iter()
                .position(|d| identity(d) == identity(declaration))
            {
                marks[at].extend(hunks);
                marks[at].sort();
                marks[at].dedup();
            }
        }
        for (declaration, hunks) in now.iter().zip(marks) {
            let is_new = is_base_known && !at_base.contains(&identity(declaration));
            let reason = match hunks.is_empty() {
                false => Reason::Hunk,
                true if is_new || change.content == Content::Binary => Reason::WholeFile,
                true => continue,
            };
            let change = if is_new { Added } else { Modified };
            found.rows.push(Row::Symbol(Symbol {
                hunks,
                ..symbol(declaration, change, reason)
            }));
        }
    }

    /// What the base version of a file declares, and of each declaration
    /// whether it has its first and last lines to itself. `None` for a
    /// version that should be read and cannot be: the delta has no text
    /// for it, since it is none in an encoding that is read.
    fn declared_at_base(
        &self,
        old: Version,
        found: &mut Found,
    ) -> Option<(Vec<Declaration>, Vec<bool>)> {
        let file = ScopeFile {
            path: old.path.to_string(),
            package: old.package.to_string(),
        };
        // A link or a submodule is not read, at head or here, and neither
        // is a file that the model does not read: it declares nothing.
        if !old.blob.regular || !sf_impact_extract::reads(old.path) {
            return Some((Vec::new(), Vec::new()));
        }
        let Some(text) = self.delta.text_of(old.blob) else {
            found.gaps.insert(Gap::ParseError);
            return None;
        };
        let read = sf_impact_extract::read_one(&file, text)?;
        if read.parse_error {
            found.gaps.insert(Gap::ParseError);
        }
        Some((read.declarations, read.alone))
    }

    /// For each declaration that `refs.jsonl` has for an Apex file at
    /// head: whether it has its first and last lines to itself. The lines
    /// of a declaration do not say that, so the text of the file is read. A declaration
    /// that this reading does not find where `refs.jsonl` has it is not
    /// known to, which only marks more.
    fn alone_at_head(&self, new: Version, now: &[Declaration]) -> Vec<bool> {
        if !sf_impact_extract::is_apex(new.path) {
            return Vec::new();
        }
        let file = ScopeFile {
            path: new.path.to_string(),
            package: new.package.to_string(),
        };
        let text = self.delta.text_of(new.blob);
        let read = text.and_then(|text| sf_impact_extract::read_one(&file, text));
        let place = |d: &Declaration| (d.kind, d.start, d.end, fold(&d.symbol));
        let alone: HashSet<_> = read
            .iter()
            .flat_map(|read| read.declarations.iter().zip(&read.alone))
            .filter(|(_, alone)| **alone)
            .map(|(declaration, _)| place(declaration))
            .collect();
        now.iter().map(|d| alone.contains(&place(d))).collect()
    }

    /// A file of an LWC bundle: any change to it is a change to the
    /// bundle, which is declared in one of its files and is there for as
    /// long as any of them is.
    fn bundle(
        &self,
        change: &Change,
        old: Option<Version>,
        new: Option<Version>,
        found: &mut Found,
    ) {
        // The bundle is marked whatever its files hold. What a file names
        // is read from it, and a version that did not parse says less.
        if let Some(old) = old {
            self.declared_at_base(old, found);
        }
        if new.is_some_and(|new| self.head.has_parse_error(new.path)) {
            found.gaps.insert(Gap::ParseError);
        }
        let (was_in, is_in) = (old.and_then(directory), new.and_then(directory));
        if let (Some(new), Some(bundle)) = (new, is_in) {
            // The bundle was there at base unless its directory came with
            // this file: is the new one, or is in it.
            let came = change.new_directory.as_deref().is_some_and(|came| {
                let in_it = bundle.strip_prefix(came);
                in_it.is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
            });
            let existed = was_in == Some(bundle) || !came;
            match self.head.bundle(bundle) {
                Some(declaration) => found.rows.push(Row::Symbol(Symbol {
                    hunks: match (&change.content, was_in == is_in) {
                        (Content::Lines(hunks), true) => hunks.clone(),
                        _ => Vec::new(),
                    },
                    ..part_of(declaration, new, if existed { Modified } else { Added })
                })),
                None => {
                    found.gaps.insert(Gap::NoDeclaration);
                }
            }
        }
        // The file left a bundle: by being deleted, or for another place.
        if let (Some(old), Some(bundle)) = (old, was_in.filter(|bundle| is_in != Some(*bundle))) {
            let left = match self.head.bundle(bundle) {
                Some(declaration) => part_of(declaration, old, Modified),
                None => {
                    let name = bundle.rsplit('/').next().unwrap_or(bundle);
                    Symbol {
                        symbol: name.to_string(),
                        kind: DeclType::Lwc,
                        package: old.package.to_string(),
                        file: old.path.to_string(),
                        change: Removed,
                        reason: Reason::WholeFile,
                        start: None,
                        end: None,
                        previous_file: None,
                        hunks: Vec::new(),
                        collision: self.head.declares(DeclType::Lwc, name),
                    }
                }
            };
            found.rows.push(Row::Symbol(left));
        }
    }

    /// The companion of an Apex file holds the API version and the status
    /// of what the file declares: its owner, which a change to the
    /// companion is a change to. Each version of the companion has the
    /// owner that its own name gives it. Git does not keep a companion and
    /// its Apex file together, so neither is taken to account for the other.
    fn companion(
        &self,
        change: &Change,
        old: Option<Version>,
        new: Option<Version>,
        found: &mut Found,
    ) {
        let same_owner = old.map(owner) == new.map(owner);
        // It moved with its owner, as it is.
        if same_owner && change.content == Content::Same {
            return;
        }
        if let Some(new) = new {
            let declarations = self.head.declarations(owning_file(new));
            let owners = outermost(declarations);
            if owners.is_empty() {
                found.gaps.insert(Gap::NoDeclaration);
            }
            // The owner declares something at head, so its file is there:
            // it was there at base too unless its path came since.
            let was_there = same_owner || !self.delta.came(owning_file(new));
            for at in owners {
                found.rows.push(Row::Symbol(Symbol {
                    hunks: match (&change.content, same_owner) {
                        (Content::Lines(hunks), true) => hunks.clone(),
                        _ => Vec::new(),
                    },
                    ..part_of(
                        &declarations[at],
                        new,
                        if was_there { Modified } else { Added },
                    )
                }));
            }
        }
        // The owner it had and has no longer: one that lost its companion,
        // if it is still declared, or one that is gone.
        if let Some(old) = old.filter(|_| !same_owner) {
            let still = self.head.declarations(owning_file(old));
            let mut lost: Vec<Symbol> = outermost(still)
                .into_iter()
                .map(|at| part_of(&still[at], old, Modified))
                .collect();
            if lost.is_empty() {
                let file = ScopeFile {
                    path: owning_file(old).to_string(),
                    package: old.package.to_string(),
                };
                // An owner that declares nothing at head and something at
                // base is not the same file in both: the delta has it.
                let owner = self.delta.at_base(owning_file(old));
                let text = owner.and_then(|owner| self.delta.text_of(owner));
                let was = text.and_then(|text| sf_impact_extract::read_one(&file, text));
                let was = was.map_or(Vec::new(), |read| read.declarations);
                lost = outermost(&was)
                    .into_iter()
                    .map(|at| Symbol {
                        collision: self.head.declares(was[at].kind, &was[at].symbol),
                        ..part_of(&was[at], old, Removed)
                    })
                    .collect();
            }
            if lost.is_empty() {
                found.gaps.insert(Gap::NoDeclaration);
            }
            found.rows.extend(lost.into_iter().map(Row::Symbol));
        }
    }

    /// The names in the entries that the change touches, in a file that is
    /// read entry by entry: at head from `refs.jsonl`, at base from the
    /// base version read against what head declares.
    fn touched(
        &self,
        change: &Change,
        old: Option<Version>,
        new: Option<Version>,
        found: &mut Found,
    ) {
        for (side, version) in [(Side::Base, old), (Side::Head, new)] {
            let Some(version) = version.filter(|v| sf_impact_extract::has_entries(v.path)) else {
                continue;
            };
            // Without lines for both sides, every line of a side changed.
            // A file that only moved, and is what it was, changed in none.
            let spans: Vec<Span> = match (&change.content, old, new) {
                (Content::Lines(hunks), Some(_), Some(_)) => {
                    hunks.iter().map(|hunk| Span::of(hunk, side)).collect()
                }
                (Content::Same, Some(old), Some(new)) if named(old) == named(new) => continue,
                _ => vec![Span::EVERYTHING],
            };
            let file = ScopeFile {
                path: version.path.to_string(),
                package: version.package.to_string(),
            };
            // A version without text was not read, here or for what it
            // declares, which has said so.
            let Some(text) = self.delta.text_of(version.blob) else {
                continue;
            };
            let Some(read) = sf_impact_extract::read_entries(&file, text.as_bytes().to_vec())
            else {
                continue;
            };
            // What a record is by its own name does not depend on a line:
            // any change to it is a change to a record of that type and of
            // that name. Its type is how code that queries the type is
            // reached, and its name may be what binds a class.
            let is_intrinsic = |reference: &Reference| {
                reference.kind == RefKind::CmdtType
                    || reference.source_field.as_deref() == Some("DeveloperName")
            };
            // Every other name is in an entry, and is touched if any line
            // of its entry changed.
            let entries = read.entries();
            let is_touched = |line: u32| {
                let changed = |first, last| spans.iter().any(|span| span.meets(first, last));
                let mut around = entries
                    .iter()
                    .filter(|(first, last)| (*first..=*last).contains(&line));
                around.any(|(first, last)| changed(*first, *last))
            };
            let (references, unsupported): (Vec<Reference>, Vec<u32>) = match side {
                Side::Head => (
                    self.head.references(version.path).to_vec(),
                    self.head.unsupported(version.path).to_vec(),
                ),
                Side::Base => {
                    let rows = declarative::references(
                        &read,
                        &self.head.catalog,
                        version.package,
                        version.path,
                    );
                    let mut read = (Vec::new(), Vec::new());
                    for row in rows {
                        match row {
                            rows::Row::Reference(reference) => read.0.push(reference),
                            rows::Row::UnsupportedElement(element) => read.1.push(element.line),
                            _ => {}
                        }
                    }
                    read
                }
            };
            if unsupported.into_iter().any(is_touched) {
                found.gaps.insert(Gap::UnsupportedElement);
            }
            let touched = |r: &Reference| is_intrinsic(r) || is_touched(r.line);
            for reference in references.into_iter().filter(touched) {
                found.rows.push(Row::Touched(Touched {
                    referrer: reference.referrer.unwrap_or_default(),
                    side,
                    kind: reference.kind,
                    referenced: reference.referenced,
                    referenced_type: reference.referenced_type,
                    resolution: reference.resolution.unwrap_or(Resolution::Unresolved),
                    candidates: reference.candidates,
                    package: reference.package,
                    file: reference.file,
                    line: reference.line,
                }));
            }
        }
    }
}

/// A row for a declaration, on the evidence of the file that declares it.
fn symbol(declaration: &Declaration, change: changed::Change, reason: Reason) -> Symbol {
    Symbol {
        symbol: declaration.symbol.clone(),
        kind: declaration.kind,
        package: declaration.package.clone(),
        file: declaration.file.clone(),
        change,
        reason,
        start: Some(declaration.start),
        end: Some(declaration.end),
        previous_file: None,
        hunks: Vec::new(),
        collision: declaration.collision,
    }
}

/// A row for a declaration, on the evidence of a changed file that is a
/// part of what is declared: a file of a bundle, the companion of a class.
/// It has the lines of the declaration only if it is the file they are in.
fn part_of(declaration: &Declaration, part: Version, change: changed::Change) -> Symbol {
    let holds_the_lines = declaration.file == part.path;
    Symbol {
        package: part.package.to_string(),
        file: part.path.to_string(),
        start: holds_the_lines.then_some(declaration.start),
        end: holds_the_lines.then_some(declaration.end),
        ..symbol(declaration, change, Reason::WholeFile)
    }
}

/// How a file in scope is mapped, going by its path.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A file of an LWC bundle: it marks the bundle.
    Bundle,
    /// The `-meta.xml` beside an Apex file: it marks what that declares.
    Companion,
    /// A component of a type that is not modelled.
    Unmodelled,
    /// Any other file: it marks what is declared in it, if anything is.
    Declares,
}

impl Kind {
    fn of(path: &str) -> Kind {
        if sf_impact_extract::bundle(path).is_some() {
            Kind::Bundle
        } else if companion_of(path).is_some() {
            Kind::Companion
        } else if sf_impact_extract::unmodelled(path).is_some() {
            Kind::Unmodelled
        } else {
            Kind::Declares
        }
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// What a file is called in its package: what its component is named by.
fn named<'a>(version: Version<'a>) -> (&'a str, &'a str) {
    (version.package, file_name(version.path))
}

/// The Apex file that a version of a companion is beside, by its name.
fn owning_file<'a>(companion: Version<'a>) -> &'a str {
    companion_of(companion.path).unwrap_or_default()
}

/// What a version of a companion is the companion of: the type or trigger
/// that an Apex file of that name declares, in that package.
fn owner<'a>(companion: Version<'a>) -> (&'a str, String) {
    (companion.package, fold(file_name(owning_file(companion))))
}

/// The directory of the LWC bundle that a file is in.
fn directory(version: Version<'_>) -> Option<&str> {
    sf_impact_extract::bundle(version.path).map(|(directory, _)| directory)
}

/// The Apex file that `path` is the companion of: `A.cls-meta.xml`.
fn companion_of(path: &str) -> Option<&str> {
    let apex = path.strip_suffix("-meta.xml")?;
    sf_impact_extract::is_apex(apex).then_some(apex)
}

/// Lines `first..=last` of one version of a file. Where a hunk leaves a
/// version no lines, the span is the gap between two lines: `first` is the
/// line after it and `last` the line before it, so that it meets whatever
/// holds both and no line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    first: u32,
    last: u32,
}

impl Span {
    const EVERYTHING: Span = Span {
        first: 1,
        last: u32::MAX,
    };

    fn new(start: u32, lines: u32) -> Span {
        match lines {
            0 => Span {
                first: start + 1,
                last: start,
            },
            _ => Span {
                first: start,
                last: start + lines - 1,
            },
        }
    }

    /// The lines of a hunk in one version of the file.
    fn of(hunk: &Hunk, side: Side) -> Span {
        match side {
            Side::Base => Span::new(hunk.base_start, hunk.base_lines),
            Side::Head => Span::new(hunk.head_start, hunk.head_lines),
        }
    }

    fn meets(self, first: u32, last: u32) -> bool {
        self.first <= last && first <= self.last
    }
}

/// For each declaration of one version of a file, the hunks that mark it.
/// A line of that version that a hunk changes, adds or takes away marks the
/// innermost declaration around it: a method, or the class where it is in
/// no member. At head, a hunk that only takes lines away marks the
/// innermost declaration around the gap it leaves. `alone` says of each
/// declaration whether it has its first and last lines to itself.
fn marked(
    declarations: &[Declaration],
    alone: &[bool],
    hunks: &[Hunk],
    side: Side,
) -> Vec<Vec<Hunk>> {
    let mut marked = vec![Vec::new(); declarations.len()];
    for hunk in hunks {
        let span = Span::of(hunk, side);
        let places: Vec<Span> = match (span.first <= span.last, side) {
            (true, _) => (span.first..=span.last)
                .map(|line| Span::new(line, 1))
                .collect(),
            (false, Side::Head) => vec![span],
            (false, Side::Base) => Vec::new(),
        };
        for place in places {
            for at in innermost(declarations, alone, place) {
                if marked[at].last() != Some(hunk) {
                    marked[at].push(*hunk);
                }
            }
        }
    }
    marked
}

/// The declarations of a file that a line or a gap marks: the innermost
/// around it, and more than one where they have the same lines.
///
/// A line that a declaration starts or ends on can hold more than the
/// declaration: another member, the rest of a class header, the end of an
/// initializer, none of which has lines of its own to be found by. A line
/// cannot say which of them changed. So such a line marks every
/// declaration around it, the type they are in too, unless it is proven to
/// be a line of one declaration alone: only declarations on the very same
/// lines start or end on it, and each has its first and last lines to
/// itself. A declaration that is not known to, does not.
///
/// A place outside every declaration belongs to the file, and so to what
/// the file declares at the top: a comment above a class is a change to
/// the class.
fn innermost(declarations: &[Declaration], alone: &[bool], place: Span) -> Vec<usize> {
    let size = |d: &Declaration| d.end - d.start;
    let all = || declarations.iter().enumerate();
    let around: Vec<usize> = all()
        .filter(|(_, d)| place.meets(d.start, d.end))
        .map(|(at, _)| at)
        .collect();
    let Some(smallest) = around.iter().map(|at| size(&declarations[*at])).min() else {
        return outermost(declarations);
    };
    let lines = |at: &usize| (declarations[*at].start, declarations[*at].end);
    let is_line = place.first == place.last;
    let mut on_edge = around.iter().filter(|at| {
        let (start, end) = lines(at);
        is_line && [start, end].contains(&place.first)
    });
    let is_of_one_alone = match on_edge.next() {
        Some(first) => {
            let stands_alone = |at: &usize| alone.get(*at) == Some(&true);
            stands_alone(first) && on_edge.all(|at| stands_alone(at) && lines(at) == lines(first))
        }
        None => true,
    };
    if !is_of_one_alone {
        return around;
    }
    let innermost = around.into_iter();
    innermost
        .filter(|at| size(&declarations[*at]) == smallest)
        .collect()
}

/// The declarations of a file that are inside no other.
fn outermost(declarations: &[Declaration]) -> Vec<usize> {
    let size = |d: &Declaration| d.end - d.start;
    let is_inside = |d: &Declaration| {
        let holds = |other: &Declaration| other.start <= d.start && d.end <= other.end;
        declarations
            .iter()
            .any(|other| holds(other) && size(other) > size(d))
    };
    let all = declarations.iter().enumerate();
    all.filter(|(_, d)| !is_inside(d))
        .map(|(at, _)| at)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declaration(symbol: &str, start: u32, end: u32) -> Declaration {
        Declaration {
            symbol: symbol.into(),
            name: symbol.into(),
            kind: DeclType::Method,
            package: String::new(),
            file: String::new(),
            start,
            end,
            is_test: false,
            annotations: Vec::new(),
            collision: false,
            scope: None,
        }
    }

    /// A class from line 1 to 20 with two methods and an inner class.
    fn class() -> Vec<Declaration> {
        vec![
            declaration("A", 1, 20),
            declaration("A.m()", 3, 5),
            declaration("A.n()", 7, 9),
            declaration("A.Inner", 11, 19),
            declaration("A.Inner.o()", 13, 15),
        ]
    }

    fn hunk(base_start: u32, base_lines: u32, head_start: u32, head_lines: u32) -> Hunk {
        Hunk {
            base_start,
            base_lines,
            head_start,
            head_lines,
        }
    }

    /// The symbols that a hunk marks, where every declaration has its
    /// first and last lines to itself, as code is usually written.
    fn marks(declarations: &[Declaration], hunk: Hunk) -> Vec<&str> {
        marks_where(declarations, &vec![true; declarations.len()], hunk)
    }

    /// The symbols that a hunk marks, where `alone` says which declarations
    /// have their first and last lines to themselves.
    fn marks_where<'a>(
        declarations: &'a [Declaration],
        alone: &[bool],
        hunk: Hunk,
    ) -> Vec<&'a str> {
        let marked = marked(declarations, alone, &[hunk], Side::Head);
        let all = declarations.iter().zip(marked);
        let marked = all.filter(|(_, hunks)| !hunks.is_empty());
        marked.map(|(d, _)| d.symbol.as_str()).collect()
    }

    #[test]
    fn a_line_a_member_starts_or_ends_on_marks_what_is_around_it_unless_it_is_the_members_alone() {
        // `small` has the rest of the class header in front of it, or the
        // end of an initializer: something of the class is on its line.
        let class = [
            declaration("A", 1, 5),
            declaration("A.small()", 2, 2),
            declaration("A.second()", 3, 3),
        ];
        let small_shares_its_line = [true, false, true];

        assert_eq!(
            marks_where(&class, &small_shares_its_line, hunk(2, 1, 2, 1)),
            ["A", "A.small()"]
        );
        assert_eq!(
            marks_where(&class, &small_shares_its_line, hunk(3, 1, 3, 1)),
            ["A.second()"]
        );

        // A member of several lines: its first and its last are in doubt,
        // the lines between them are not.
        let long = [declaration("A", 1, 9), declaration("A.m()", 2, 6)];
        let shares = [true, false];
        assert_eq!(
            marks_where(&long, &shares, hunk(2, 1, 2, 1)),
            ["A", "A.m()"]
        );
        assert_eq!(marks_where(&long, &shares, hunk(4, 1, 4, 1)), ["A.m()"]);
        assert_eq!(
            marks_where(&long, &shares, hunk(6, 1, 6, 1)),
            ["A", "A.m()"]
        );
    }

    #[test]
    fn a_member_that_is_not_known_to_have_its_line_to_itself_does_not() {
        let class = [declaration("A", 1, 5), declaration("A.m()", 2, 4)];

        assert_eq!(marks_where(&class, &[], hunk(2, 1, 2, 1)), ["A", "A.m()"]);
        assert_eq!(marks_where(&class, &[], hunk(3, 1, 3, 1)), ["A.m()"]);
    }

    #[test]
    fn a_changed_line_marks_the_innermost_declaration_around_it() {
        let class = class();

        assert_eq!(marks(&class, hunk(4, 1, 4, 1)), ["A.m()"]);
        assert_eq!(marks(&class, hunk(6, 1, 6, 1)), ["A"]);
        assert_eq!(marks(&class, hunk(12, 1, 12, 1)), ["A.Inner"]);
        assert_eq!(marks(&class, hunk(14, 1, 14, 1)), ["A.Inner.o()"]);
    }

    #[test]
    fn a_hunk_over_several_declarations_marks_each_by_its_own_lines() {
        let class = class();

        // The end of one method, the line between, the start of the next.
        assert_eq!(marks(&class, hunk(5, 3, 5, 3)), ["A", "A.m()", "A.n()"]);
        // Two methods whole, and nothing of the class between them: a
        // class is not marked by lines that are all in its members.
        let adjacent = [
            declaration("A", 1, 9),
            declaration("A.m()", 2, 4),
            declaration("A.n()", 5, 7),
        ];
        assert_eq!(marks(&adjacent, hunk(2, 6, 2, 6)), ["A.m()", "A.n()"]);
    }

    #[test]
    fn lines_taken_away_mark_what_holds_the_gap_they_leave() {
        let class = class();

        // Out of the body of a method.
        assert_eq!(marks(&class, hunk(4, 2, 3, 0)), ["A.m()"]);
        // Between two methods: a member that is gone.
        assert_eq!(marks(&class, hunk(6, 4, 5, 0)), ["A"]);
        assert_eq!(marks(&class, hunk(10, 4, 9, 0)), ["A"]);
        // Just inside the inner class, before its first member.
        assert_eq!(marks(&class, hunk(12, 1, 11, 0)), ["A.Inner"]);
    }

    #[test]
    fn a_line_outside_every_declaration_marks_what_the_file_declares_at_the_top() {
        let below_a_header = [declaration("A", 3, 9), declaration("A.m()", 4, 6)];
        let labels = [
            declaration("Label.One", 3, 5),
            declaration("Label.Two", 6, 8),
        ];

        assert_eq!(marks(&below_a_header, hunk(1, 1, 1, 1)), ["A"]);
        assert_eq!(marks(&below_a_header, hunk(1, 1, 0, 0)), ["A"]);
        assert_eq!(marks(&labels, hunk(2, 1, 2, 1)), ["Label.One", "Label.Two"]);
    }

    #[test]
    fn a_line_that_declarations_share_without_being_on_the_same_lines_marks_all_around_it() {
        // `first` is all on line 2, where `second` starts.
        let meeting = [
            declaration("A", 1, 5),
            declaration("A.first()", 2, 2),
            declaration("A.second()", 2, 4),
        ];
        // `small` is all on the line of the class header.
        let header = [declaration("A", 1, 5), declaration("A.small()", 1, 1)];
        // The inner class ends on the line its last method ends on.
        let closing = [
            declaration("A", 1, 9),
            declaration("A.Inner", 3, 8),
            declaration("A.Inner.m()", 5, 8),
        ];

        assert_eq!(
            marks(&meeting, hunk(2, 1, 2, 1)),
            ["A", "A.first()", "A.second()"]
        );
        assert_eq!(marks(&header, hunk(1, 1, 1, 1)), ["A", "A.small()"]);
        assert_eq!(
            marks(&closing, hunk(8, 1, 8, 1)),
            ["A", "A.Inner", "A.Inner.m()"]
        );
        // A line that is only inside them is as clear as ever.
        assert_eq!(marks(&meeting, hunk(3, 1, 3, 1)), ["A.second()"]);
        assert_eq!(marks(&closing, hunk(6, 1, 6, 1)), ["A.Inner.m()"]);
        assert_eq!(marks(&closing, hunk(4, 1, 4, 1)), ["A.Inner"]);
        // And so is a gap: lines taken from above the shared line were
        // inside whatever holds the lines on both sides of them.
        assert_eq!(marks(&closing, hunk(8, 2, 7, 0)), ["A.Inner.m()"]);
    }

    #[test]
    fn declarations_on_the_same_lines_are_marked_together() {
        let fields = [
            declaration("A", 1, 5),
            declaration("A.n", 2, 2),
            declaration("A.m", 2, 2),
        ];

        assert_eq!(marks(&fields, hunk(2, 1, 2, 1)), ["A.n", "A.m"]);
    }

    #[test]
    fn a_line_that_is_gone_marks_what_it_was_a_line_of_at_base() {
        let class = class();
        let marks = |hunk: Hunk| -> Vec<&str> {
            let marked = marked(&class, &[true; 5], &[hunk], Side::Base);
            let all = class.iter().zip(marked);
            let marked = all.filter(|(_, hunks)| !hunks.is_empty());
            marked.map(|(d, _)| d.symbol.as_str()).collect()
        };

        // The first line of a method: an annotation, say.
        assert_eq!(marks(hunk(3, 1, 2, 0)), ["A.m()"]);
        assert_eq!(marks(hunk(6, 1, 5, 0)), ["A"]);
        // Lines that are only added were lines of nothing at base.
        assert_eq!(marks(hunk(4, 0, 5, 2)), [] as [&str; 0]);
    }

    #[test]
    fn a_file_that_declares_nothing_has_nothing_to_mark() {
        assert_eq!(marks(&[], hunk(1, 1, 1, 1)), [] as [&str; 0]);
    }

    #[test]
    fn a_span_without_lines_meets_what_holds_the_lines_on_both_sides_of_it() {
        let gap = Span::new(4, 0);

        assert!(gap.meets(3, 6));
        assert!(gap.meets(4, 5));
        // It ends at line 4, or starts at line 5: the gap is outside it.
        assert!(!gap.meets(1, 4));
        assert!(!gap.meets(5, 9));
        assert!(!gap.meets(4, 4));
    }
}
