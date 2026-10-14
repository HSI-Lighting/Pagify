//! Lifting drawn objects off one page and putting them down on another.
//!
//! Edit Object's copy used to be a picture of the page, a 3 px/pt render of
//! the object's box, and the owner's pasted lens drawing (VIVO mini, page 1)
//! came back visibly softer than the original: 0.50–0.87 of its edge
//! crispness depending on zoom, and no render scale fixes that. So a copy
//! carries the page's own operators and its original images instead.
//!
//! This half is pure: which resources a stream names, cutting a resource
//! dictionary down to them, and taking marked content out. No PDFium.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::{PdfError, Result};

use super::content::{self, Operation};
use super::object::{Dict, Lexer, Object};
use super::reader::{write_object, write_stream, File};

/// Resource category (`b"Font"`, `b"XObject"`, …) to the names a stream uses in it.
pub type Used = BTreeMap<Vec<u8>, BTreeSet<Vec<u8>>>;

/// Colour spaces an operator can name without a resource behind them.
const DEVICE_SPACES: [&[u8]; 4] = [b"DeviceGray", b"DeviceRGB", b"DeviceCMYK", b"Pattern"];

/// An inline image's own abbreviations for the device spaces (ISO 32000-1 table 93).
const INLINE_DEVICE_SPACES: [&[u8]; 6] = [b"DeviceGray", b"DeviceRGB", b"DeviceCMYK", b"G", b"RGB", b"CMYK"];

/// Every resource a content stream names, by category.
///
/// Inline images are read from the bytes between operations, because
/// [`content::parse`] skips them whole: `BI /CS /CS1 ID … EI` names a colour
/// space no operation shows.
pub fn used_resource_names(stream: &[u8]) -> Result<Used> {
    let operations = content::parse(stream)?;
    let mut used = Used::new();
    let mut add = |category: &[u8], name: &[u8]| {
        used.entry(category.to_vec()).or_default().insert(name.to_vec());
    };

    for op in &operations {
        if let Some((category, name)) = names_a_resource(op) {
            add(category, name);
        }
    }

    let mut at = 0;
    for span in operations.iter().map(|op| op.span.clone()).chain([stream.len()..stream.len()]) {
        for name in inline_image_spaces(stream, at..span.start)? {
            add(b"ColorSpace", &name);
        }
        at = span.end;
    }
    Ok(used)
}

/// The resource an operation names, as its category and the name.
fn names_a_resource(op: &Operation) -> Option<(&'static [u8], &[u8])> {
    let first = op.operands.first().and_then(Object::as_name);
    let last = op.operands.last().and_then(Object::as_name);
    match (op.operator.as_slice(), first, last) {
        (b"Tf", Some(name), _) => Some((b"Font", name)),
        (b"gs", Some(name), _) => Some((b"ExtGState", name)),
        (b"Do", Some(name), _) => Some((b"XObject", name)),
        (b"sh", Some(name), _) => Some((b"Shading", name)),
        (b"cs" | b"CS", Some(name), _) if !DEVICE_SPACES.contains(&name) => Some((b"ColorSpace", name)),
        // `scn` names a pattern only when its last operand is a name.
        (b"scn" | b"SCN", _, Some(name)) => Some((b"Pattern", name)),
        _ => None,
    }
}

/// `(category, old name)` to the new name — see [`rename_resources`].
pub type Renames = BTreeMap<(Vec<u8>, Vec<u8>), Vec<u8>>;

