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

use std::collections::{BTreeMap, BTreeSet};

use crate::error::Result;

use super::object::Dict;
use super::flatten::{collect_pages, PageAt};
use super::lift;
use super::{content, write_object, write_stream, File, Object};

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
    /// Interactive form fields that still carry a value — something somebody
    /// typed in, not the field's own definition.
    pub form_fields: usize,
    /// An XFA dynamic-form description, which duplicates and can override the
    /// AcroForm fields beside it.
    pub xfa: bool,
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
            && self.form_fields == 0
            && !self.xfa
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
        if self.form_fields > 0 {
            found.push(format!(
                "{} form field value(s)",
                self.form_fields
            ));
        }
        if self.xfa {
            found.push("an XFA dynamic form".into());
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
            form_fields: self.form_fields.saturating_sub(remaining.form_fields),
            xfa: self.xfa && !remaining.xfa,
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

    let reachable = reachable(
        file,
        &BTreeMap::new(),
        [file.trailer().get(b"Root"), file.trailer().get(b"Info")]
            .into_iter()
            .flatten()
            .filter_map(Object::as_reference)
            .map(|(number, _)| number),
    );
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

    // A field's own `/V` is what somebody typed in — the same secrecy a
    // filled-in text box or checkbox on the page would deserve, but held in
    // the form's data model rather than drawn. `/FT` marks a field (or a
    // field merged with its widget annotation) and appears nowhere else, so
    // it names exactly the dictionaries this looks at. Found by audit.
    for object in reachable.iter().filter_map(|n| file.object(*n).ok()) {
        let Some(dict) = object.as_dict() else { continue };
        if dict.get(b"FT").is_some() && dict.get(b"V").is_some() {
            found.form_fields += 1;
        }
        if dict.get(b"XFA").is_some() {
            found.xfa = true;
        }
    }
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
    // `/FT` marks an interactive form field, or a field merged with its own
    // widget annotation, and appears nowhere else — the same signal
    // `survey` counts by. Its `/V` and `/DV` are what somebody typed in and
    // what it resets to, not the field's own definition.
    let is_field = dict.get(b"FT").is_some();
    for (key, value) in &dict.0 {
        if key == b"XFA" {
            // The whole dynamic-form description, which duplicates and can
            // override the AcroForm fields beside it. Found by audit.
            changed = true;
            continue;
        }
        if is_field && (key == b"V" || key == b"DV") {
            changed = true;
            continue;
        }
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
        // `write_object` cannot write a stream's bytes — its second field is
        // a range into *this* file, and the generic writer has no bytes to
        // read that range from, so it silently fell through to writing the
        // dictionary alone: a content stream, an image, a font program with
        // a script action scrubbed from its own dictionary came back with
        // its actual data gone. Sanitising a script off a stream must not
        // sanitise the stream out of existence. Found by audit.
        // `write_object` cannot write a stream's bytes — its second field is
        // a range into *this* file, and the generic writer has no bytes to
        // read that range from, so it silently fell through to writing the
        // dictionary alone: a content stream, an image, a font program with
        // a script action scrubbed from its own dictionary came back with
        // its actual data gone. Sanitising a script off a stream must not
        // sanitise the stream out of existence. Found by audit.
        let body = match object {
            Object::Stream(dict, range) => write_stream(dict, &bytes[range.clone()]),
            other => {
                let mut body = Vec::new();
                write_object(&mut body, other);
                body
            }
        };
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
    let reachable = reachable(file, &rewritten, root_number);
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

/// Everything reachable from `starts`, following references — one iterative walk with **no depth
/// limit**, used by the survey, the clean, and the full save's own garbage collection.
///
/// **The visited set is the only cycle guard.** The old walk stopped at depth 96 to keep a file that
/// points an object at itself from hanging, which also stopped real documents: outlines are a
/// `/First` then `/Next` chain, so 300 sibling bookmarks were cut to 96. A file cannot recurse forever
/// without revisiting a number, so a set is enough and a cap only loses real objects.
///
/// `rewritten` is the clean's own replacement objects; the survey and the full save pass an empty map.
/// An object that cannot be read is reached but not followed.
fn reachable(
    file: &File<'_>,
    rewritten: &BTreeMap<u32, Object>,
    starts: impl IntoIterator<Item = u32>,
) -> BTreeSet<u32> {
    let mut seen = BTreeSet::new();
    let mut stack: Vec<u32> = starts.into_iter().collect();
    while let Some(number) = stack.pop() {
        if !seen.insert(number) {
            continue;
        }
        let object = match rewritten.get(&number) {
            Some(object) => object.clone(),
            None => match file.object(number) {
                Ok(object) => object,
                Err(_) => continue,
            },
        };
        references(&object, &mut |n| stack.push(n));
    }
    seen
}

/// The pages' unused `/XObject` and `/Font` entries taken off, and how many went.
///
/// **A removed picture or line leaves its name on the page's list.** Edit Object's delete takes
/// the drawing operator out of the page's content, but `PgFlat3` (and the image and its `/SMask`)
/// stay in `/Resources`, so the page still refers to the bytes and `drop_unreachable` rightly keeps
/// them — which is why PRISMA stayed at 796 KB after its pasted screenshot was removed.
///
/// Only `/XObject` and `/Font` entries are ever removed: every other category is left exactly as it
/// is. What a page uses is [`lift::used_resource_names`] over its `/Contents`, decoded with
/// [`content::decode`] and joined with a newline; a page whose drawing cannot be decoded or parsed
/// is skipped whole. Pages whose `/Resources` is one indirect object are pruned to the **union** of
/// their uses, and only when every object in the file that refers to it is one of those pages. A
/// page that inherits its resources has none of its own to prune, so it is skipped. A category table
/// that is itself an indirect object is changed only when the resources holder is its only referrer.
///
/// **Anything that could be leaning on the page's list skips the group:** an annotation appearance
/// with no `/Resources` of its own, a Form XObject with none, a Type3 font with none, or a pattern
/// stream with none — old files draw those with the page's names.
pub fn drop_unused_resources(bytes: &[u8]) -> Result<(Vec<u8>, usize)> {
    let file = File::parse(bytes)?;
    // **A still-encrypted copy is not pruned.** A save can reach the fix-ups before a rewrite, with
    // `/Encrypt` in the trailer and ciphertext in the streams; parsing that as drawing could name
    // nothing — or happen to parse — and take every font and image off its page. Same check
    // `without_password` makes.
    if file.trailer().get(b"Encrypt").is_some() {
        return Ok((bytes.to_vec(), 0));
    }
    let Some(root) = dict_of(&file, file.trailer().get(b"Root")) else {
        return Ok((bytes.to_vec(), 0));
    };
    let mut pages = Vec::new();
    if let Some(tree) = root.get(b"Pages") {
        collect_pages(&file, tree, None, 0, &mut BTreeSet::new(), &mut pages)?;
    }

    // Who refers to what, once, over every readable object.
    let mut referrers: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for number in file.numbers().collect::<Vec<_>>() {
        if let Ok(object) = file.object(number) {
            references(&object, &mut |n| {
                referrers.entry(n).or_default().insert(number);
            });
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Holder {
        /// The page's `/Resources` is a direct dictionary: only that page.
        Page(usize),
        /// An indirect object, possibly named by several pages.
        Indirect(u32),
    }

    let mut work: Vec<(usize, Holder)> = Vec::new();
    for (index, page) in pages.iter().enumerate() {
        match page.dict.get(b"Resources") {
            Some(Object::Dict(_)) => work.push((index, Holder::Page(index))),
            Some(Object::Reference(number, _)) => work.push((index, Holder::Indirect(*number))),
            // None of its own: inherited, so not this page's list to prune.
            _ => {}
        }
    }
    let mut holders: Vec<(Holder, Vec<usize>)> = Vec::new();
    for (index, holder) in work {
        match holders.iter_mut().find(|(h, _)| *h == holder) {
            Some((_, group)) => group.push(index),
            None => holders.push((holder, vec![index])),
        }
    }

    let mut replacements: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut dropped = 0usize;
    for (holder, group) in holders {
        let holder_number = match holder {
            Holder::Indirect(number) => {
                // The union is safe only while these pages are the holder's only referrers.
                let Some(found) = referrers.get(&number) else { continue };
                if !found.iter().all(|n| group.iter().any(|&i| pages[i].number == *n)) {
                    continue;
                }
                number
            }
            Holder::Page(index) => pages[index].number,
        };

        let mut used: lift::Used = BTreeMap::new();
        let mut sound = true;
        for &index in &group {
            let Some(page_used) = page_uses(&file, &pages[index]) else {
                sound = false;
                break;
            };
            for (category, names) in page_used {
                used.entry(category).or_default().extend(names);
            }
            if appearance_without_resources(&file, &pages[index]) {
                sound = false;
                break;
            }
        }
        if !sound {
            continue;
        }

        let holder_dict = match holder {
            Holder::Indirect(number) => match file.object(number) {
                Ok(Object::Dict(dict)) => dict,
                _ => continue,
            },
            Holder::Page(index) => {
                let Some(Object::Dict(dict)) = pages[index].dict.get(b"Resources") else { continue };
                dict.clone()
            }
        };
        if leans_on_page_names(&file, &holder_dict) {
            continue;
        }

        let mut changed = holder_dict.clone();
        let mut holder_changed = false;
        for category in [b"XObject".as_slice(), b"Font".as_slice()] {
            let Some(stored) = holder_dict.get(category) else { continue };
            let Ok(Object::Dict(table)) = file.resolve(stored) else { continue };
            let names = used.get(category);
            let mut pruned = table.clone();
            pruned.0.retain(|(name, _)| names.is_some_and(|names| names.contains(name)));
            if pruned.0.len() == table.0.len() {
                continue;
            }
            let removed = table.0.len() - pruned.0.len();
            match stored {
                Object::Reference(number, _) => {
                    // The table is shared unless the resources holder is its only referrer.
                    let only = referrers
                        .get(number)
                        .is_some_and(|set| set.len() == 1 && set.contains(&holder_number));
                    if !only {
                        continue;
                    }
                    let mut body = Vec::new();
                    write_object(&mut body, &Object::Dict(pruned));
                    replacements.push((*number, body));
                    dropped += removed;
                }
                _ => {
                    dropped += removed;
                    changed.set(category, Object::Dict(pruned));
                    holder_changed = true;
                }
            }
        }
        if !holder_changed {
            continue;
        }
        let (number, dict) = match holder {
            Holder::Indirect(number) => (number, changed),
            Holder::Page(index) => (pages[index].number, {
                let mut dict = pages[index].dict.clone();
                dict.set(b"Resources", Object::Dict(changed));
                dict
            }),
        };
        let mut body = Vec::new();
        write_object(&mut body, &Object::Dict(dict));
        replacements.push((number, body));
    }

    if replacements.is_empty() {
        return Ok((bytes.to_vec(), 0));
    }
    Ok((file.rewrite(&replacements)?, dropped))
}

/// A value as a dictionary, through any reference.
fn dict_of(file: &File<'_>, value: Option<&Object>) -> Option<Dict> {
    let value = value?;
    match file.resolve(value).ok()? {
        Object::Dict(dict) => Some(dict),
        _ => None,
    }
}

/// A resources dict's `category` table, when it is a dict.
fn table(file: &File<'_>, resources: &Dict, category: &[u8]) -> Option<Dict> {
    dict_of(file, resources.get(category))
}

/// The resource names one page's own drawing uses, or `None` when a stream cannot be read.
fn page_uses(file: &File<'_>, page: &PageAt) -> Option<lift::Used> {
    let contents = page.dict.get(b"Contents")?;
    let items = match file.resolve(contents).ok()? {
        Object::Array(items) => items,
        _ => vec![contents.clone()],
    };
    let mut drawing = Vec::new();
    for item in items {
        let Object::Stream(dict, range) = file.resolve(&item).ok()? else { return None };
        let decoded = content::decode(&dict, &file.bytes()[range])?;
        drawing.extend_from_slice(&decoded);
        drawing.push(b'\n');
    }
    lift::used_resource_names(&drawing).ok()
}

/// Whether an appearance stream (any `/AP` mode or state) has no `/Resources` of its own.
///
/// **One level deep, as ISO 32000-1 §12.5.5 defines `/AP`:** each entry is a stream, or a
/// dictionary whose values are streams. Nothing below that is an appearance, and following a
/// dictionary that points at itself overflowed the stack — a crash no `unwrap_or_else` can catch.
fn appearance_without_resources(file: &File<'_>, page: &PageAt) -> bool {
    let Some(annots) = page.dict.get(b"Annots") else { return false };
    let Ok(Object::Array(items)) = file.resolve(annots) else { return false };
    for item in items {
        let Ok(Object::Dict(annot)) = file.resolve(&item) else { continue };
        let Some(ap) = annot.get(b"AP") else { continue };
        let Ok(Object::Dict(modes)) = file.resolve(ap) else { continue };
        if modes.0.iter().any(|(_, mode)| appearance_entry_lacks_resources(file, mode)) {
            return true;
        }
    }
    false
}

/// One `/AP` entry: a stream, or a dictionary of state streams.
fn appearance_entry_lacks_resources(file: &File<'_>, value: &Object) -> bool {
    match file.resolve(value) {
        Ok(Object::Stream(dict, _)) => dict.get(b"Resources").is_none(),
        Ok(Object::Dict(states)) => states.0.iter().any(|(_, state)| {
            matches!(
                file.resolve(state),
                Ok(Object::Stream(dict, _)) if dict.get(b"Resources").is_none()
            )
        }),
        _ => false,
    }
}

/// Whether a Form, Type3 font or pattern stream in the list draws with the page's names.
fn leans_on_page_names(file: &File<'_>, resources: &Dict) -> bool {
    if let Some(xobjects) = table(file, resources, b"XObject") {
        for (_, value) in &xobjects.0 {
            if let Ok(Object::Stream(dict, _)) = file.resolve(value) {
                if dict.get(b"Subtype").and_then(Object::as_name) == Some(b"Form")
                    && dict.get(b"Resources").is_none()
                {
                    return true;
                }
            }
        }
    }
    if let Some(fonts) = table(file, resources, b"Font") {
        for (_, value) in &fonts.0 {
            if let Ok(Object::Dict(dict)) = file.resolve(value) {
                if dict.get(b"Subtype").and_then(Object::as_name) == Some(b"Type3")
                    && dict.get(b"Resources").is_none()
                {
                    return true;
                }
            }
        }
    }
    if let Some(patterns) = table(file, resources, b"Pattern") {
        for (_, value) in &patterns.0 {
            if let Ok(Object::Stream(dict, _)) = file.resolve(value) {
                if dict.get(b"Resources").is_none() {
                    return true;
                }
            }
        }
    }
    false
}

/// A copy with the objects nothing refers to left out, and how many went.
///
/// **Acrobat's and Foxit's full saves do this too** (`PDSaveCollectGarbage`, "Remove unreferenced
/// objects, often reducing file size"). PDFium's full save writes every object made in the session,
/// so each turn or resize of a picture used to leave a whole copy of it in the file. Nothing on a
/// page changes: only objects the file does not point at any more are omitted.
///
/// Starts are every reference in every trailer entry except `/Prev` and `/XRefStm`, which name
/// places in the file rather than objects. The input bytes are returned unchanged when there is
/// nothing to drop.
pub fn drop_unreachable(bytes: &[u8]) -> Result<(Vec<u8>, usize)> {
    let file = File::parse(bytes)?;
    let starts = file
        .trailer()
        .0
        .iter()
        .filter(|(key, _)| key.as_slice() != b"Prev" && key.as_slice() != b"XRefStm")
        .flat_map(|(_, value)| {
            let mut found = Vec::new();
            references(value, &mut |n| found.push(n));
            found
        });
    let reachable = reachable(&file, &BTreeMap::new(), starts);
    let drop: Vec<u32> = file.numbers().filter(|n| !reachable.contains(n)).collect();
    if drop.is_empty() {
        return Ok((bytes.to_vec(), 0));
    }
    let cleaned = file.rewrite_dropping(&[], &[], &Dict(Vec::new()), &drop)?;
    Ok((cleaned, drop.len()))
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
            form_fields: 3,
            xfa: true,
            keeps_lock: false,
        };
        let said = found.describe();
        for expected in [
            "earlier version",
            "Author",
            "XMP",
            "embedded",
            "JavaScript",
            "8 unreachable",
            "3 form field",
            "XFA",
        ] {
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

    /// A file with one AcroForm text field carrying a value, and an XFA
    /// stream beside it. **Found by audit**: neither used to be looked at by
    /// `survey` or `strip` at all — a filled-in field's value, and the whole
    /// dynamic-form description that can override it, both survived a
    /// "clean" document unmentioned and untouched.
    #[test]
    fn a_survey_finds_form_field_values_and_xfa_and_strip_removes_them() {
        let mut out = Vec::new();
        let mut offsets = Vec::new();
        out.extend_from_slice(b"%PDF-1.7\n");

        offsets.push(out.len());
        out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>\nendobj\n");
        offsets.push(out.len());
        out.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        offsets.push(out.len());
        out.extend_from_slice(b"3 0 obj\n<< /Type /Page /Parent 2 0 R >>\nendobj\n");
        offsets.push(out.len());
        out.extend_from_slice(b"4 0 obj\n<< /Fields [5 0 R] /XFA 6 0 R >>\nendobj\n");
        offsets.push(out.len());
        out.extend_from_slice(b"5 0 obj\n<< /FT /Tx /T (secret) /V (TopSecretValue123) >>\nendobj\n");
        offsets.push(out.len());
        out.extend_from_slice(b"6 0 obj\n<< /Length 4 >>\nstream\nXFA!\nendstream\nendobj\n");

        let xref_at = out.len();
        out.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
        for offset in &offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(b"trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n");
        out.extend_from_slice(format!("{xref_at}\n%%EOF").as_bytes());

        let file = File::parse(&out).expect("parse");
        let found = survey(&file, &out).expect("survey");
        assert_eq!(found.form_fields, 1, "{found:?}");
        assert!(found.xfa, "{found:?}");
        assert!(!found.is_empty());

        let (cleaned, report) = strip(&file, &out).expect("strip");
        assert_eq!(report.after.form_fields, 0, "{:?}", report.after);
        assert!(!report.after.xfa, "{:?}", report.after);

        let needle: &[u8] = b"TopSecretValue123";
        assert!(
            !cleaned.windows(needle.len()).any(|w| w == needle),
            "the field's value survived the clean"
        );
    }

    /// **Found by audit.** `write_object` cannot write a stream's bytes — a
    /// stream's second field is a range into the file, meaningless without
    /// the bytes to read it from — so a stream whose *dictionary* needed a
    /// script action scrubbed out of it came back rewritten as a bare
    /// dictionary, its actual data gone. Sanitising a script off a stream
    /// must not sanitise the stream's content out of existence.
    #[test]
    fn a_script_carrying_stream_keeps_its_bytes_after_cleaning() {
        let mut out = Vec::new();
        let mut offsets = Vec::new();
        out.extend_from_slice(b"%PDF-1.7\n");

        offsets.push(out.len());
        out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        offsets.push(out.len());
        out.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        offsets.push(out.len());
        out.extend_from_slice(b"3 0 obj\n<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>\nendobj\n");

        offsets.push(out.len());
        let payload = b"BT (Hello World) Tj ET";
        out.extend_from_slice(
            format!("4 0 obj\n<< /AA << /O 5 0 R >> /Length {} >>\nstream\n", payload.len())
                .as_bytes(),
        );
        out.extend_from_slice(payload);
        out.extend_from_slice(b"\nendstream\nendobj\n");

        offsets.push(out.len());
        out.extend_from_slice(b"5 0 obj\n<< /S /JavaScript /JS (evil) >>\nendobj\n");

        let xref_at = out.len();
        out.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
        for offset in &offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(b"trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n");
        out.extend_from_slice(format!("{xref_at}\n%%EOF").as_bytes());

        let file = File::parse(&out).expect("parse");
        let found = survey(&file, &out).expect("survey");
        assert!(found.javascript, "{found:?}");

        let (cleaned, report) = strip(&file, &out).expect("strip");
        assert!(!report.after.javascript, "{:?}", report.after);

        let again = File::parse(&cleaned).expect("parse the cleaned file");
        let stream_object = again.object(4).expect("object 4 (the stream) must still exist");
        let Object::Stream(dict, range) = &stream_object else {
            panic!("object 4 came back as something other than a stream: {stream_object:?}");
        };
        assert!(dict.get(b"AA").is_none(), "the script action survived");
        assert_eq!(&cleaned[range.clone()], payload, "the stream's own bytes were dropped");
    }
}
