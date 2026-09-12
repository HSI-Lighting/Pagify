//! What a document carries that is not on its pages.
//!
//! # Why this is its own tool
//!
//! Redaction and Lock answer "take these words off the page". Neither answers
//! "what else is in this file?", and the difference has embarrassed a great
//! many people: a document saved incrementally keeps **everything it used to
//! say** in front of the new version, so a passage removed on Tuesday is still
//! there on Friday for anyone who opens the file in a text editor.
//!
//! Measured on a real 149-page catalogue:
//!
//! ```text
//! saved revisions in the file : 2
//! document information        : CreationDate, Creator, ModDate, Producer, Trapped
//! XMP metadata stream         : yes
//! objects                     : 4888 total, 8 unreachable
//! ```
//!
//! None of that is visible on a single page, and all of it travels with the
//! file.
//!
//! # It reports before it removes
//!
//! [`survey`] says what is there; [`strip`] takes it out. Two calls rather than
//! one, because a person deciding whether to sanitise a document needs to know
//! what sanitising would cost — a form's field values and an author's name are
//! both "hidden data", and only one of them is usually unwanted.
//!
//! # What it will never remove
//!
//! **The lock's own attachment.** A locked document keeps the sealed original
//! inside itself; that is hidden data by any definition and removing it would
//! destroy the only copy of what somebody was promised they could get back.
//! It is named explicitly and always kept.

use std::collections::BTreeSet;

use crate::error::Result;

use super::object::Dict;
use super::{write_object, File, Object};

/// The lock's attachment, which is hidden data that must survive sanitising.
const KEEP: &str = "pagify-lock.json";

/// What a file carries beyond its pages.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hidden {
    /// How many times the file has been saved into. Anything above one means
    /// earlier versions of every page are still in it.
    pub revisions: usize,
    /// The `/Info` entries — author, producer, the times it was written.
    pub information: Vec<String>,
    /// An XMP packet, which usually repeats `/Info` and often says more.
    pub xmp: bool,
    /// Files carried inside this one, not counting the lock's own.
    pub embedded_files: usize,
    /// Script that runs when the document is opened or acted on.
    pub javascript: bool,
    /// Objects nothing reaches from the catalogue: content taken off the pages
    /// but never taken out of the file.
    pub unreachable: usize,
    /// Whether a lock's sealed copy is present, which is kept whatever else
    /// goes.
    pub keeps_lock: bool,
}

impl Hidden {
    /// Whether there is anything here worth removing.
    pub fn is_empty(&self) -> bool {
        self.revisions <= 1
            && self.information.is_empty()
            && !self.xmp
            && self.embedded_files == 0
            && !self.javascript
            && self.unreachable == 0
    }

    /// What was found, in words.
    pub fn describe(&self) -> String {
        let mut found: Vec<String> = Vec::new();
        if self.revisions > 1 {
            found.push(format!(
                "{} earlier version{} of the whole document",
                self.revisions - 1,
                if self.revisions == 2 { "" } else { "s" }
            ));
        }
        if !self.information.is_empty() {
            found.push(format!("document information ({})", self.information.join(", ")));
        }
        if self.xmp {
            found.push("an XMP metadata packet".into());
        }
        if self.embedded_files > 0 {
            found.push(format!("{} embedded file(s)", self.embedded_files));
        }
        if self.javascript {
            found.push("JavaScript".into());
        }
        if self.unreachable > 0 {
            found.push(format!("{} unreachable object(s)", self.unreachable));
        }
        if found.is_empty() {
            return "nothing hidden beyond the pages themselves".into();
        }
        found.join("; ")
    }
}

impl Hidden {
    /// What was here and is not in `remaining`: the part a clean removed.
    pub fn without(&self, remaining: &Hidden) -> Hidden {
        Hidden {
            revisions: (self.revisions + 1).saturating_sub(remaining.revisions).max(1),
            information: self
                .information
                .iter()
                .filter(|k| !remaining.information.contains(k))
                .cloned()
                .collect(),
            xmp: self.xmp && !remaining.xmp,
            embedded_files: self.embedded_files.saturating_sub(remaining.embedded_files),
            javascript: self.javascript && !remaining.javascript,
            unreachable: self.unreachable.saturating_sub(remaining.unreachable),
            keeps_lock: self.keeps_lock,
        }
    }
}