/// The stream with each resource it names in `renames` named the new way.
///
/// What makes pasted lines lines: they are written into the page's own
/// drawing, and the page may already have a `/GS0` or an `/Im0` that is
/// something else. Only the operations that name a renamed resource are
/// written again; every other byte is copied through. An inline image whose
/// colour space would need renaming is refused: its name is in bytes this
/// does not rewrite (carried blocks never hold one).
pub fn rename_resources(stream: &[u8], renames: &Renames) -> Result<Vec<u8>> {
    let operations = content::parse(stream)?;
    let mut at = 0;
    for span in operations.iter().map(|op| op.span.clone()).chain([stream.len()..stream.len()]) {
        for name in inline_image_spaces(stream, at..span.start)? {
            if renames.contains_key(&(b"ColorSpace".to_vec(), name)) {
                return Err(PdfError::Unsupported("an inline image in the drawing names a colour space the page also has"));
            }
        }
        at = span.end;
    }

    let mut edits = Vec::new();
    for op in &operations {
        let Some((category, name)) = names_a_resource(op) else { continue };
        let Some(new) = renames.get(&(category.to_vec(), name.to_vec())) else { continue };
        let mut bytes = Vec::new();
        for operand in &op.operands {
            let operand = match operand {
                Object::Name(n) if n.as_slice() == name => Object::Name(new.clone()),
                other => other.clone(),
            };
            write_object(&mut bytes, &operand);
            bytes.push(b' ');
        }
        bytes.extend_from_slice(&op.operator);
        edits.push((op.span.clone(), bytes));
    }
    Ok(content::splice(stream, &edits))
}

/// The named colour spaces of the inline images between two operations.
///
/// ponytail: a name only. An `[/Indexed /CS0 …]` array naming a resource as
/// its base is not followed; the census check on the clip is the backstop.
fn inline_image_spaces(stream: &[u8], gap: std::ops::Range<usize>) -> Result<Vec<Vec<u8>>> {
    let mut found = Vec::new();
    let mut lexer = Lexer::new(stream, gap.start);
    loop {
        lexer.skip_space();
        if lexer.at >= gap.end {
            return Ok(found);
        }
        let begin = lexer.at;
        if lexer.token() != b"BI" {
            return Err(PdfError::Unsupported("the stream holds something between its operations that is not an inline image"));
        }
        loop {
            lexer.skip_space();
            if stream[lexer.at..].starts_with(b"ID") {
                break;
            }
            let key = lexer.object()?;
            let value = lexer.object()?;
            if let (Some(b"CS" | b"ColorSpace"), Some(name)) = (key.as_name(), value.as_name()) {
                if !INLINE_DEVICE_SPACES.contains(&name) {
                    found.push(name.to_vec());
                }
            }
        }
        lexer.at = content::skip_inline_image(stream, begin)?;
    }
}

/// A resource dictionary cut down to the names `used` lists, plus `/ProcSet`.
///
/// Values are kept as they are (a reference stays a reference); a category
/// table behind a reference is read through it. A used name the resources do
/// not hold is a refusal: a clip that drew with it would draw something else.
pub fn prune_resources(file: &File<'_>, resources: &Dict, used: &Used) -> Result<Dict> {
    let mut out = Dict::default();
    if let Some(procset) = resources.get(b"ProcSet") {
        out.set(b"ProcSet", procset.clone());
    }
    for (category, names) in used {
        let table = resources
            .get(category)
            .map(|entry| file.resolve(entry))
            .transpose()?
            .ok_or(PdfError::Unsupported("the drawing uses a kind of resource the page does not have"))?;
        let table = table
            .as_dict()
            .ok_or(PdfError::Unsupported("a resource table on the page is not a dictionary"))?;
        let mut kept = Dict::default();
        for name in names {
            let value = table
                .get(name)
                .ok_or(PdfError::Unsupported("the drawing uses a resource the page does not have"))?;
            kept.set(name, value.clone());
        }
        out.set(category, Object::Dict(kept));
    }
    Ok(out)
}

/// The stream with its marked-content operators (`BMC BDC EMC MP DP`) cut out.
///
/// They never change what is drawn, and a lifted span can hold one end of a
/// pair. Cut with [`content::remove`], so every other byte (an inline image
/// included) is copied through.
pub fn without_marked_content(stream: &[u8]) -> Result<Vec<u8>> {
    without(stream, &[b"BMC", b"BDC", b"EMC", b"MP", b"DP"])
}

