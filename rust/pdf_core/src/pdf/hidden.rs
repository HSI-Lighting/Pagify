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
        // An action on opening is JavaScript's other home.
        if root.get(b"OpenAction").is_some() {
            found.javascript = found.javascript || open_action_runs_script(file, root);
        }
    }

    let reachable = reachable_from_root(file);
    found.unreachable = file.numbers().filter(|n| !reachable.contains(n)).count();
    Ok(found)
}

/// A copy with the hidden data taken out.
///
/// Everything on the pages is untouched — this rewrites the file's *structure*,
/// never a content stream. Objects nothing reaches are left out, `/Info` and the
/// XMP packet go, and the result is one revision rather than several, which is
/// what removes the earlier versions.
pub fn strip(file: &File<'_>, bytes: &[u8]) -> Result<(Vec<u8>, Hidden)> {
    let found = survey(file, bytes)?;

    // The catalogue without its metadata pointer.
    let mut replacements: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut cleaned_root = None;
    if let Some(Object::Reference(number, _)) = file.trailer().get(b"Root") {
        if let Ok(Object::Dict(mut root)) = file.object(*number) {
            root.remove(b"Metadata");
            let mut body = Vec::new();
            write_object(&mut body, &Object::Dict(root.clone()));
            replacements.push((*number, body));
            cleaned_root = Some((*number, root));
        }
    }

    // **Reachability is computed against the file as it will be, not as it
    // is.** Taking `/Info` off the trailer and `/Metadata` off the catalogue
    // orphans those objects; walking the original would still count them as
    // reached and copy them through, leaving the metadata in the file with
    // nothing pointing at it. Measured on a catalogue: two objects survived a
    // clean that way.
    let mut reachable = BTreeSet::new();
    if let Some((number, root)) = &cleaned_root {
        reachable.insert(*number);
        references(&Object::Dict(root.clone()), &mut |n| {
            walk(file, n, &mut reachable, 0)
        });
    }
    let drop: Vec<u32> = file.numbers().filter(|n| !reachable.contains(n)).collect();

    // The trailer loses `/Info` entirely. An empty `/Info` dictionary would
    // still say a Pagify-shaped tool had been over the file.
    let mut trailer = Dict(Vec::new());
    trailer.set(b"Info", Object::Null);

    let cleaned = file.rewrite_dropping(&replacements, &[], &trailer, &drop)?;
    Ok((cleaned, found))
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

/// Whether the document runs a script when it opens.
fn open_action_runs_script(file: &File<'_>, root: &Dict) -> bool {
    let Some(action) = root.get(b"OpenAction").and_then(|a| file.resolve(a).ok()) else {
        return false;
    };
    // An `/OpenAction` is usually a destination — go to page 3 — which is not
    // script and not hidden. Only `/S /JavaScript` is.
    action
        .as_dict()
        .and_then(|d| d.get(b"S"))
        .and_then(Object::as_name)
        .is_some_and(|s| s == b"JavaScript")
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