/// What a clean did: what the file carried, and what it still carries.
///
/// **Reported from a second survey of the cleaned bytes, not from the first
/// one.** The message used to print what the survey *found* as what the clean
/// *removed* — and for attachments and JavaScript, which the clean did not
/// touch, that was a claim with nothing behind it. Found by audit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sanitised {
    /// What the file carried before.
    pub before: Hidden,
    /// What a survey of the cleaned bytes still finds.
    pub after: Hidden,
}

impl Sanitised {
    /// What actually came out.
    pub fn removed(&self) -> Hidden {
        self.before.without(&self.after)
    }

    /// Whether everything that was found came out.
    pub fn is_clean(&self) -> bool {
        self.after.is_empty()
    }

    /// In words: what came out, and — first, because it matters more — what
    /// did not.
    pub fn describe(&self) -> String {
        let removed = self.removed();
        match (removed.is_empty(), self.after.is_empty()) {
            (true, true) => "there was nothing hidden to remove".into(),
            (false, true) => format!("removed: {}", removed.describe()),
            (true, false) => format!("NOTHING REMOVED — still there: {}", self.after.describe()),
            (false, false) => format!(
                "STILL THERE: {}. Removed: {}",
                self.after.describe(),
                removed.describe()
            ),
        }
    }
}

/// Read what a file carries beyond its pages. Changes nothing.
pub fn survey(file: &File<'_>, bytes: &[u8]) -> Result<Hidden> {
    let mut found = Hidden {
        // Each save writes one `startxref`, so counting them counts revisions.
        revisions: bytes.windows(9).filter(|w| *w == b"startxref").count().max(1),
        ..Hidden::default()
    };

    if let Some(info) = file.trailer().get(b"Info").and_then(|i| file.resolve(i).ok()) {
        if let Some(dict) = info.as_dict() {
            found.information = dict
                .0
                .iter()
                .map(|(key, _)| String::from_utf8_lossy(key).into_owned())
                .collect();
        }
    }

    let root = file
        .trailer()
        .get(b"Root")
        .and_then(|r| file.resolve(r).ok())
        .and_then(|r| r.as_dict().cloned());
    if let Some(root) = &root {
        found.xmp = root.get(b"Metadata").is_some();

        let names = root
            .get(b"Names")
            .and_then(|n| file.resolve(n).ok())
            .and_then(|n| n.as_dict().cloned());
        if let Some(names) = &names {
            found.javascript = names.get(b"JavaScript").is_some();
            let (all, lock) = count_attachments(file, names);
            found.embedded_files = all.saturating_sub(usize::from(lock));
            found.keeps_lock = lock;
        }
    }

    let reachable = reachable_from_root(file);
    found.unreachable = file.numbers().filter(|n| !reachable.contains(n)).count();

    // **Script is wherever an action can run it**, not only in the name tree:
    // on opening, on a page being shown, on a link being followed, on a form
    // field changing. Every reachable object is searched for an action that
    // runs script, so what this reports is what `strip` takes out — the two
    // are the same rule, and a survey that looked in fewer places than the
    // clean would report a file as clean that was not.
    found.javascript = found.javascript
        || reachable
            .iter()
            .filter_map(|n| file.object(*n).ok())
            .any(|object| holds_script(file, &object, 0));
    Ok(found)
}

/// Whether an action runs script: `/S /JavaScript`, or a `/JS` entry, which
/// only a JavaScript action carries.
fn runs_script(file: &File<'_>, action: &Object) -> bool {
    let Ok(action) = file.resolve(action) else { return false };
    let Some(dict) = action.as_dict() else { return false };
    dict.get(b"S").and_then(Object::as_name) == Some(&b"JavaScript"[..]) || dict.get(b"JS").is_some()
}

/// Whether anything in an object — at any depth, references not followed —
/// is an action that runs script.
fn holds_script(file: &File<'_>, object: &Object, depth: usize) -> bool {
    if depth > 32 {
        return false;
    }
    match object {
        Object::Dict(dict) | Object::Stream(dict, _) => {
            runs_script(file, object)
                || dict.0.iter().any(|(_, value)| holds_script(file, value, depth + 1))
        }
        Object::Array(items) => items.iter().any(|item| holds_script(file, item, depth + 1)),
        _ => false,
    }
}

/// The keys an action hangs from. `/AA` is a dictionary of them, one per
/// trigger; the others hold one action, or an array of them.
const ACTION_KEYS: [&[u8]; 3] = [b"A", b"OpenAction", b"Next"];

