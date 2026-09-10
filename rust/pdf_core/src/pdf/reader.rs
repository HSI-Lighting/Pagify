//! Finding the objects in a file, and writing one back changed.

use super::object::{Dict, Lexer, Object, Offsets};
use crate::error::{PdfError, Result};

/// A PDF file, read only as far as it takes to find its objects.
///
/// `Debug` shows the shape rather than the bytes: a file is megabytes, and a
/// failing assertion that prints one is unreadable.
pub struct File<'a> {
    bytes: &'a [u8],
    offsets: Offsets,
    trailer: Dict,
}

impl std::fmt::Debug for File<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("File")
            .field("bytes", &self.bytes.len())
            .field("objects", &self.offsets.len())
            .finish()
    }
}

impl<'a> File<'a> {
    /// Read the cross-reference chain.
    ///
    /// Refuses a file it cannot fully account for rather than reading part of
    /// it — see the module note. A caller that gets an error here has a file
    /// this cannot safely rewrite, and should leave it alone.
    pub fn parse(bytes: &'a [u8]) -> Result<File<'a>> {
        let start = Self::start_xref(bytes)?;
        let mut offsets = Offsets::new();
        let mut trailer: Option<Dict> = None;
        let mut at = Some(start);
        let mut seen = Vec::new();

        while let Some(section) = at {
            // A `/Prev` pointing back into a section already read is a loop, and
            // a file that walks one forever is a file that never opens.
            if seen.contains(&section) {
                return Err(PdfError::InvalidArgument(
                    "the cross-reference table points at itself".into(),
                ));
            }
            seen.push(section);
            if section >= bytes.len() {
                return Err(PdfError::InvalidArgument(
                    "the cross-reference table is past the end of the file".into(),
                ));
            }

            let (section_offsets, section_trailer) = Self::xref_section(bytes, section)?;
            // Earlier sections must not overwrite later ones: the newest
            // revision of an object is the one the latest table names, and the
            // chain is walked newest first.
            for (number, offset) in section_offsets {
                offsets.entry(number).or_insert(offset);
            }

            at = section_trailer
                .get(b"Prev")
                .and_then(Object::as_i64)
                .and_then(|n| usize::try_from(n).ok());
            if trailer.is_none() {
                trailer = Some(section_trailer);
            }
        }

        Ok(File {
            bytes,
            offsets,
            trailer: trailer.unwrap_or_default(),
        })
    }

    fn start_xref(bytes: &[u8]) -> Result<usize> {
        // Searched from the end, because that is where the last revision's
        // pointer is and a file may carry several.
        let tail_from = bytes.len().saturating_sub(2048);
        let tail = &bytes[tail_from..];
        let found = tail
            .windows(9)
            .rposition(|w| w == b"startxref")
            .ok_or_else(|| PdfError::InvalidArgument("no startxref".into()))?;
        let mut lexer = Lexer::new(bytes, tail_from + found + 9);
        let token = lexer.token();
        std::str::from_utf8(token)
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .ok_or_else(|| PdfError::InvalidArgument("startxref is not a number".into()))
    }

    /// One `xref` section and the trailer after it.
    fn xref_section(bytes: &[u8], at: usize) -> Result<(Offsets, Dict)> {
        let mut lexer = Lexer::new(bytes, at);
        lexer.skip_space();
        if !bytes[lexer.at..].starts_with(b"xref") {
            // A cross-reference *stream* — legal, and not what PDFium writes.
            // Refused whole rather than half-read.
            return Err(PdfError::Unsupported(
                "a cross-reference stream, which this build does not write",
            ));
        }
        lexer.at += b"xref".len();

        let mut offsets = Offsets::new();
        loop {
            lexer.skip_space();
            if bytes[lexer.at..].starts_with(b"trailer") {
                lexer.at += b"trailer".len();
                let trailer = match lexer.object()? {
                    Object::Dict(d) => d,
                    _ => return Err(PdfError::InvalidArgument("the trailer is not a dict".into())),
                };
                return Ok((offsets, trailer));
            }

            // A subsection header: first object number, then how many.
            let first: u32 = std::str::from_utf8(lexer.token())
                .ok()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| PdfError::InvalidArgument("a bad xref subsection".into()))?;
            let count: u32 = std::str::from_utf8(lexer.token())
                .ok()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| PdfError::InvalidArgument("a bad xref subsection".into()))?;

            for index in 0..count {
                let offset = std::str::from_utf8(lexer.token())
                    .ok()
                    .and_then(|s| s.parse::<usize>().ok())
                    .ok_or_else(|| PdfError::InvalidArgument("a bad xref entry".into()))?;
                let _generation = lexer.token();
                let kind = lexer.token();
                // `f` is a free entry — a number nothing uses. Recording one
                // would point a lookup at whatever happens to be at offset 0.
                //
                // **An in-use entry at offset zero is a free entry too.** The
                // header lives at zero, so no object can begin there; a writer
                // that marks one `n` has produced a table it cannot mean.
                // Measured on a real document — one rebuilt by CoreGraphics
                // wrote object 9 that way — where taking it at its word made
                // the whole file unreadable to us over a single bad row.
                if kind == b"n" && offset != 0 {
                    offsets.insert(first + index, offset);
                }
            }
        }
    }

    pub fn trailer(&self) -> &Dict {
        &self.trailer
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Every object number the table names, in order.
    pub fn numbers(&self) -> impl Iterator<Item = u32> + '_ {
        self.offsets.keys().copied()
    }

    /// One object, parsed where the table says it is.
    pub fn object(&self, number: u32) -> Result<Object> {
        let at = *self
            .offsets
            .get(&number)
            .ok_or_else(|| PdfError::InvalidArgument(format!("no object {number}")))?;
        if at >= self.bytes.len() {
            return Err(PdfError::InvalidArgument(format!(
                "object {number} is past the end of the file"
            )));
        }
        let mut lexer = Lexer::new(self.bytes, at);
        // `n g obj` — the header is checked rather than skipped, because an
        // offset that has drifted lands mid-object and would otherwise be read
        // as whatever it happened to hit.
        let found: u32 = std::str::from_utf8(lexer.token())
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| {
                PdfError::InvalidArgument(format!("object {number} has no header where it should"))
            })?;
        if found != number {
            return Err(PdfError::InvalidArgument(format!(
                "the table says object {number} is where object {found} is"
            )));
        }
        let _generation = lexer.token();
        lexer.expect(b"obj")?;
        lexer.object()
    }

    /// Follow a reference, or return the object as it stands.
    pub fn resolve(&self, object: &Object) -> Result<Object> {
        match object {
            Object::Reference(number, _) => self.object(*number),
            other => Ok(other.clone()),
        }
    }

    /// Write the file out with some objects replaced.
    ///
    /// **Every other object is copied through byte for byte**, from its own
    /// `n g obj` to its `endobj`, rather than being parsed and printed again.
    /// That is the whole point: the content streams that draw the text are
    /// never read, so they cannot come back different — which is exactly what
    /// `FPDFPage_GenerateContent` could not promise.
    ///
    /// A whole file rather than an incremental update. An update appends and
    /// leaves the earlier revision intact, so an image somebody had just locked
    /// would still be sitting in the bytes for anyone who read the older
    /// cross-reference table — the same reason a locked document already
    /// refuses to save incrementally.
    pub fn rewrite(&self, replacements: &[(u32, Vec<u8>)]) -> Result<Vec<u8>> {
        self.rewrite_adding(replacements, &[], &Dict(Vec::new()))
    }

    /// The general form: replace objects, add new ones, and change the trailer.
    ///
    /// Securing a document needs all three — every object is rewritten with its
    /// strings and streams encrypted, the `/Encrypt` dictionary is a new object,
    /// and the trailer has to point at it.
    pub fn rewrite_adding(
        &self,
        replacements: &[(u32, Vec<u8>)],
        extra: &[(u32, Vec<u8>)],
        trailer_edits: &Dict,
    ) -> Result<Vec<u8>> {
        self.rewrite_dropping(replacements, extra, trailer_edits, &[])
    }

    /// The fullest form: also leave some objects out altogether.
    ///
    /// Sanitising needs that — an object nothing reaches is content taken off
    /// the pages but never taken out of the file, and copying it through would
    /// defeat the point. A dropped number is written as a **free** entry in the
    /// cross-reference table, so nothing can follow it to an offset that no
    /// longer means anything.
    pub fn rewrite_dropping(
        &self,
        replacements: &[(u32, Vec<u8>)],
        extra: &[(u32, Vec<u8>)],
        trailer_edits: &Dict,
        drop: &[u32],
    ) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(self.bytes.len());

        // The header, and the binary comment after it that marks the file as
        // not-text. Copied from the original so a reader that sniffed one goes
        // on seeing it.
        let mut header_end = self.bytes.iter().position(|b| *b == b'\n').map_or(0, |n| n + 1);
        if self.bytes.get(header_end) == Some(&b'%') {
            header_end = self.bytes[header_end..]
                .iter()
                .position(|b| *b == b'\n')
                .map_or(header_end, |n| header_end + n + 1);
        }
        out.extend_from_slice(&self.bytes[..header_end]);

        let mut written: Offsets = Offsets::new();
        for number in self.numbers().collect::<Vec<_>>() {
            if drop.contains(&number) {
                continue;
            }
            written.insert(number, out.len());
            match replacements.iter().find(|(n, _)| *n == number) {
                Some((_, body)) => {
                    out.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
                    out.extend_from_slice(body);
                    out.extend_from_slice(b"\nendobj\n");
                }
                None => {
                    let span = self.span_of(number)?;
                    out.extend_from_slice(&self.bytes[span]);
                    if !out.ends_with(b"\n") {
                        out.push(b'\n');
                    }
                }
            }
        }

        for (number, body) in extra {
            written.insert(*number, out.len());
            out.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }

        // One subsection covering everything, which is legal and simpler than
        // reproducing however many the original happened to have.
        let highest = written.keys().copied().max().unwrap_or(0);
        let xref_at = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", highest + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for number in 1..=highest {
            match written.get(&number) {
                Some(offset) => {
                    out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes())
                }
                // A number the original never used stays free, so nothing can
                // follow it to an offset that means nothing.
                None => out.extend_from_slice(b"0000000000 65535 f \n"),
            }
        }

        let mut trailer = self.trailer.clone();
        trailer.set(b"Size", Object::Number(format!("{}", highest + 1).into_bytes()));
        // `/Prev` named a table in the file this was read from, and there is
        // only one table now. Left in, it would send a reader to an offset that
        // means nothing here.
        trailer.remove(b"Prev");
        trailer.remove(b"XRefStm");
        for (key, value) in &trailer_edits.0 {
            // `Null` says to take the key out rather than to write it as null,
            // which is how sanitising drops `/Info` without leaving a stub that
            // still says a tool has been over the file.
            match value {
                Object::Null => trailer.remove(key),
                other => trailer.set(key, other.clone()),
            }
        }

        out.extend_from_slice(b"trailer\n");
        write_object(&mut out, &Object::Dict(trailer));
        out.extend_from_slice(format!("\nstartxref\n{xref_at}\n%%EOF").as_bytes());
        Ok(out)
    }

    /// Where one object's own bytes begin and end — `n g obj` to `endobj`.
    fn span_of(&self, number: u32) -> Result<std::ops::Range<usize>> {
        let at = *self
            .offsets
            .get(&number)
            .ok_or_else(|| PdfError::InvalidArgument(format!("no object {number}")))?;
        let mut lexer = Lexer::new(self.bytes, at);
        let _number = lexer.token();
        let _generation = lexer.token();
        lexer.expect(b"obj")?;
        // Parsing is how the end is found: a stream's length is in its own
        // dictionary, and scanning for `endobj` finds whatever the binary data
        // happens to contain.
        let _object = lexer.object()?;
        lexer.skip_space();
        if self.bytes[lexer.at..].starts_with(b"endobj") {
            lexer.at += b"endobj".len();
        }
        Ok(at..lexer.at)
    }
}

