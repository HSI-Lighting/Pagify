//! PDF objects, and the reader that turns bytes into them.
//!
//! Enough of ISO 32000-1 §7.3 to read what PDFium writes and write it back
//! unchanged. Strings and numbers keep their **original bytes** rather than
//! being re-formatted on the way out: a `Real` written back as `0.10000000149`
//! because it went through an `f32` is a file that differs from the one it came
//! from for no reason, and the whole point here is to differ only where asked.

use std::collections::BTreeMap;

use crate::error::{PdfError, Result};

/// A PDF object.
#[derive(Debug, Clone, PartialEq)]
pub enum Object {
    Null,
    Bool(bool),
    /// Kept as written — see the module note on why numbers are not re-formatted.
    Number(Vec<u8>),
    /// The bytes between the delimiters, still escaped as the file had them.
    LiteralString(Vec<u8>),
    HexString(Vec<u8>),
    Name(Vec<u8>),
    Array(Vec<Object>),
    Dict(Dict),
    /// A dictionary and where its stream data sits in the file.
    Stream(Dict, std::ops::Range<usize>),
    /// `12 0 R`.
    Reference(u32, u16),
}

impl Object {
    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Object::Dict(d) | Object::Stream(d, _) => Some(d),
            _ => None,
        }
    }

    pub fn as_reference(&self) -> Option<(u32, u16)> {
        match self {
            Object::Reference(n, g) => Some((*n, *g)),
            _ => None,
        }
    }

    pub fn as_name(&self) -> Option<&[u8]> {
        match self {
            Object::Name(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Object::Number(raw) => std::str::from_utf8(raw).ok()?.parse().ok(),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Object::Number(raw) => std::str::from_utf8(raw).ok()?.parse().ok(),
            _ => None,
        }
    }
}

/// A PDF dictionary, in the order the file had it.
///
/// Ordered rather than a `HashMap` so that writing one back produces the same
/// bytes it was read from. A `BTreeMap` would sort the keys and change every
/// dictionary it touched.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Dict(pub Vec<(Vec<u8>, Object)>);

impl Dict {
    pub fn get(&self, key: &[u8]) -> Option<&Object> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn set(&mut self, key: &[u8], value: Object) {
        match self.0.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = value,
            None => self.0.push((key.to_vec(), value)),
        }
    }

    pub fn remove(&mut self, key: &[u8]) {
        self.0.retain(|(k, _)| k != key);
    }
}

/// Where the objects in a file are, by object number.
pub type Offsets = BTreeMap<u32, usize>;