/// The object with every action that runs script taken out of it, and
/// whether anything was.
///
/// References are not followed: an object reached only through a removed
/// action becomes unreachable, and the clean leaves it out for that reason.
fn scrubbed(file: &File<'_>, object: &Object, depth: usize) -> (Object, bool) {
    if depth > 32 {
        return (object.clone(), false);
    }
    match object {
        Object::Dict(dict) => {
            let (dict, changed) = scrubbed_dict(file, dict, depth);
            (Object::Dict(dict), changed)
        }
        // A stream's bytes are never touched; its dictionary is a dictionary.
        Object::Stream(dict, data) => {
            let (dict, changed) = scrubbed_dict(file, dict, depth);
            (Object::Stream(dict, data.clone()), changed)
        }
        Object::Array(items) => {
            let mut changed = false;
            let out = items
                .iter()
                .map(|item| {
                    let (item, c) = scrubbed(file, item, depth + 1);
                    changed |= c;
                    item
                })
                .collect();
            (Object::Array(out), changed)
        }
        other => (other.clone(), false),
    }
}

fn scrubbed_dict(file: &File<'_>, dict: &Dict, depth: usize) -> (Dict, bool) {
    let mut out = Vec::with_capacity(dict.0.len());
    let mut changed = false;
    for (key, value) in &dict.0 {
        if key == b"AA" {
            // One action per trigger: keep the triggers whose action is not
            // script, drop the entry when none are.
            let Ok(Object::Dict(triggers)) = file.resolve(value) else {
                out.push((key.clone(), value.clone()));
                continue;
            };
            let kept: Vec<(Vec<u8>, Object)> = triggers
                .0
                .iter()
                .filter(|(_, action)| !runs_script(file, action))
                .cloned()
                .collect();
            if kept.len() != triggers.0.len() {
                changed = true;
                if !kept.is_empty() {
                    out.push((key.clone(), Object::Dict(Dict(kept))));
                }
                continue;
            }
        } else if ACTION_KEYS.contains(&key.as_slice()) {
            match file.resolve(value) {
                Ok(Object::Array(actions)) => {
                    let kept: Vec<Object> =
                        actions.iter().filter(|a| !runs_script(file, a)).cloned().collect();
                    if kept.len() != actions.len() {
                        changed = true;
                        if !kept.is_empty() {
                            out.push((key.clone(), Object::Array(kept)));
                        }
                        continue;
                    }
                }
                _ if runs_script(file, value) => {
                    changed = true;
                    continue;
                }
                _ => {}
            }
        }
        let (value, c) = scrubbed(file, value, depth + 1);
        changed |= c;
        out.push((key.clone(), value));
    }
    (Dict(out), changed)
}

/// The `/Names` dictionary without its script and without any attachment but
/// the lock's; `None` when nothing in it is left.
fn names_kept(file: &File<'_>, names: &Dict) -> Option<Dict> {
    let mut out = Vec::new();
    for (key, value) in &names.0 {
        match key.as_slice() {
            b"JavaScript" => continue,
            b"EmbeddedFiles" => {
                let Ok(tree) = file.resolve(value) else { continue };
                let mut lock = Vec::new();
                lock_entries(file, &tree, &mut lock, 0);
                if lock.is_empty() {
                    continue;
                }
                // A flat tree holding only the lock's entry — the shape every
                // reader accepts, and the whole of what is kept.
                let mut leaf = Dict(Vec::new());
                leaf.set(b"Names", Object::Array(lock));
                out.push((key.clone(), Object::Dict(leaf)));
            }
            _ => out.push((key.clone(), value.clone())),
        }
    }
    (!out.is_empty()).then_some(Dict(out))
}

/// The `[name value]` pairs in an attachment tree that are the lock's own.
fn lock_entries(file: &File<'_>, node: &Object, out: &mut Vec<Object>, depth: usize) {
    if depth > 32 {
        return;
    }
    let Some(dict) = node.as_dict() else { return };
    if let Some(Object::Array(items)) = dict.get(b"Names").and_then(|n| file.resolve(n).ok()) {
        for pair in items.chunks(2) {
            let (Some(name), Some(value)) = (pair.first(), pair.get(1)) else { continue };
            let spelled = match name {
                Object::LiteralString(raw) | Object::HexString(raw) => {
                    String::from_utf8_lossy(raw).into_owned()
                }
                _ => String::new(),
            };
            if spelled.contains(KEEP) {
                out.push(name.clone());
                out.push(value.clone());
            }
        }
    }
    if let Some(Object::Array(kids)) = dict.get(b"Kids").and_then(|k| file.resolve(k).ok()) {
        for kid in kids {
            if let Ok(kid) = file.resolve(&kid) {
                lock_entries(file, &kid, out, depth + 1);
            }
        }
    }
}