/// Write an object out.
///
/// Only ever used for objects this module *makes* — a replacement, or the
/// trailer. Everything read from a file is copied rather than printed, so this
/// never has to reproduce somebody else's formatting exactly.
pub fn write_object(out: &mut Vec<u8>, object: &Object) {
    match object {
        Object::Null => out.extend_from_slice(b"null"),
        Object::Bool(true) => out.extend_from_slice(b"true"),
        Object::Bool(false) => out.extend_from_slice(b"false"),
        Object::Number(raw) => out.extend_from_slice(raw),
        Object::Name(name) => {
            out.push(b'/');
            out.extend_from_slice(name);
        }
        Object::LiteralString(raw) => {
            out.push(b'(');
            out.extend_from_slice(raw);
            out.push(b')');
        }
        Object::HexString(raw) => {
            out.push(b'<');
            out.extend_from_slice(raw);
            out.push(b'>');
        }
        Object::Reference(number, generation) => {
            out.extend_from_slice(format!("{number} {generation} R").as_bytes())
        }
        Object::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(b' ');
                }
                write_object(out, item);
            }
            out.push(b']');
        }
        Object::Dict(dict) | Object::Stream(dict, _) => {
            out.extend_from_slice(b"<<");
            for (key, value) in &dict.0 {
                out.push(b'/');
                out.extend_from_slice(key);
                out.push(b' ');
                write_object(out, value);
            }
            out.extend_from_slice(b">>");
        }
    }
}

