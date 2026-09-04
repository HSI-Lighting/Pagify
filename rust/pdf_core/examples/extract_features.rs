//! Turn a labelled dataset into the feature matrix a trainer reads.
//!
//! **This binary is the whole of Part 13's contract.** The training script does
//! not compute features; it is handed them, by the same code that will compute
//! them at inference. There is no second implementation to drift from the
//! first, which is the failure Part 13 calls the most likely in the project —
//! and the one with no symptom except a model that is quietly worse than the
//! rules it replaced.
//!
//! ```text
//! cargo run --release --example extract_features -- <dataset.json> <out.tsv>
//! ```
//!
//! The dataset is read from a path rather than the tree on purpose: its
//! validation half is twenty third parties' real names and employers, and
//! Part 7 does not let that be committed. See `contacts::labels::Dataset`.
//!
//! The output is one row per line: the label column, then `LEXICAL_WIDTH`
//! values, tab separated. The header carries the feature version, and a trainer
//! that ignores it will eventually train against a vector layout the app no
//! longer produces.

use pdf_core::contacts::features::{self, LEXICAL_WIDTH};
use pdf_core::contacts::labels::{Dataset, LineLabel};
use std::io::Write;

fn main() {
    let mut args = std::env::args().skip(1);
    let (input, output) = match (args.next(), args.next()) {
        (Some(input), Some(output)) => (input, output),
        _ => {
            eprintln!("usage: extract_features <dataset.json> <out.tsv>");
            std::process::exit(2);
        }
    };

    let raw = match std::fs::read_to_string(&input) {
        Ok(raw) => raw,
        Err(error) => {
            eprintln!("could not read {input}: {error}");
            std::process::exit(1);
        }
    };
    let dataset: Dataset = match serde_json::from_str(&raw) {
        Ok(dataset) => dataset,
        Err(error) => {
            eprintln!("{input} is not a dataset: {error}");
            std::process::exit(1);
        }
    };

    let counts = dataset.counts();
    eprintln!("{} rows from {input}", dataset.lines.len());
    for (label, count) in LineLabel::ALL.iter().zip(counts.iter()) {
        eprintln!("  {:<13} {count}", label.as_str());
    }
    // Said out loud rather than left for the trainer to discover. A class with
    // almost no rows produces a model that is confidently never that class, and
    // the accuracy figure will look fine because the class is rare.
    if let Some(thin) = LineLabel::ALL.iter().zip(counts.iter()).find(|(_, c)| **c < 20) {
        eprintln!(
            "\nwarning: only {} rows labelled {} — too few to learn from",
            thin.1,
            thin.0.as_str(),
        );
    }

    let file = match std::fs::File::create(&output) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("could not write {output}: {error}");
            std::process::exit(1);
        }
    };
    let mut out = std::io::BufWriter::new(file);

    // The version goes in the file. A matrix outlives the session that made it,
    // and a trainer cannot otherwise tell that the extractor has moved under it.
    writeln!(out, "# feature_version\t{}", features::VERSION).unwrap();
    writeln!(out, "# width\t{LEXICAL_WIDTH}").unwrap();
    write!(out, "label").unwrap();
    for index in 0..LEXICAL_WIDTH {
        write!(out, "\tf{index}").unwrap();
    }
    writeln!(out).unwrap();

    for line in &dataset.lines {
        write!(out, "{}", line.label.column()).unwrap();
        for value in features::extract_lexical(&line.text) {
            write!(out, "\t{value}").unwrap();
        }
        writeln!(out).unwrap();
    }

    out.flush().unwrap();
    eprintln!("\nwrote {output}: {} rows of {LEXICAL_WIDTH}", dataset.lines.len());
}