/// A copy with the hidden data taken out.
///
/// Everything on the pages is untouched — this rewrites the file's *structure*,
/// never a content stream. Objects nothing reaches are left out; `/Info`, the
/// XMP packet, the attachments (but the lock's), the script name tree and
/// every action that runs script go; and the result is one revision rather
/// than several, which is what removes the earlier versions.
///
/// **What it says it removed is measured, not assumed**: the cleaned bytes are
/// surveyed again, and the second survey is what the caller shows.
pub fn strip(file: &File<'_>, bytes: &[u8]) -> Result<(Vec<u8>, Sanitised)> {
    let before = survey(file, bytes)?;

    let mut replacements: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut rewritten: std::collections::BTreeMap<u32, Object> = std::collections::BTreeMap::new();

    // The catalogue without its metadata pointer, its script, its attachments
    // (but the lock's) and its opening action if that runs script.
    let root_number = match file.trailer().get(b"Root") {
        Some(Object::Reference(number, _)) => Some(*number),
        _ => None,
    };
    if let Some(number) = root_number {
        if let Ok(Object::Dict(mut root)) = file.object(number) {
            root.remove(b"Metadata");
            if let Some(names) = root
                .get(b"Names")
                .and_then(|n| file.resolve(n).ok())
                .and_then(|n| n.as_dict().cloned())
            {
                match names_kept(file, &names) {
                    Some(kept) => root.set(b"Names", Object::Dict(kept)),
                    None => root.remove(b"Names"),
                }
            }
            let (root, _) = scrubbed(file, &Object::Dict(root), 0);
            rewritten.insert(number, root);
        }
    }

    // Every other object with its script actions taken out. Only objects
    // that change are rewritten; the rest are copied through byte for byte.
    for number in file.numbers() {
        if Some(number) == root_number {
            continue;
        }
        let Ok(object) = file.object(number) else { continue };
        let (object, changed) = scrubbed(file, &object, 0);
        if changed {
            rewritten.insert(number, object);
        }
    }
    for (number, object) in &rewritten {
        let mut body = Vec::new();
        write_object(&mut body, object);
        replacements.push((*number, body));
    }

    // **Reachability is computed against the file as it will be, not as it
    // is.** Taking `/Info` off the trailer and `/Metadata` off the catalogue
    // orphans those objects; walking the original would still count them as
    // reached and copy them through, leaving the metadata in the file with
    // nothing pointing at it. Measured on a catalogue: two objects survived a
    // clean that way. The same goes for the attachments and the script: the
    // walk reads the rewritten objects, so what they no longer point at is
    // left out.
    let mut reachable = BTreeSet::new();
    if let Some(number) = root_number {
        walk_through(file, &rewritten, number, &mut reachable, 0);
    }
    let drop: Vec<u32> = file.numbers().filter(|n| !reachable.contains(n)).collect();

    // The trailer loses `/Info` entirely. An empty `/Info` dictionary would
    // still say a Pagify-shaped tool had been over the file.
    let mut trailer = Dict(Vec::new());
    trailer.set(b"Info", Object::Null);

    let cleaned = file.rewrite_dropping(&replacements, &[], &trailer, &drop)?;

    // Measured on the result, not assumed from the intent.
    let after = File::parse(&cleaned).and_then(|f| survey(&f, &cleaned))?;
    Ok((cleaned, Sanitised { before, after }))
}

/// [`walk`], reading rewritten objects where there are any.
fn walk_through(
    file: &File<'_>,
    rewritten: &std::collections::BTreeMap<u32, Object>,
    number: u32,
    seen: &mut BTreeSet<u32>,
    depth: usize,
) {
    if depth > 96 || !seen.insert(number) {
        return;
    }
    let object = match rewritten.get(&number) {
        Some(object) => object.clone(),
        None => match file.object(number) {
            Ok(object) => object,
            Err(_) => return,
        },
    };
    references(&object, &mut |n| walk_through(file, rewritten, n, seen, depth + 1));
}

