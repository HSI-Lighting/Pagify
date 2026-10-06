use super::*;

fn said(app: &PagifyApp) -> String {
    app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pagify-open-error-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// One blank page in an object stream, with a cross-reference stream whose
/// table holds `<<` in an offset — and, when `damaged`, the type the old
/// repair wrote into that table. Same shape as `pdf_core`'s own test of it.
fn pdf(damaged: bool) -> Vec<u8> {
    let bodies = [
        "<< /Type /Catalog /Pages 3 0 R >>",
        "<< /Type /Pages /Kids [4 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 3 0 R /MediaBox [0 0 200 300] >>",
    ];
    let (mut header, mut packed) = (String::new(), String::new());
    for (i, body) in bodies.iter().enumerate() {
        header.push_str(&format!("{} {} ", i + 2, packed.len()));
        packed.push_str(body);
        packed.push('\n');
    }
    let (first, objstm) = (header.len(), format!("{header}{packed}"));
    const AT: usize = 0x3C3C;
    let mut out = b"%PDF-1.5\n%".to_vec();
    out.resize(AT - 1, b'x');
    out.push(b'\n');
    out.extend_from_slice(
        format!(
            "1 0 obj\n<< /Type /ObjStm /N 3 /First {first} /Length {} >>\nstream\n{objstm}\nendstream\nendobj\n",
            objstm.len()
        )
        .as_bytes(),
    );
    let table_at = out.len();
    let mut data: Vec<u8> = vec![0, 0, 0, 0, 0, 255, 1];
    data.extend_from_slice(&(AT as u32).to_be_bytes());
    data.push(0);
    for index in 0..3u8 {
        data.push(2);
        data.extend_from_slice(&1u32.to_be_bytes());
        data.push(index);
    }
    data.push(1);
    data.extend_from_slice(&(table_at as u32).to_be_bytes());
    data.push(0);
    let length = data.len();
    if damaged {
        let at = data.windows(2).position(|w| w == b"<<").unwrap() + 2;
        data.splice(at..at, b"/Type/XRef".iter().copied());
    }
    out.extend_from_slice(
        format!(
            "5 0 obj\n<<{}/Root 2 0 R/Size 6/W[1 4 1]/Length {length}>>stream\r\n",
            if damaged { "" } else { "/Type/XRef" }
        )
        .as_bytes(),
    );
    out.extend_from_slice(&data);
    out.extend_from_slice(format!("\r\nendstream\nendobj\nstartxref\n{table_at}\n%%EOF\n").as_bytes());
    out
}

#[test]
fn a_file_an_older_build_damaged_opens_and_says_it_was_repaired() {
    let path = scratch("damaged.pdf");
    std::fs::write(&path, pdf(true)).unwrap();
    let mut app = PagifyApp::new(None);
    app.open(path.to_str().unwrap());

    assert!(app.tab().doc.is_some(), "the damaged file did not open:\n{}", said(&app));
    assert!(said(&app).contains("had been damaged by a save in an older version"), "{}", said(&app));
    // And the file on disk was left as it was.
    assert_eq!(std::fs::read(&path).unwrap(), pdf(true));
}

#[test]
fn a_sound_file_is_not_said_to_have_been_repaired() {
    let path = scratch("sound.pdf");
    std::fs::write(&path, pdf(false)).unwrap();
    let mut app = PagifyApp::new(None);
    app.open(path.to_str().unwrap());
    assert!(app.tab().doc.is_some(), "{}", said(&app));
    assert!(!said(&app).contains("damaged"), "{}", said(&app));
}

#[test]
fn a_file_that_will_not_open_says_what_is_wrong_with_it_and_does_not_blame_the_library() {
    let not_a_pdf = scratch("letter.pdf");
    std::fs::write(&not_a_pdf, b"Dear Sir, this is a letter and not a PDF at all.").unwrap();
    let mut app = PagifyApp::new(None);
    app.open(not_a_pdf.to_str().unwrap());
    let said_so = said(&app);
    assert!(said_so.contains("letter.pdf could not be opened"), "{said_so}");
    assert!(said_so.contains("does not begin like a PDF"), "{said_so}");
    assert!(!said_so.contains("PDFium was looked for"), "it blamed the install:\n{said_so}");
    assert!(!said_so.contains("PdfiumLibraryInternalError"), "{said_so}");
    assert!(app.tab().doc.is_none());

    // One cut short while it was copied.
    let cut = scratch("cut.pdf");
    let whole = pdf(false);
    std::fs::write(&cut, &whole[..whole.len() - 40]).unwrap();
    let mut app = PagifyApp::new(None);
    app.open(cut.to_str().unwrap());
    // Whether PDFium refuses this or recovers from it, it never blames the library.
    assert!(!said(&app).contains("PDFium was looked for"), "{}", said(&app));
}
