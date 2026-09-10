//! Does the reader in `pdf::` cope with real files?
//!
//! A parser that passes its own hand-written fixtures and falls over on a
//! 40 MB catalogue has proved nothing. This walks every object a file names and
//! reports what could not be read — the number that has to be zero before Lock
//! is allowed to rewrite anything.
//!
//! ```text
//! cargo run --example pdf_reader_probe -- <file.pdf>
//! ```

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let bytes = std::fs::read(&path).expect("read");
    println!("{path}: {} bytes", bytes.len());

    let file = match pdf_core::pdf::File::parse(&bytes) {
        Ok(file) => file,
        Err(e) => {
            println!("refused: {e}");
            return;
        }
    };

    let numbers: Vec<u32> = file.numbers().collect();
    let (mut read, mut streams, mut images) = (0usize, 0usize, 0usize);
    let mut problems: Vec<String> = Vec::new();

    for number in &numbers {
        match file.object(*number) {
            Ok(object) => {
                read += 1;
                if let pdf_core::pdf::Object::Stream(dict, _) = &object {
                    streams += 1;
                    if dict.get(b"Subtype").and_then(pdf_core::pdf::Object::as_name)
                        == Some(&b"Image"[..])
                    {
                        images += 1;
                    }
                }
            }
            Err(e) => {
                if problems.len() < 8 {
                    problems.push(format!("  object {number}: {e}"));
                }
            }
        }
    }

    println!("objects named : {}", numbers.len());
    println!("objects read  : {read}");
    println!("  streams     : {streams}");
    println!("  images      : {images}");
    println!("unreadable    : {}", numbers.len() - read);
    for problem in &problems {
        println!("{problem}");
    }

    // The writer's own claim, checked against the whole file: rewriting
    // nothing must leave every object readable and identical.
    let rewritten = match file.rewrite(&[]) {
        Ok(bytes) => bytes,
        Err(e) => {
            println!("\nrewrite refused: {e}");
            return;
        }
    };
    println!("\nrewritten     : {} bytes", rewritten.len());

    let after = match pdf_core::pdf::File::parse(&rewritten) {
        Ok(after) => after,
        Err(e) => {
            println!("the rewritten file does not parse: {e}");
            return;
        }
    };
    let mut differed = 0usize;
    let mut missing = 0usize;
    for number in &numbers {
        let (before, now) = (file.object(*number), after.object(*number));
        match (before, now) {
            (Ok(before), Ok(now)) => {
                let (mut a, mut b) = (Vec::new(), Vec::new());
                pdf_core::pdf::write_object(&mut a, &before);
                pdf_core::pdf::write_object(&mut b, &now);
                if a != b {
                    differed += 1;
                }
            }
            _ => missing += 1,
        }
    }
    println!("objects after : {}", after.numbers().count());
    println!("  differing   : {differed}");
    println!("  unreadable  : {missing}");
}