/// A stream object's bytes, ready for [`File::rewrite`].
///
/// `/Length` is taken from the data rather than trusted from the dictionary:
/// the caller is handing over new data, and the old length would make the file
/// unreadable from that point on.
pub fn write_stream(dict: &Dict, data: &[u8]) -> Vec<u8> {
    let mut dict = dict.clone();
    dict.set(b"Length", Object::Number(format!("{}", data.len()).into_bytes()));

    let mut out = Vec::with_capacity(data.len() + 128);
    write_object(&mut out, &Object::Dict(dict));
    out.extend_from_slice(b"\nstream\n");
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A whole small file, written by hand so the offsets are known.
    fn a_file() -> Vec<u8> {
        let mut out = Vec::new();
        let mut offsets = Vec::new();

        out.extend_from_slice(b"%PDF-1.7\n");

        offsets.push(out.len());
        out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

        offsets.push(out.len());
        out.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");

        offsets.push(out.len());
        out.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /Resources << /XObject << /Im0 4 0 R >> >> >>\nendobj\n",
        );

        offsets.push(out.len());
        out.extend_from_slice(
            b"4 0 obj\n<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /Length 6 >>\nstream\nABCDEF\nendstream\nendobj\n",
        );

        let xref_at = out.len();
        out.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for offset in &offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n");
        out.extend_from_slice(format!("{xref_at}\n%%EOF").as_bytes());
        out
    }

    #[test]
    fn every_object_is_found_where_the_table_says() {
        let bytes = a_file();
        let file = File::parse(&bytes).expect("parse");
        assert_eq!(file.numbers().collect::<Vec<_>>(), vec![1, 2, 3, 4]);

        let catalogue = file.object(1).expect("object 1");
        assert_eq!(
            catalogue.as_dict().and_then(|d| d.get(b"Type")).and_then(Object::as_name),
            Some(&b"Catalog"[..])
        );
        assert_eq!(
            file.trailer().get(b"Root").and_then(Object::as_reference),
            Some((1, 0))
        );
    }

    /// **An in-use entry at offset zero is not an object either.** The header
    /// is at zero, so nothing can start there — and a real document, rebuilt by
    /// another program, had exactly one such row. Refusing the file over it
    /// would be strict about a byte nobody meant.
    #[test]
    fn an_in_use_entry_at_offset_zero_is_treated_as_free() {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.7\n");
        let one = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<</Type/Catalog>>\nendobj\n");
        let start = pdf.len();
        pdf.extend_from_slice(b"xref\n0 3\n0000000000 65535 f \n");
        pdf.extend_from_slice(format!("{one:010} 00000 n \n").as_bytes());
        // Object 2: in use, at nowhere.
        pdf.extend_from_slice(b"0000000000 00000 n \n");
        pdf.extend_from_slice(
            format!("trailer\n<</Size 3/Root 1 0 R>>\nstartxref\n{start}\n%%EOF").as_bytes(),
        );

        let file = File::parse(&pdf).expect("a file with one bad row is still readable");
        assert!(file.object(1).is_ok(), "the good object was lost with the bad one");
        assert!(
            !file.numbers().any(|n| n == 2),
            "the object at nowhere was recorded as being somewhere"
        );
    }

    /// **A free entry is not an object.** Recording one would point a lookup at
    /// offset 0, which is the file header.
    #[test]
    fn the_free_entry_is_not_recorded() {
        let bytes = a_file();
        let file = File::parse(&bytes).expect("parse");
        assert!(file.object(0).is_err(), "object 0 was read as though it were real");
    }

    #[test]
    fn a_stream_comes_back_with_its_bytes() {
        let bytes = a_file();
        let file = File::parse(&bytes).expect("parse");
        match file.object(4).expect("object 4") {
            Object::Stream(dict, range) => {
                assert_eq!(dict.get(b"Subtype").and_then(Object::as_name), Some(&b"Image"[..]));
                assert_eq!(&bytes[range], b"ABCDEF");
            }
            other => panic!("expected a stream, got {other:?}"),
        }
    }

    /// The path Lock needs: page → resources → the image's object number.
    #[test]
    fn an_images_object_number_can_be_reached_from_its_page() {
        let bytes = a_file();
        let file = File::parse(&bytes).expect("parse");
        let page = file.object(3).expect("the page");
        let xobjects = page
            .as_dict()
            .and_then(|d| d.get(b"Resources"))
            .and_then(Object::as_dict)
            .and_then(|d| d.get(b"XObject"))
            .and_then(Object::as_dict)
            .expect("an xobject dictionary");
        assert_eq!(xobjects.get(b"Im0").and_then(Object::as_reference), Some((4, 0)));
    }

    /// **An offset that has drifted is caught, not followed.** Reading whatever
    /// sits at a stale offset is how a rewrite corrupts a file quietly.
    #[test]
    fn an_offset_pointing_at_the_wrong_object_is_refused() {
        let mut bytes = a_file();
        // Point object 1 at object 2's header.
        let two_at = bytes
            .windows(7)
            .position(|w| w == b"2 0 obj")
            .expect("object 2");
        // `position`, not `rposition`: the last "xref" in a file is the one
        // inside "startxref".
        let table = bytes.windows(5).position(|w| w == b"xref\n").expect("xref");
        let entry = table + b"xref\n0 5\n0000000000 65535 f \n".len();
        bytes.splice(entry..entry + 10, format!("{two_at:010}").bytes());

        let file = File::parse(&bytes).expect("parse");
        let problem = file.object(1).expect_err("should refuse");
        assert!(format!("{problem}").contains("object 2 is"), "{problem}");
    }

    /// A cross-reference stream is refused whole. PDFium does not write one —
    /// see `examples/save_shape_probe.rs` — and half-reading a file this cannot
    /// account for is how a rewrite loses content.
    #[test]
    fn a_file_this_cannot_read_is_refused_whole() {
        let bytes = b"%PDF-1.5\n5 0 obj\n<< /Type /XRef /Length 0 >>\nstream\nendstream\nendobj\nstartxref\n9\n%%EOF";
        let problem = File::parse(bytes).expect_err("should refuse");
        assert!(
            format!("{problem}").contains("cross-reference stream"),
            "it did not say what it could not read: {problem}"
        );
    }

    #[test]
    fn a_file_with_no_startxref_is_refused() {
        assert!(File::parse(b"%PDF-1.7\nnothing here").is_err());
    }

    // -- writing -----------------------------------------------------------

    /// **The guarantee this module exists for.** Rewriting one object leaves
    /// every other object's bytes exactly as they were — including the content
    /// streams that draw the text, which is what `FPDFPage_GenerateContent`
    /// could not promise.
    #[test]
    fn rewriting_one_object_leaves_every_other_byte_alone() {
        let bytes = a_file();
        let file = File::parse(&bytes).expect("parse");

        // The page's own bytes, before.
        let page_before = {
            let mut out = Vec::new();
            write_object(&mut out, &file.object(3).expect("page"));
            out
        };

        let blank = write_stream(
            &Dict(vec![
                (b"Type".to_vec(), Object::Name(b"XObject".to_vec())),
                (b"Subtype".to_vec(), Object::Name(b"Image".to_vec())),
                (b"Width".to_vec(), Object::Number(b"1".to_vec())),
                (b"Height".to_vec(), Object::Number(b"1".to_vec())),
            ]),
            b"\x00",
        );
        let rewritten = file.rewrite(&[(4, blank)]).expect("rewrite");

        let after = File::parse(&rewritten).expect("the rewritten file must parse");
        let page_after = {
            let mut out = Vec::new();
            write_object(&mut out, &after.object(3).expect("page"));
            out
        };
        assert_eq!(page_after, page_before, "an untouched object came back different");

        // And the one asked for really did change.
        match after.object(4).expect("object 4") {
            Object::Stream(dict, range) => {
                assert_eq!(dict.get(b"Width").and_then(Object::as_i64), Some(1));
                assert_eq!(&rewritten[range], b"\x00");
            }
            other => panic!("expected a stream, got {other:?}"),
        }
    }

    /// Every object still has to be findable afterwards — a table whose offsets
    /// drifted by one byte is a file no reader can open.
    #[test]
    fn every_object_is_still_where_the_new_table_says() {
        let bytes = a_file();
        let file = File::parse(&bytes).expect("parse");
        let rewritten = file
            .rewrite(&[(1, b"<< /Type /Catalog /Pages 2 0 R /Marked true >>".to_vec())])
            .expect("rewrite");

        let after = File::parse(&rewritten).expect("parse");
        assert_eq!(after.numbers().collect::<Vec<_>>(), vec![1, 2, 3, 4]);
        for number in [1u32, 2, 3, 4] {
            after.object(number).unwrap_or_else(|e| panic!("object {number}: {e}"));
        }
        assert_eq!(
            after.trailer().get(b"Size").and_then(Object::as_i64),
            Some(5),
            "the trailer still claims the old size"
        );
    }

    /// A file with nothing replaced must survive the round trip, or the writer
    /// is damaging files it was only asked to copy.
    #[test]
    fn a_file_rewritten_with_no_changes_still_reads_the_same() {
        let bytes = a_file();
        let file = File::parse(&bytes).expect("parse");
        let rewritten = file.rewrite(&[]).expect("rewrite");

        let after = File::parse(&rewritten).expect("parse");
        for number in file.numbers() {
            let mut before = Vec::new();
            write_object(&mut before, &file.object(number).expect("before"));
            let mut now = Vec::new();
            write_object(&mut now, &after.object(number).expect("after"));
            assert_eq!(now, before, "object {number} changed in a no-op rewrite");
        }
    }

    /// The header, and the binary marker after it, have to survive — a reader
    /// that sniffed the file as binary must go on doing so.
    #[test]
    fn the_header_and_its_binary_marker_are_kept() {
        // The four high bytes PDF writers put on the second line, spelt as
        // numbers so no editor or script can helpfully re-encode them.
        let marker = [b'%', 0xE2u8, 0xE3, 0xCF, 0xD3, b'\n'];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        bytes.extend_from_slice(&marker);
        let body_at = bytes.len();
        bytes.extend_from_slice(b"1 0 obj\n<< /Type /Catalog >>\nendobj\n");
        let xref_at = bytes.len();
        bytes.extend_from_slice(b"xref\n0 2\n0000000000 65535 f \n");
        bytes.extend_from_slice(format!("{body_at:010} 00000 n \n").as_bytes());
        bytes.extend_from_slice(b"trailer\n<< /Size 2 /Root 1 0 R >>\nstartxref\n");
        bytes.extend_from_slice(format!("{xref_at}\n%%EOF").as_bytes());

        let file = File::parse(&bytes).expect("parse");
        let rewritten = file.rewrite(&[]).expect("rewrite");
        assert!(rewritten.starts_with(b"%PDF-1.7\n"), "the version line changed");
        assert_eq!(
            &rewritten[9..15],
            &marker,
            "the binary marker was dropped, so the file now sniffs as text"
        );
    }

    /// A `/Prev` chain that loops must stop rather than spin.
    #[test]
    fn a_looping_cross_reference_chain_is_refused() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"%PDF-1.7\n");
        let xref_at = bytes.len();
        bytes.extend_from_slice(b"xref\n0 1\n0000000000 65535 f \ntrailer\n<< /Size 1 /Prev ");
        bytes.extend_from_slice(format!("{xref_at} >>\nstartxref\n{xref_at}\n%%EOF").as_bytes());

        let problem = File::parse(&bytes).expect_err("should refuse");
        assert!(format!("{problem}").contains("itself"), "{problem}");
    }
}