/// Everything reachable from the catalogue as the file stands, following
/// references.
///
/// `/Info` counts as reached here because it still is: this is what the survey
/// reports against, and calling a referenced object "unreachable" would be a
/// lie. `strip` computes its own set against the file it is about to write.
fn reachable_from_root(file: &File<'_>) -> BTreeSet<u32> {
    let mut seen = BTreeSet::new();
    if let Some(Object::Reference(number, _)) = file.trailer().get(b"Root") {
        walk(file, *number, &mut seen, 0);
    }
    if let Some(Object::Reference(number, _)) = file.trailer().get(b"Info") {
        walk(file, *number, &mut seen, 0);
    }
    seen
}

fn walk(file: &File<'_>, number: u32, seen: &mut BTreeSet<u32>, depth: usize) {
    // A depth cap rather than trust: a file can point an object at itself, and
    // a sanitiser that hangs is a sanitiser nobody runs.
    if depth > 96 || !seen.insert(number) {
        return;
    }
    let Ok(object) = file.object(number) else { return };
    references(&object, &mut |n| walk(file, n, seen, depth + 1));
}

fn references(object: &Object, found: &mut impl FnMut(u32)) {
    match object {
        Object::Reference(n, _) => found(*n),
        Object::Array(items) => items.iter().for_each(|item| references(item, found)),
        Object::Dict(dict) | Object::Stream(dict, _) => {
            dict.0.iter().for_each(|(_, value)| references(value, found))
        }
        _ => {}
    }
}

/// How many files are carried inside, and whether one of them is the lock's.
fn count_attachments(file: &File<'_>, names: &Dict) -> (usize, bool) {
    let Some(tree) = names.get(b"EmbeddedFiles").and_then(|e| file.resolve(e).ok()) else {
        return (0, false);
    };
    let mut count = 0usize;
    let mut lock = false;
    collect_names(file, &tree, &mut count, &mut lock, 0);
    (count, lock)
}

/// A name tree is `/Names [key value key value …]` at the leaves and `/Kids`
/// above; both shapes have to be walked to count what is in it.
fn collect_names(file: &File<'_>, node: &Object, count: &mut usize, lock: &mut bool, depth: usize) {
    if depth > 32 {
        return;
    }
    let Some(dict) = node.as_dict() else { return };

    if let Some(Object::Array(items)) = dict.get(b"Names").and_then(|n| file.resolve(n).ok()) {
        for pair in items.chunks(2) {
            let Some(name) = pair.first() else { continue };
            *count += 1;
            let spelled = match name {
                Object::LiteralString(raw) | Object::HexString(raw) => {
                    String::from_utf8_lossy(raw).into_owned()
                }
                _ => String::new(),
            };
            if spelled.contains(KEEP) {
                *lock = true;
            }
        }
    }
    if let Some(Object::Array(kids)) = dict.get(b"Kids").and_then(|k| file.resolve(k).ok()) {
        for kid in kids {
            if let Ok(kid) = file.resolve(&kid) {
                collect_names(file, &kid, count, lock, depth + 1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_with_nothing_hidden_says_so() {
        let found = Hidden { revisions: 1, ..Hidden::default() };
        assert!(found.is_empty());
        assert_eq!(found.describe(), "nothing hidden beyond the pages themselves");
    }

    /// The count is of revisions; what it *means* is earlier versions, and the
    /// wording has to be the second thing or nobody acts on it.
    #[test]
    fn earlier_revisions_are_described_as_what_they_are() {
        let two = Hidden { revisions: 2, ..Hidden::default() };
        assert_eq!(two.describe(), "1 earlier version of the whole document");
        let four = Hidden { revisions: 4, ..Hidden::default() };
        assert!(four.describe().starts_with("3 earlier versions"));
    }

    #[test]
    fn everything_found_is_listed() {
        let found = Hidden {
            revisions: 2,
            information: vec!["Author".into(), "Producer".into()],
            xmp: true,
            embedded_files: 1,
            javascript: true,
            unreachable: 8,
            keeps_lock: false,
        };
        let said = found.describe();
        for expected in ["earlier version", "Author", "XMP", "embedded", "JavaScript", "8 unreachable"] {
            assert!(said.contains(expected), "{expected:?} was not reported: {said}");
        }
        assert!(!found.is_empty());
    }

    /// A single-revision file with only a lock in it has nothing to clean —
    /// the lock is not surplus, it is the way back.
    #[test]
    fn a_lock_alone_is_not_hidden_data_to_remove() {
        let found = Hidden { revisions: 1, keeps_lock: true, ..Hidden::default() };
        assert!(found.is_empty(), "it offered to remove a lock's own copy");
    }
}