/// The stream with its text-state operators (`Tf Tc Tw Tz TL Ts Tr`) cut out.
///
/// A carried block replays the state in force, and a font set by words drawn
/// earlier is still in force for a shape after them: kept, its `Tf` would copy
/// a whole font into every pasted square. Only for blocks that draw no words.
pub fn without_text_state(stream: &[u8]) -> Result<Vec<u8>> {
    without(stream, &[b"Tf", b"Tc", b"Tw", b"Tz", b"TL", b"Ts", b"Tr"])
}

fn without(stream: &[u8], operators: &[&[u8]]) -> Result<Vec<u8>> {
    let operations = content::parse(stream)?;
    let cut: Vec<&Operation> = operations.iter().filter(|op| operators.contains(&op.operator.as_slice())).collect();
    Ok(content::remove(stream, &cut))
}

/// A whole one-page file: a page with `entries` (its boxes and `/Rotate`),
/// a page tree with `inherited` (the ones the page takes from it), `content`
/// as the page's stream, and `resources` with everything they reach in `from`
/// copied in ([`copy_graph`]). Nothing else of `from` comes along.
///
/// The boxes go where the source page had them, on the page or inherited,
/// because PDFium measures the two differently: `FPDFPage_GetMediaBox` does
/// not follow inheritance, so a rotated page whose MediaBox is on its page
/// tree is measured from its *turned* height. A clip that put the same box on
/// the page itself measured 180 pt away from the page it came from.
pub fn one_page_file(from: &File<'_>, entries: Dict, inherited: Dict, content: &[u8], resources: &Dict) -> Result<Vec<u8>> {
    const FIRST_COPY: u32 = 5;
    let (resources, copies) = copy_graph(from, &Object::Dict(resources.clone()), FIRST_COPY)?;

    let reference = |n: u32| Object::Reference(n, 0);
    let name = |n: &[u8]| Object::Name(n.to_vec());
    let mut page = entries;
    page.set(b"Type", name(b"Page"));
    page.set(b"Parent", reference(2));
    page.set(b"Contents", reference(4));
    page.set(b"Resources", resources);
    let written = |dict: Dict| {
        let mut out = Vec::new();
        write_object(&mut out, &Object::Dict(dict));
        out
    };
    let mut objects = vec![
        written(Dict(vec![(b"Type".to_vec(), name(b"Catalog")), (b"Pages".to_vec(), reference(2))])),
        written({
            let mut tree = inherited;
            tree.set(b"Type", name(b"Pages"));
            tree.set(b"Kids", Object::Array(vec![reference(3)]));
            tree.set(b"Count", Object::Number(b"1".to_vec()));
            tree
        }),
        written(page),
        write_stream(&Dict(vec![(b"Filter".to_vec(), name(b"FlateDecode"))]), &content::encode(content)?),
    ];
    objects.extend(copies.into_iter().map(|(_, body)| body));

    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n", objects.len() + 1).as_bytes(),
    );
    Ok(out)
}

/// Copy everything `root` reaches in `from` into fresh object numbers.
///
/// Returns `root` with every reference renumbered, and each copied object's
/// body, numbered `first_free..` (an object reached twice is copied once).
/// Streams are copied from their raw, still-encoded bytes with their filters
/// and a direct `/Length`. `/OC`, `/StructParent` and `/StructParents` are
/// dropped: the paste is drawn whatever layer it came from, and the target's
/// structure tree knows nothing of it. A graph that reaches a page, the page
/// tree or the catalogue (`/Parent`, `/P`, or a dict of that `/Type`) is
/// refused, because copying it would copy the source document.
pub fn copy_graph(from: &File<'_>, root: &Object, first_free: u32) -> Result<(Object, Vec<(u32, Vec<u8>)>)> {
    let mut copier = Copier { from, next: first_free, numbers: BTreeMap::new(), out: Vec::new() };
    let root = copier.object(root)?;
    copier.out.sort_by_key(|(number, _)| *number);
    Ok((root, copier.out))
}

struct Copier<'f, 'a> {
    from: &'f File<'a>,
    next: u32,
    /// Source number to copy number.
    numbers: BTreeMap<u32, u32>,
    out: Vec<(u32, Vec<u8>)>,
}

