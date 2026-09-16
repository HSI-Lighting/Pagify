//! The tool driven as a person drives it: the built binary, a subprocess,
//! passphrases on stdin, output read from stdout and stderr — none of the
//! functions inside `main.rs` called directly. What the unit tests check
//! about the certificates, this checks about the *tool*: the refusal before
//! a restore check, the mismatch refusal, that a wrong passphrase is refused
//! and a right one is not, and that the root this makes is one
//! `rust/pdf_core/trust` would actually pin.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_pagify-issue"))
}

/// A scratch directory under `target/`, unique to this test process and this
/// call — so tests run with `--test-threads` greater than one do not collide,
/// and nothing is left in a real person's temp directory.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp-pagify-issue-tests")
        .join(format!("{name}-{}-{}", std::process::id(), name.len()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Run the tool with `stdin_lines` fed to it one per line, and return
/// (stdout, stderr, success).
fn run(args: &[&str], stdin_lines: &[&str]) -> (String, String, bool) {
    let mut child = Command::new(bin())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn pagify-issue");
    {
        let mut input = child.stdin.take().expect("stdin");
        for line in stdin_lines {
            writeln!(input, "{line}").expect("write stdin");
        }
    }
    let output = child.wait_with_output().expect("wait");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

/// A root made in `dir`, backed up to `dir/../media`, and restore-checked —
/// the state every other test in this file needs before it can issue.
fn root_ready(dir: &Path) -> PathBuf {
    let (_, err, ok) = run(
        &["root", "new", "--dir", dir.to_str().unwrap(), "--subject", "CN=CLI Test Root,O=Pagify"],
        &["cli-test-pass", "cli-test-pass"],
    );
    assert!(ok, "root new failed: {err}");
    let media = dir.join("media");
    let (_, err, ok) = run(
        &["root", "backup", "--dir", dir.to_str().unwrap(), "--to", media.to_str().unwrap()],
        &[],
    );
    assert!(ok, "root backup failed: {err}");
    let (_, err, ok) = run(
        &[
            "root",
            "restore-check",
            "--dir",
            dir.to_str().unwrap(),
            "--backup",
            media.to_str().unwrap(),
        ],
        &["cli-test-pass"],
    );
    assert!(ok, "restore-check failed: {err}");
    media
}

/// **No usage, no crash.** Called with nothing, or with something that is not
/// one of the five commands, the tool prints usage and exits non-zero —
/// never a panic, never a stack trace.
#[test]
fn bad_arguments_get_usage_not_a_crash() {
    for args in [vec![], vec!["frobnicate"], vec!["root"], vec!["leaf", "revoke"]] {
        let (_, err, ok) = run(&args, &[]);
        assert!(!ok, "{args:?} should fail");
        assert!(err.contains("usage:"), "{args:?}: {err}");
    }
}

/// **A mismatched passphrase is refused before anything is written** — no
/// root left half-made for the next run to trip on.
#[test]
fn a_mismatched_passphrase_leaves_nothing_behind() {
    let dir = scratch("mismatch");
    let (_, err, ok) = run(
        &["root", "new", "--dir", dir.to_str().unwrap(), "--subject", "CN=X,O=Y"],
        &["one", "two"],
    );
    assert!(!ok);
    assert!(err.contains("do not match"), "{err}");
    assert!(!dir.join("root.der").exists(), "a root was written despite the mismatch");
    assert!(!dir.join("root.key").exists());
}

/// **`leaf issue` refuses before a restore check** — the whole reason the
/// state file exists — and the message names the command to run.
#[test]
fn leaf_issue_refuses_before_a_restore_check() {
    let dir = scratch("no-restore");
    let (_, err, ok) = run(
        &["root", "new", "--dir", dir.to_str().unwrap(), "--subject", "CN=X,O=Y"],
        &["pw", "pw"],
    );
    assert!(ok, "{err}");
    let (_, err, ok) = run(
        &[
            "leaf",
            "issue",
            "--dir",
            dir.to_str().unwrap(),
            "--subject",
            "CN=Alice,O=Y",
            "--out",
            dir.join("alice.p12").to_str().unwrap(),
        ],
        &["pw"],
    );
    assert!(!ok);
    assert!(err.contains("restore-check"), "{err}");
    assert!(!dir.join("alice.p12").exists());
}

/// **A wrong root passphrase is refused**, both for a restore check and for
/// issuing — and a right one is not.
#[test]
fn a_wrong_root_passphrase_is_refused() {
    let dir = scratch("wrong-pass");
    let media = root_ready(&dir);

    let (_, err, ok) = run(
        &[
            "root",
            "restore-check",
            "--dir",
            dir.join("elsewhere").to_str().unwrap(),
            "--backup",
            media.to_str().unwrap(),
        ],
        &["not the passphrase"],
    );
    assert!(!ok);
    assert!(err.contains("did not open"), "{err}");

    let (_, err, ok) = run(
        &[
            "leaf",
            "issue",
            "--dir",
            dir.to_str().unwrap(),
            "--subject",
            "CN=Alice,O=Y",
            "--out",
            dir.join("alice.p12").to_str().unwrap(),
        ],
        &["not the passphrase either"],
    );
    assert!(!ok);
    assert!(err.contains("did not open"), "{err}");
    assert!(!dir.join("alice.p12").exists());
}

/// **The happy path, end to end, through the binary**: a root, a backup, a
/// restore check, and a leaf — and the leaf that comes out is one
/// `pdf_core::pdf::trust` pins under that root, checked here by calling the
/// engine directly on the two files the tool wrote. The identity opens with
/// its own password, and the ledger and the denylist line reflect the serial
/// actually issued.
#[test]
fn a_leaf_issued_end_to_end_is_pinned_under_the_root_it_names() {
    let dir = scratch("happy-path");
    root_ready(&dir);

    let out = dir.join("alice.p12");
    let (said, err, ok) = run(
        &[
            "leaf",
            "issue",
            "--dir",
            dir.to_str().unwrap(),
            "--subject",
            "CN=Alice,O=Pagify",
            "--out",
            out.to_str().unwrap(),
        ],
        &["cli-test-pass", "alice-identity-pw", "alice-identity-pw"],
    );
    assert!(ok, "{err}");
    assert!(said.contains("CN=Alice,O=Pagify"), "{said}");
    assert!(out.is_file());

    let ledger = std::fs::read_to_string(dir.join("issued.txt")).expect("ledger");
    assert!(ledger.contains("CN=Alice,O=Pagify"), "{ledger}");
    let serial_line = ledger.lines().find(|l| l.contains("Alice")).expect("Alice's line");
    let serial = serial_line.split_whitespace().next().expect("serial");
    assert_eq!(serial, "1001", "the first leaf from a fresh root gets the ledger's first serial");

    // The engine checks the pair the way it will check a real document: the
    // leaf's certificate, pulled out of the .p12, against the root's DER.
    let root_der = std::fs::read(dir.join("root.der")).expect("root.der");
    let identity =
        pdf_core::pdf::sign::Identity::from_pkcs12(&std::fs::read(&out).expect("p12"), "alice-identity-pw")
            .expect("the identity opens with its own password");
    assert_eq!(identity.subject().unwrap(), "CN=Alice,O=Pagify");
    identity.sm2_key().expect("an SM2 key, signs");
    assert!(
        pdf_core::pdf::sign::Identity::from_pkcs12(&std::fs::read(&out).unwrap(), "wrong password").is_err(),
        "the identity opened with the wrong password"
    );

    use der::Decode;
    let certificate = x509_cert::Certificate::from_der(&identity.certificates[0]).expect("certificate");
    let anchors = pdf_core::pdf::trust::Anchors::new(&root_der, "").expect("anchors");
    assert_eq!(
        pdf_core::pdf::trust::trust_in(&certificate, &anchors),
        pdf_core::pdf::trust::Trust::Pinned,
        "the leaf pagify-issue wrote does not chain to the root it wrote"
    );

    // And the denylist line it prints is the one that revokes exactly this
    // leaf, by issuer and serial — matched the way `Anchors` reads a line.
    let (line, err, ok) =
        run(&["leaf", "denylist-line", "--dir", dir.to_str().unwrap(), "--serial", serial], &[]);
    assert!(ok, "{err}");
    let denied = pdf_core::pdf::trust::Anchors::new(&root_der, line.trim()).expect("denylist parses");
    assert_eq!(pdf_core::pdf::trust::trust_in(&certificate, &denied), pdf_core::pdf::trust::Trust::Revoked);

    // A second leaf gets the next serial, and is pinned too.
    let bob_out = dir.join("bob.p12");
    let (said, err, ok) = run(
        &[
            "leaf",
            "issue",
            "--dir",
            dir.to_str().unwrap(),
            "--subject",
            "CN=Bob,O=Pagify",
            "--out",
            bob_out.to_str().unwrap(),
        ],
        &["cli-test-pass", "bob-pw", "bob-pw"],
    );
    assert!(ok, "{err}");
    assert!(said.contains("1002"), "{said}");

    // And this tool will not overwrite an identity that already exists.
    let (_, err, ok) = run(
        &[
            "leaf",
            "issue",
            "--dir",
            dir.to_str().unwrap(),
            "--subject",
            "CN=Alice Again,O=Pagify",
            "--out",
            out.to_str().unwrap(),
        ],
        &["cli-test-pass"],
    );
    assert!(!ok);
    assert!(err.contains("exists"), "{err}");
}

/// **`root new` will not overwrite an existing root.**
#[test]
fn root_new_refuses_to_overwrite_an_existing_root() {
    let dir = scratch("no-overwrite");
    root_ready(&dir);
    let (_, err, ok) = run(
        &["root", "new", "--dir", dir.to_str().unwrap(), "--subject", "CN=Different,O=Pagify"],
        &["pw", "pw"],
    );
    assert!(!ok);
    assert!(err.contains("already holds a root"), "{err}");
}
