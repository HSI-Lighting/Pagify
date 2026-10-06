//! Why a file that would not open would not — in words a person can act on.
//!
//! **Reported from use, twice:** the only thing said was
//! `pdfium error: PdfiumLibraryInternalError(Unknown)`, followed by where PDFium
//! was looked for, which reads as a broken installation. PDFium's own answer for
//! a file it cannot read is that one word, "Unknown"; what can be said is said
//! here, from the file's own first and last bytes, without asking PDFium anything.
//!
//! Only things that can be checked cheaply and that are true when they are said.
//! `None` is the honest answer when nothing obvious is wrong — the caller then
//! says only that the structure is damaged.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// How much of each end of the file is looked at.
const PROBE: u64 = 2048;

/// What is wrong with the file at `path`, if something obvious is.
pub fn why_unreadable(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    if length == 0 {
        return Some("the file is empty".into());
    }
    let mut head = vec![0u8; length.min(PROBE) as usize];
    file.read_exact(&mut head).ok()?;
    file.seek(SeekFrom::Start(length.saturating_sub(PROBE))).ok()?;
    let mut tail = Vec::new();
    file.take(PROBE).read_to_end(&mut tail).ok()?;
    why(&head, &tail)
}

/// The same from the two ends themselves.
fn why(head: &[u8], tail: &[u8]) -> Option<String> {
    let has = |haystack: &[u8], needle: &[u8]| haystack.windows(needle.len()).any(|w| w == needle);
    if !has(head, b"%PDF-") {
        return Some("it does not begin like a PDF, so it is not one — or not one that survived being copied".into());
    }
    if !has(tail, b"%%EOF") {
        return Some(
            "the end of the file is missing — it was probably cut short while it was copied, downloaded or synced"
                .into(),
        );
    }
    if !has(tail, b"startxref") {
        return Some("it has no table saying where its pages are".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_is_not_a_pdf_says_so() {
        let text = why(b"hello, this is a letter", b"yours sincerely").unwrap();
        assert!(text.contains("does not begin like a PDF"), "{text}");
    }

    #[test]
    fn a_pdf_cut_short_says_its_end_is_missing() {
        let text = why(b"%PDF-1.7\n1 0 obj", b"...endstream\nendobj\n").unwrap();
        assert!(text.contains("end of the file is missing"), "{text}");
    }

    #[test]
    fn a_pdf_with_an_end_but_no_table_says_so() {
        let text = why(b"%PDF-1.7\n", b"endobj\n%%EOF\n").unwrap();
        assert!(text.contains("no table"), "{text}");
    }

    #[test]
    fn a_pdf_with_both_ends_is_not_blamed_for_anything_obvious() {
        assert_eq!(why(b"%PDF-1.7\n", b"startxref\n116\n%%EOF\n"), None);
    }

    #[test]
    fn files_on_disk_are_read_by_their_ends_only() {
        let dir = std::env::temp_dir().join(format!("pagify-diagnose-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let empty = dir.join("empty.pdf");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(why_unreadable(&empty).as_deref(), Some("the file is empty"));

        let cut = dir.join("cut.pdf");
        let mut body = b"%PDF-1.7\n".to_vec();
        body.extend(std::iter::repeat(b'x').take(100_000));
        std::fs::write(&cut, &body).unwrap();
        assert!(why_unreadable(&cut).unwrap().contains("end of the file is missing"));

        let whole = dir.join("whole.pdf");
        body.extend_from_slice(b"\nstartxref\n9\n%%EOF\n");
        std::fs::write(&whole, &body).unwrap();
        assert_eq!(why_unreadable(&whole), None);

        assert_eq!(why_unreadable(&dir.join("missing.pdf")), None, "a file that is not there is not this module's to explain");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