impl Copier<'_, '_> {
    fn object(&mut self, object: &Object) -> Result<Object> {
        Ok(match object {
            Object::Reference(number, _) => Object::Reference(self.number(*number)?, 0),
            Object::Array(items) => Object::Array(items.iter().map(|item| self.object(item)).collect::<Result<_>>()?),
            Object::Dict(dict) => Object::Dict(self.dict(dict)?),
            Object::Stream(..) => return Err(PdfError::Unsupported("a stream that is not an object of its own")),
            other => other.clone(),
        })
    }

    /// The copy's number, copying the object the first time it is reached.
    fn number(&mut self, source: u32) -> Result<u32> {
        if let Some(&copy) = self.numbers.get(&source) {
            return Ok(copy);
        }
        let copy = self.next;
        self.next += 1;
        // Noted before the walk goes in, so a cycle comes back to this number.
        self.numbers.insert(source, copy);
        let body = match self.from.object(source)? {
            Object::Stream(mut dict, raw) => {
                dict.remove(b"Length");
                write_stream(&self.dict(&dict)?, &self.from.bytes()[raw])
            }
            other => {
                let mut body = Vec::new();
                write_object(&mut body, &self.object(&other)?);
                body
            }
        };
        self.out.push((copy, body));
        Ok(copy)
    }

