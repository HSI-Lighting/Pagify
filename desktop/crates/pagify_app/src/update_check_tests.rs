use super::{newer_version, read_update_manifest, update_outcome, UpdateOutcome};

#[test]
fn a_newer_patch_counts() {
    assert!(newer_version("0.1.0", "0.1.1"));
}

#[test]
fn a_newer_minor_counts_even_with_a_lower_patch() {
    assert!(newer_version("0.1.9", "0.2.0"));
}

#[test]
fn a_newer_major_counts() {
    assert!(newer_version("0.9.9", "1.0.0"));
}

#[test]
fn an_equal_version_is_not_newer() {
    assert!(!newer_version("0.1.0", "0.1.0"));
}

#[test]
fn an_older_version_is_not_newer() {
    assert!(!newer_version("0.2.0", "0.1.9"));
}

#[test]
fn malformed_input_does_not_panic() {
    assert!(!newer_version("not-a-version", "also-not"));
    assert!(!newer_version("", ""));
}

#[test]
fn a_missing_folder_reports_nothing() {
    let dir = std::env::temp_dir().join("pagify-update-check-tests-missing");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(read_update_manifest(&dir), None);
}

#[test]
fn a_published_version_is_read_and_trimmed() {
    let dir = std::env::temp_dir().join(format!(
        "pagify-update-check-tests-{:?}",
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("version.txt"), "0.2.0\n").expect("write manifest");
    assert_eq!(read_update_manifest(&dir), Some("0.2.0".to_string()));
    std::fs::remove_dir_all(&dir).ok();
}

/// Notepad, and Windows PowerShell's own `-Encoding utf8`, both write a
/// byte-order mark by default — this must not become the version's own
/// leading character.
#[test]
fn a_leading_byte_order_mark_does_not_corrupt_the_version() {
    let dir = std::env::temp_dir().join(format!(
        "pagify-update-check-tests-bom-{:?}",
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("version.txt"), "\u{FEFF}0.2.0").expect("write manifest");
    assert_eq!(read_update_manifest(&dir), Some("0.2.0".to_string()));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_empty_manifest_reports_nothing() {
    let dir = std::env::temp_dir().join(format!(
        "pagify-update-check-tests-empty-{:?}",
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("version.txt"), "   \n").expect("write manifest");
    assert_eq!(read_update_manifest(&dir), None);
    std::fs::remove_dir_all(&dir).ok();
}

/// **The three answers are different facts.** A missing folder is "no channel"
/// (what every macOS/Linux machine and most Windows machines will see), a
/// readable manifest that is not newer is "up to date", and only a newer one
/// is offered.
#[test]
fn a_look_at_the_channel_answers_no_channel_up_to_date_or_found() {
    let base = std::env::temp_dir().join(format!(
        "pagify-update-outcome-{:?}",
        std::thread::current().id()
    ));
    let missing = base.join("missing");
    let _ = std::fs::remove_dir_all(&base);
    assert!(
        matches!(update_outcome(&missing, "0.1.0"), UpdateOutcome::NoChannel),
        "a folder that is not there is no update channel"
    );

    std::fs::create_dir_all(&base).expect("scratch dir");
    std::fs::write(base.join("version.txt"), "0.1.0\n").expect("manifest");
    assert!(
        matches!(update_outcome(&base, "0.1.0"), UpdateOutcome::UpToDate),
        "the published build is the one running"
    );
    std::fs::write(base.join("version.txt"), "9.9.9\n").expect("manifest");
    match update_outcome(&base, "0.1.0") {
        UpdateOutcome::Found(version, dir) => {
            assert_eq!(version, "9.9.9");
            assert_eq!(dir, base);
        }
        other => panic!("a newer manifest was not found: {other:?}"),
    }
    std::fs::remove_dir_all(&base).ok();
}