/// A reader over one file's bytes.
pub struct Lexer<'a> {
    pub bytes: &'a [u8],
    pub at: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(bytes: &'a [u8], at: usize) -> Self {
        Lexer { bytes, at }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    /// PDF whitespace, and comments, which may appear anywhere a token may.
    pub fn skip_space(&mut self) {
        loop {
            match self.peek() {
                Some(b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ') => self.at += 1,
                Some(b'%') => {
                    while !matches!(self.peek(), None | Some(b'\n' | b'\r')) {
                        self.at += 1;
                    }
                }
                _ => return,
            }
        }
    }

    fn is_delimiter(b: u8) -> bool {
        matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
    }

    fn is_regular(b: u8) -> bool {
        !Self::is_delimiter(b) && !matches!(b, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
    }

    /// The next run of regular characters — a keyword, or a number.
    pub fn token(&mut self) -> &'a [u8] {
        self.skip_space();
        let start = self.at;
        while self.peek().is_some_and(Self::is_regular) {
            self.at += 1;
        }
        &self.bytes[start..self.at]
    }

    pub fn expect(&mut self, keyword: &[u8]) -> Result<()> {
        let found = self.token();
        if found == keyword {
            Ok(())
        } else {
            Err(PdfError::InvalidArgument(format!(
                "expected {:?} at {}, found {:?}",
                String::from_utf8_lossy(keyword),
                self.at,
                String::from_utf8_lossy(found)
            )))
        }
    }

    /// One object.
    ///
    /// `n n R` is only a reference when all three parts are there, so two
    /// numbers are read ahead and put back if the third is not an `R` — the
    /// alternative is an array of numbers being read as references.
    pub fn object(&mut self) -> Result<Object> {
        self.skip_space();
        match self.peek() {
            None => Err(PdfError::InvalidArgument("the file ends mid-object".into())),
            Some(b'/') => {
                self.at += 1;
                let start = self.at;
                while self.peek().is_some_and(Self::is_regular) {
                    self.at += 1;
                }
                Ok(Object::Name(self.bytes[start..self.at].to_vec()))
            }
            Some(b'(') => self.literal_string(),
            Some(b'[') => {
                self.at += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_space();
                    match self.peek() {
                        Some(b']') => {
                            self.at += 1;
                            return Ok(Object::Array(items));
                        }
                        None => {
                            return Err(PdfError::InvalidArgument("an array never closes".into()))
                        }
                        _ => items.push(self.object()?),
                    }
                }
            }
            Some(b'<') if self.bytes.get(self.at + 1) == Some(&b'<') => self.dict_or_stream(),
            Some(b'<') => self.hex_string(),
            Some(b'0'..=b'9' | b'+' | b'-' | b'.') => self.number_or_reference(),
            _ => {
                let word = self.token();
                match word {
                    b"true" => Ok(Object::Bool(true)),
                    b"false" => Ok(Object::Bool(false)),
                    b"null" => Ok(Object::Null),
                    other => Err(PdfError::InvalidArgument(format!(
                        "not an object: {:?}",
                        String::from_utf8_lossy(other)
                    ))),
                }
            }
        }
    }

    fn number_or_reference(&mut self) -> Result<Object> {
        let before = self.at;
        let first = self.token().to_vec();

        // Only a whole, non-negative number can begin a reference.
        if first.iter().all(|b| b.is_ascii_digit()) && !first.is_empty() {
            let save = self.at;
            let second = self.token().to_vec();
            if second.iter().all(|b| b.is_ascii_digit()) && !second.is_empty() {
                let after_second = self.at;
                if self.token() == b"R" {
                    let number = std::str::from_utf8(&first).ok().and_then(|s| s.parse().ok());
                    let generation = std::str::from_utf8(&second).ok().and_then(|s| s.parse().ok());
                    if let (Some(number), Some(generation)) = (number, generation) {
                        return Ok(Object::Reference(number, generation));
                    }
                }
                self.at = after_second;
            }
            self.at = save;
        }
        let _ = before;
        Ok(Object::Number(first))
    }

    fn literal_string(&mut self) -> Result<Object> {
        self.at += 1;
        let start = self.at;
        let mut depth = 1usize;
        while let Some(b) = self.peek() {
            match b {
                b'\\' => self.at += 2,
                b'(' => {
                    depth += 1;
                    self.at += 1;
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        let raw = self.bytes[start..self.at].to_vec();
                        self.at += 1;
                        return Ok(Object::LiteralString(raw));
                    }
                    self.at += 1;
                }
                _ => self.at += 1,
            }
        }
        Err(PdfError::InvalidArgument("a string never closes".into()))
    }

    fn hex_string(&mut self) -> Result<Object> {
        self.at += 1;
        let start = self.at;
        while self.peek().is_some_and(|b| b != b'>') {
            self.at += 1;
        }
        if self.peek().is_none() {
            return Err(PdfError::InvalidArgument("a hex string never closes".into()));
        }
        let raw = self.bytes[start..self.at].to_vec();
        self.at += 1;
        Ok(Object::HexString(raw))
    }

    fn dict_or_stream(&mut self) -> Result<Object> {
        self.at += 2;
        let mut dict = Dict::default();
        loop {
            self.skip_space();
            match self.peek() {
                Some(b'>') if self.bytes.get(self.at + 1) == Some(&b'>') => {
                    self.at += 2;
                    break;
                }
                Some(b'/') => {
                    let key = match self.object()? {
                        Object::Name(name) => name,
                        _ => unreachable!("a `/` always reads as a name"),
                    };
                    let value = self.object()?;
                    dict.0.push((key, value));
                }
                _ => {
                    return Err(PdfError::InvalidArgument(
                        "a dictionary key that is not a name".into(),
                    ))
                }
            }
        }

        // A stream, if `stream` follows the dictionary.
        let save = self.at;
        self.skip_space();
        if self.bytes[self.at..].starts_with(b"stream") {
            self.at += b"stream".len();
            // The keyword is followed by CRLF or LF — never CR alone.
            if self.bytes.get(self.at) == Some(&b'\r') {
                self.at += 1;
            }
            if self.bytes.get(self.at) == Some(&b'\n') {
                self.at += 1;
            }
            let start = self.at;
            let length = dict
                .get(b"Length")
                .and_then(Object::as_i64)
                .and_then(|n| usize::try_from(n).ok());
            // An indirect `/Length` is legal and PDFium does not write one, so
            // it is refused rather than guessed at by scanning for `endstream`
            // — a scan finds the first match, which inside binary image data is
            // as likely to be a coincidence as the real end.
            let Some(length) = length else {
                return Err(PdfError::Unsupported(
                    "a stream whose length is an indirect reference",
                ));
            };
            let end = start.checked_add(length).filter(|e| *e <= self.bytes.len()).ok_or_else(
                || PdfError::InvalidArgument("a stream runs past the end of the file".into()),
            )?;
            self.at = end;
            self.skip_space();
            if self.bytes[self.at..].starts_with(b"endstream") {
                self.at += b"endstream".len();
            }
            return Ok(Object::Stream(dict, start..end));
        }
        self.at = save;
        Ok(Object::Dict(dict))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(bytes: &[u8]) -> Object {
        Lexer::new(bytes, 0).object().expect("parse")
    }

    #[test]
    fn the_simple_objects_read_back() {
        assert_eq!(read(b"true"), Object::Bool(true));
        assert_eq!(read(b"false"), Object::Bool(false));
        assert_eq!(read(b"null"), Object::Null);
        assert_eq!(read(b"/Type"), Object::Name(b"Type".to_vec()));
    }

    /// **Numbers keep their own bytes.** A file re-written through `f32` differs
    /// from the one it came from everywhere a number appears, which is exactly
    /// what this module exists to avoid.
    #[test]
    fn a_number_keeps_the_text_it_was_written_as() {
        assert_eq!(read(b"0.10000000149"), Object::Number(b"0.10000000149".to_vec()));
        assert_eq!(read(b"-42"), Object::Number(b"-42".to_vec()));
        assert_eq!(read(b"842.0"), Object::Number(b"842.0".to_vec()));
        assert_eq!(read(b"42").as_i64(), Some(42));
    }

    #[test]
    fn a_reference_needs_all_three_parts() {
        assert_eq!(read(b"12 0 R"), Object::Reference(12, 0));

        // Two numbers in an array are two numbers, not a reference — the case
        // that makes the look-ahead necessary.
        assert_eq!(
            read(b"[1 2]"),
            Object::Array(vec![Object::Number(b"1".to_vec()), Object::Number(b"2".to_vec())])
        );
        // And a trailing pair with no `R` is still two numbers.
        assert_eq!(
            read(b"[4 0]"),
            Object::Array(vec![Object::Number(b"4".to_vec()), Object::Number(b"0".to_vec())])
        );
        assert_eq!(
            read(b"[4 0 R 5]"),
            Object::Array(vec![Object::Reference(4, 0), Object::Number(b"5".to_vec())])
        );
    }

    #[test]
    fn a_dictionary_keeps_its_order() {
        let object = read(b"<< /Type /Page /Parent 4 0 R /MediaBox [0 0 595 842] >>");
        let dict = object.as_dict().expect("a dict");
        let keys: Vec<&[u8]> = dict.0.iter().map(|(k, _)| k.as_slice()).collect();
        assert_eq!(keys, vec![&b"Type"[..], b"Parent", b"MediaBox"]);
        assert_eq!(dict.get(b"Parent").and_then(Object::as_reference), Some((4, 0)));
    }

    #[test]
    fn a_nested_dictionary_reads() {
        let object = read(b"<< /Resources << /XObject << /Im1 7 0 R >> >> >>");
        let inner = object
            .as_dict()
            .and_then(|d| d.get(b"Resources"))
            .and_then(Object::as_dict)
            .and_then(|d| d.get(b"XObject"))
            .and_then(Object::as_dict)
            .expect("nested");
        assert_eq!(inner.get(b"Im1").and_then(Object::as_reference), Some((7, 0)));
    }

    /// Strings are kept as written, escapes and all, because they are copied
    /// back rather than interpreted.
    #[test]
    fn a_string_keeps_its_escapes_and_its_nesting() {
        assert_eq!(read(br"(a \(nested\) one)"), Object::LiteralString(br"a \(nested\) one".to_vec()));
        assert_eq!(read(b"(outer (inner) end)"), Object::LiteralString(b"outer (inner) end".to_vec()));
        assert_eq!(read(b"<48656C6C6F>"), Object::HexString(b"48656C6C6F".to_vec()));
    }

    #[test]
    fn a_stream_is_found_by_its_declared_length() {
        let bytes = b"<< /Length 5 >>\nstream\nHELLO\nendstream";
        match read(bytes) {
            Object::Stream(dict, range) => {
                assert_eq!(dict.get(b"Length").and_then(Object::as_i64), Some(5));
                assert_eq!(&bytes[range], b"HELLO");
            }
            other => panic!("expected a stream, got {other:?}"),
        }
    }

    /// **Binary data is not scanned for `endstream`.** A JPEG contains the
    /// bytes of anything, so the declared length is the only trustworthy end —
    /// and a stream without one is refused rather than guessed at.
    #[test]
    fn a_stream_whose_length_is_indirect_is_refused_not_guessed() {
        let bytes = b"<< /Length 9 0 R >>\nstream\n....endstream\nendstream";
        assert!(Lexer::new(bytes, 0).object().is_err());
    }

    #[test]
    fn a_stream_of_binary_containing_the_word_endstream_still_reads() {
        let mut bytes = b"<< /Length 20 >>\nstream\n".to_vec();
        bytes.extend_from_slice(b"xxxxendstream xxxxxx");
        bytes.extend_from_slice(b"\nendstream");
        match read(&bytes) {
            Object::Stream(_, range) => assert_eq!(range.len(), 20),
            other => panic!("expected a stream, got {other:?}"),
        }
    }

    #[test]
    fn comments_are_skipped_wherever_they_appear() {
        assert_eq!(read(b"% a comment\n/Type"), Object::Name(b"Type".to_vec()));
        let object = read(b"<< /A 1 % why\n /B 2 >>");
        assert_eq!(object.as_dict().expect("dict").0.len(), 2);
    }

    #[test]
    fn a_dictionary_can_be_edited_without_disturbing_the_rest() {
        let mut dict = match read(b"<< /Type /XObject /Width 100 /Height 70 >>") {
            Object::Dict(d) => d,
            other => panic!("{other:?}"),
        };
        dict.set(b"Width", Object::Number(b"1".to_vec()));
        dict.remove(b"Height");
        dict.set(b"Extra", Object::Bool(true));

        let keys: Vec<&[u8]> = dict.0.iter().map(|(k, _)| k.as_slice()).collect();
        assert_eq!(keys, vec![&b"Type"[..], b"Width", b"Extra"], "editing reordered the rest");
        assert_eq!(dict.get(b"Width").and_then(Object::as_i64), Some(1));
    }
}