    fn dict(&mut self, dict: &Dict) -> Result<Dict> {
        let page_like = dict
            .get(b"Type")
            .and_then(Object::as_name)
            .is_some_and(|t| matches!(t, b"Page" | b"Pages" | b"Catalog"));
        if page_like || dict.get(b"Parent").is_some() || dict.get(b"P").is_some() {
            return Err(PdfError::Unsupported("the drawing's resources lead back into the document's pages"));
        }
        let mut out = Dict::default();
        for (key, value) in &dict.0 {
            if !matches!(key.as_slice(), b"OC" | b"StructParent" | b"StructParents") {
                out.0.push((key.clone(), self.object(value)?));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names<'a>(used: &'a Used, category: &[u8]) -> Vec<&'a str> {
        used.get(category)
            .map(|set| set.iter().map(|n| std::str::from_utf8(n).unwrap()).collect())
            .unwrap_or_default()
    }

    fn object(text: &str) -> Object {
        Lexer::new(text.as_bytes(), 0).object().expect("object")
    }

    fn dict(text: &str) -> Dict {
        object(text).as_dict().expect("a dict").clone()
    }

    /// A classic-xref file whose objects are numbered 1.. in the order given.
    pub(super) fn file_of(objects: &[&str]) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
        }
        let xref_at = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for offset in &offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF", objects.len() + 1)
                .as_bytes(),
        );
        out
    }

    #[test]
    fn used_names_cover_every_operator_that_names_a_resource() {
        let used = used_resource_names(
            b"/F1 12 Tf /GS0 gs /CS0 cs /P0 scn /Im0 Do /Sh0 sh /DeviceRGB CS /Pattern cs 0.5 /P1 SCN 1 0 0 sc",
        )
        .expect("used");
        assert_eq!(names(&used, b"Font"), ["F1"]);
        assert_eq!(names(&used, b"ExtGState"), ["GS0"]);
        // Device spaces and /Pattern are not resources.
        assert_eq!(names(&used, b"ColorSpace"), ["CS0"]);
        assert_eq!(names(&used, b"Pattern"), ["P0", "P1"]);
        assert_eq!(names(&used, b"XObject"), ["Im0"]);
        assert_eq!(names(&used, b"Shading"), ["Sh0"]);
        assert_eq!(used.len(), 6, "{used:?}");
    }

    /// `content::parse` skips an inline image whole, so its colour space is
    /// found in the bytes between operations.
    #[test]
    fn an_inline_images_colour_space_is_used_too() {
        let used = used_resource_names(
            b"q 1 0 0 1 0 0 cm BI /W 1 /H 1 /BPC 8 /CS /CS1 ID x EI BI /W 1 /H 1 /BPC 8 /CS /RGB ID xyz EI Q",
        )
        .expect("used");
        assert_eq!(names(&used, b"ColorSpace"), ["CS1"]);
        assert_eq!(used.len(), 1, "{used:?}");
    }

    #[test]
    fn pruning_keeps_only_used_names_and_refuses_a_missing_one() {
        // The Font table is behind a reference, as producers often write it.
        let bytes = file_of(&[
            "<< /Type /Catalog >>",
            "<< /F1 3 0 R /F2 4 0 R >>",
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
            "<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>",
        ]);
        let file = File::parse(&bytes).expect("parse");
        let resources = dict("<< /ProcSet [/PDF /Text] /Font 2 0 R /XObject << /Im0 9 0 R >> >>");

        let one = |category: &[u8], name: &[u8]| -> Used {
            BTreeMap::from([(category.to_vec(), BTreeSet::from([name.to_vec()]))])
        };
        let pruned = prune_resources(&file, &resources, &one(b"Font", b"F1")).expect("prune");
        assert_eq!(pruned, dict("<< /ProcSet [/PDF /Text] /Font << /F1 3 0 R >> >>"));

        assert!(matches!(
            prune_resources(&file, &resources, &one(b"Font", b"F9")),
            Err(PdfError::Unsupported(_))
        ));
        assert!(matches!(
            prune_resources(&file, &resources, &one(b"Shading", b"Sh0")),
            Err(PdfError::Unsupported(_))
        ));
    }

    #[test]
    fn marked_content_is_taken_out_and_nothing_else() {
        let source: &[u8] = b"/OC /MC0 BDC 0 0 m 1 1 l S EMC /Tag MP BI /W 1 /H 1 /BPC 8 /CS /G ID x EI /T <</MCID 3>> DP";
        let out = without_marked_content(source).expect("strip");

        let kept = content::parse(&out).expect("parse");
        let wanted: Vec<_> = content::parse(source)
            .unwrap()
            .into_iter()
            .filter(|op| !matches!(op.operator.as_slice(), b"BMC" | b"BDC" | b"EMC" | b"MP" | b"DP"))
            .collect();
        assert_eq!(kept.len(), 3);
        for (now, was) in kept.iter().zip(&wanted) {
            assert_eq!(&out[now.span.clone()], &source[was.span.clone()], "each kept operator byte for byte");
        }
        // The inline image, which is not an operation, is still there.
        assert!(out.windows(14).any(|w| w == b"/CS /G ID x EI"), "{}", String::from_utf8_lossy(&out));
    }

    /// Every reference anywhere inside `object`.
    fn references(object: &Object, out: &mut Vec<u32>) {
        match object {
            Object::Reference(n, _) => out.push(*n),
            Object::Array(items) => items.iter().for_each(|item| references(item, out)),
            Object::Dict(dict) | Object::Stream(dict, _) => dict.0.iter().for_each(|(_, v)| references(v, out)),
            _ => {}
        }
    }

    fn copied(copies: &[(u32, Vec<u8>)], number: u32) -> Object {
        let (_, body) = copies.iter().find(|(n, _)| *n == number).expect("that copy");
        Lexer::new(body, 0).object().expect("a copy reads back")
    }

    fn reference_at(dict: &Dict, path: &[&[u8]]) -> u32 {
        let mut at = dict.clone();
        for key in &path[..path.len() - 1] {
            at = at.get(key).and_then(Object::as_dict).expect("path").clone();
        }
        at.get(path[path.len() - 1]).and_then(Object::as_reference).expect("a reference").0
    }

    #[test]
    fn a_resource_graph_is_copied_with_every_reference_renumbered() {
        let bytes = file_of(&[
            "<< /Type /Catalog >>",
            "<< /Type /Font /Subtype /TrueType /BaseFont /Body /FontDescriptor 3 0 R >>",
            "<< /Type /FontDescriptor /FontName /Body /FontFile2 4 0 R >>",
            "<< /Length 5 /Filter /FlateDecode >>\nstream\nABCDE\nendstream",
            "<< /Type /ExtGState /SMask 6 0 R >>",
            "<< /Type /Mask /S /Luminosity /G 7 0 R >>",
            "<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /Im 8 0 R >> >> /Length 3 >>\nstream\nabc\nendstream",
            "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray /Filter [/ASCIIHexDecode] /Length 3 >>\nstream\n7F>\nendstream",
        ]);
        let file = File::parse(&bytes).expect("parse");
        let root = object("<< /Font << /F1 2 0 R >> /ExtGState << /GS0 5 0 R >> /ProcSet [/PDF] >>");

        let (new_root, copies) = copy_graph(&file, &root, 100).expect("copy");

        let numbers: BTreeSet<u32> = copies.iter().map(|(n, _)| *n).collect();
        assert_eq!(numbers, (100..107).collect(), "seven objects, numbered from the first free one");

        let mut refs = Vec::new();
        references(&new_root, &mut refs);
        for (number, _) in &copies {
            references(&copied(&copies, *number), &mut refs);
        }
        assert!(refs.iter().all(|n| numbers.contains(n)), "a reference left pointing into the source: {refs:?}");

        // Streams: raw bytes as they were, filters kept, /Length direct.
        let root = new_root.as_dict().unwrap();
        let font = copied(&copies, reference_at(root, &[b"Font", b"F1"]));
        let descriptor = copied(&copies, reference_at(font.as_dict().unwrap(), &[b"FontDescriptor"]));
        let program = reference_at(descriptor.as_dict().unwrap(), &[b"FontFile2"]);
        let gs = copied(&copies, reference_at(root, &[b"ExtGState", b"GS0"]));
        let mask = copied(&copies, reference_at(gs.as_dict().unwrap(), &[b"SMask"]));
        let form = reference_at(mask.as_dict().unwrap(), &[b"G"]);
        let image = reference_at(copied(&copies, form).as_dict().unwrap(), &[b"Resources", b"XObject", b"Im"]);
        for (number, raw, filter) in [(program, &b"ABCDE"[..], "/FlateDecode"), (form, b"abc", ""), (image, b"7F>", "[/ASCIIHexDecode]")] {
            let (_, body) = copies.iter().find(|(n, _)| *n == number).unwrap();
            let Object::Stream(dict, range) = copied(&copies, number) else { panic!("{number} is not a stream") };
            assert_eq!(&body[range], raw);
            assert_eq!(dict.get(b"Length").and_then(Object::as_i64), Some(raw.len() as i64));
            let mut written = Vec::new();
            if let Some(f) = dict.get(b"Filter") {
                write_object(&mut written, f);
            }
            assert_eq!(String::from_utf8(written).unwrap(), filter);
        }
    }

    #[test]
    fn a_shared_object_is_copied_once() {
        let bytes = file_of(&[
            "<< /Type /Catalog >>",
            "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray /Length 1 >>\nstream\nZ\nendstream",
        ]);
        let file = File::parse(&bytes).expect("parse");
        let (root, copies) = copy_graph(&file, &object("<< /XObject << /A 2 0 R /B 2 0 R >> >>"), 50).expect("copy");
        assert_eq!(copies.len(), 1);
        let root = root.as_dict().unwrap();
        assert_eq!(reference_at(root, &[b"XObject", b"A"]), 50);
        assert_eq!(reference_at(root, &[b"XObject", b"B"]), 50);
    }

    #[test]
    fn a_cycle_does_not_loop() {
        let bytes = file_of(&["<< /Type /Catalog >>", "<< /Next 3 0 R >>", "<< /Next 2 0 R >>"]);
        let file = File::parse(&bytes).expect("parse");
        let (root, copies) = copy_graph(&file, &object("<< /X 2 0 R >>"), 10).expect("copy");
        assert_eq!(copies.len(), 2);
        let a = reference_at(root.as_dict().unwrap(), &[b"X"]);
        let b = reference_at(copied(&copies, a).as_dict().unwrap(), &[b"Next"]);
        assert_eq!(reference_at(copied(&copies, b).as_dict().unwrap(), &[b"Next"]), a);
    }

    #[test]
    fn a_graph_that_reaches_a_page_is_refused() {
        let bytes = file_of(&[
            "<< /Type /Catalog /Pages 6 0 R >>",
            "<< /Parent 6 0 R >>",
            "<< /P 7 0 R >>",
            "<< /Type /Page >>",
            "<< /Kids [4 0 R] >>",
            "<< /Type /Pages /Kids [] /Count 0 >>",
            "<< /Type /Annot >>",
        ]);
        let file = File::parse(&bytes).expect("parse");
        for root in ["<< /X 2 0 R >>", "<< /X 3 0 R >>", "<< /X 5 0 R >>", "<< /X 6 0 R >>", "<< /X 1 0 R >>"] {
            assert!(
                matches!(copy_graph(&file, &object(root), 20), Err(PdfError::Unsupported(_))),
                "{root} was copied"
            );
        }
    }

    #[test]
    fn optional_content_and_structure_keys_are_dropped() {
        let bytes = file_of(&[
            "<< /Type /Catalog >>",
            "<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /OC 3 0 R /StructParents 4 /StructParent 2 /Length 1 >>\nstream\nn\nendstream",
            "<< /Type /OCG /Name (Layer) >>",
        ]);
        let file = File::parse(&bytes).expect("parse");
        let (root, copies) = copy_graph(&file, &object("<< /XObject << /Fm0 2 0 R >> >>"), 30).expect("copy");
        assert_eq!(copies.len(), 1, "the layer it was on is not copied");
        let form = copied(&copies, reference_at(root.as_dict().unwrap(), &[b"XObject", b"Fm0"]));
        let dict = form.as_dict().unwrap();
        for key in [&b"OC"[..], b"StructParents", b"StructParent"] {
            assert!(dict.get(key).is_none(), "{} kept", String::from_utf8_lossy(key));
        }
        assert!(dict.get(b"BBox").is_some());
    }

    #[test]
    fn renamed_resources_are_renamed_where_they_are_named_and_nowhere_else() {
        let stream: &[u8] = b"/F1 9 Tf /GS0 gs /CS0 cs 0.5 /P0 scn /Im0 Do /Sh0 sh /DeviceRGB CS (GS0) Tj /GS1 gs";
        let renames: Renames = [
            (&b"Font"[..], &b"F1"[..]),
            (b"ExtGState", b"GS0"),
            (b"ColorSpace", b"CS0"),
            (b"Pattern", b"P0"),
            (b"XObject", b"Im0"),
            (b"Shading", b"Sh0"),
        ]
        .into_iter()
        .map(|(category, name)| ((category.to_vec(), name.to_vec()), [name, b"_1"].concat()))
        .collect();

        let out = rename_resources(stream, &renames).expect("rename");
        let used = used_resource_names(&out).expect("used");
        assert_eq!(names(&used, b"Font"), ["F1_1"]);
        assert_eq!(names(&used, b"ExtGState"), ["GS0_1", "GS1"], "one not renamed stays as it was");
        assert_eq!(names(&used, b"ColorSpace"), ["CS0_1"]);
        assert_eq!(names(&used, b"Pattern"), ["P0_1"]);
        assert_eq!(names(&used, b"XObject"), ["Im0_1"]);
        assert_eq!(names(&used, b"Shading"), ["Sh0_1"]);
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("(GS0) Tj") && text.contains("/DeviceRGB CS") && text.contains("0.5 /P0_1 scn"), "{text}");
    }

    #[test]
    fn an_inline_image_whose_colour_space_must_be_renamed_is_refused() {
        let renames: Renames = [((b"ColorSpace".to_vec(), b"CS1".to_vec()), b"CS1_1".to_vec())].into_iter().collect();
        let stream = b"q BI /W 1 /H 1 /BPC 8 /CS /CS1 ID x EI Q";
        assert!(matches!(rename_resources(stream, &renames), Err(PdfError::Unsupported(_))));
    }
}
