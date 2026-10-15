use super::{newer_version, read_update_manifest};

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

// ---- Check for Updates, asked for: it always answers ----

use crate::update::{Places, UpdateOutcome, TEST_PLACES, TEST_REMEMBERED, TEST_SERIAL};
use crate::ui_tests::harness;
use crate::{Closing, Tab};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

/// A folder as the build script leaves one, with `version` published in it.
fn published(tag: &str, version: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pagify-asked-check-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("version.txt"), version).unwrap();
    std::fs::write(dir.join("Pagify.exe"), b"MZ").unwrap();
    dir
}

/// Say where this PC's published builds are, for the length of one test.
struct Folders(std::sync::MutexGuard<'static, ()>);
impl Folders {
    fn are(folders: Vec<std::path::PathBuf>) -> Folders {
        let guard = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        *TEST_PLACES.lock().unwrap() = Some(Places { fixed: folders, ..Default::default() });
        Folders(guard)
    }

    /// And the remembered folder is kept in `file`.
    fn remembering(self, file: std::path::PathBuf) -> Folders {
        *TEST_REMEMBERED.lock().unwrap() = Some(file);
        self
    }
}
impl Drop for Folders {
    fn drop(&mut self) {
        *TEST_PLACES.lock().unwrap() = None;
        *TEST_REMEMBERED.lock().unwrap() = None;
    }
}

/// Frames until the check's thread has answered (it is a thread and a poll).
fn wait_for_an_answer(h: &mut Harness<'static, crate::PagifyApp>) {
    for _ in 0..200 {
        h.run_steps(1);
        if h.state().update_state.update_check.is_none() {
            h.run_steps(2);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the update check never answered");
}

/// The Help > Check for Updates button, clicked for real.
fn click_check_for_updates(h: &mut Harness<'static, crate::PagifyApp>) {
    h.state_mut().tab_mut().ribbon = Tab::Help;
    h.run_steps(2);
    h.get_by_label_contains("Check for Updates").click();
    h.run_steps(1);
    wait_for_an_answer(h);
}

/// **The button used to say the tool was planned for "the documentation phase".**
/// It named `checkupdates`; the live verb is `checkupdate`.
#[test]
fn the_help_button_runs_the_check_and_a_newer_build_is_offered_in_a_window() {
    let published_here = published("newer", "99.0.0");
    let _folders = Folders::are(vec![published_here.clone()]);
    let mut h = harness("two-column.pdf");

    click_check_for_updates(&mut h);

    assert_eq!(
        h.state().update_state.update_report,
        Some(UpdateOutcome::Newer { version: "99.0.0".into(), folder: published_here }),
    );
    h.get_by_label_contains("Pagify 99.0.0 is available");
}

#[test]
fn up_to_date_says_so_and_the_button_installs_the_published_build_anyway() {
    let here = published("same", pagify_shell::VERSION);
    let _folders = Folders::are(vec![here.clone()]);
    let mut h = harness("two-column.pdf");
    h.state_mut().submit("l 10,10 100,100");
    assert!(h.state().unsaved().is_some(), "test assumption: the mark should count as unsaved");

    click_check_for_updates(&mut h);
    assert!(matches!(h.state().update_state.update_report, Some(UpdateOutcome::Current { .. })));
    h.get_by_label_contains("You have the latest build");
    assert!(h.state().hub_state.pending_update.is_none(), "asking must not stage anything");

    // The force: the same build, installed over this one.
    h.get_by_label_contains("Install it anyway").click();
    h.run_steps(1);
    assert_eq!(h.state().hub_state.pending_update, Some(here));
    assert!(h.state().update_state.update_report.is_none(), "the window should have closed");
    assert_eq!(
        h.state().tab().closing,
        Some(Closing::Program),
        "a forced update goes through the unsaved-work guard like any quit"
    );
}

#[test]
fn when_this_pc_cannot_see_the_update_folder_it_says_where_it_looked() {
    let nowhere = std::env::temp_dir().join("pagify-asked-check-nowhere");
    let _ = std::fs::remove_dir_all(&nowhere);
    let _folders = Folders::are(vec![nowhere.clone()]);
    let mut h = harness("two-column.pdf");

    click_check_for_updates(&mut h);

    assert_eq!(h.state().update_state.update_report, Some(UpdateOutcome::NoFolder { tried: vec![nowhere.clone()], searched: Vec::new() }));
    h.get_by_label_contains("cannot see the folder updates are published to");
    h.get_by_label_contains(&nowhere.display().to_string());
    h.get_by_label_contains("No Dropbox was found on this PC");
    h.get_by_label_contains("Choose folder");
    // Nothing to install from: no install button.
    assert!(h.query_by_label_contains("Install it anyway").is_none());
    assert!(h.query_by_label_contains("Update now").is_none());
    h.get_by_label_contains("Close").click();
    h.run_steps(1);
    assert!(h.state().update_state.update_report.is_none());
}

/// The launch-time look keeps its manners: silent unless there is something to offer.
#[test]
fn the_launch_time_check_stays_quiet_unless_a_newer_build_is_there() {
    let here = published("quiet", pagify_shell::VERSION);
    let _folders = Folders::are(vec![here]);
    let mut h = harness("two-column.pdf");
    h.state_mut().spawn_update_check(false);
    wait_for_an_answer(&mut h);
    assert!(h.state().update_state.update_report.is_none(), "an unasked check opened a window");
    assert!(h.state().update_state.update_available.is_none());

    let newer = published("quiet-newer", "99.0.0");
    *TEST_PLACES.lock().unwrap() = Some(Places { fixed: vec![newer.clone()], ..Default::default() });
    h.state_mut().spawn_update_check(false);
    wait_for_an_answer(&mut h);
    assert_eq!(h.state().update_state.update_available, Some(("99.0.0".to_string(), newer)));
    assert!(h.state().update_state.update_report.is_none());
}

/// A press while the launch-time look is still running does not start a second one:
/// it turns that one into an asked-for one.
#[test]
fn asking_while_the_launch_check_is_running_gets_its_answer_shown() {
    let here = published("inflight", pagify_shell::VERSION);
    let _folders = Folders::are(vec![here]);
    let mut h = harness("two-column.pdf");
    h.state_mut().spawn_update_check(false);
    h.state_mut().spawn_update_check(true);
    wait_for_an_answer(&mut h);
    assert!(matches!(h.state().update_state.update_report, Some(UpdateOutcome::Current { .. })));
}

/// **The way out on any PC with an odd layout:** point Pagify at the folder once. It
/// looks there at once, and remembers it.
#[test]
fn a_folder_the_reader_chooses_is_looked_in_at_once_and_remembered() {
    let nowhere = std::env::temp_dir().join("pagify-asked-check-chosen-nowhere");
    let _ = std::fs::remove_dir_all(&nowhere);
    let memory = std::env::temp_dir().join(format!("pagify-asked-check-memory-{}", std::process::id())).join("update-folder.txt");
    let _ = std::fs::remove_dir_all(memory.parent().unwrap());
    let _folders = Folders::are(vec![nowhere]).remembering(memory.clone());
    let mut h = harness("two-column.pdf");
    click_check_for_updates(&mut h);
    assert!(matches!(h.state().update_state.update_report, Some(UpdateOutcome::NoFolder { .. })));

    // What the Choose folder button does once a folder is picked.
    let chosen = published("chosen", "99.0.0");
    h.state_mut().use_update_folder(chosen.clone());
    wait_for_an_answer(&mut h);

    assert_eq!(
        h.state().update_state.update_report,
        Some(UpdateOutcome::Newer { version: "99.0.0".into(), folder: chosen.clone() }),
    );
    assert_eq!(std::fs::read_to_string(&memory).unwrap(), chosen.to_string_lossy());
}

/// A build found, by any check, is where the next check looks first.
#[test]
fn the_folder_a_check_found_a_build_in_is_remembered() {
    let here = published("remember", pagify_shell::VERSION);
    let memory = std::env::temp_dir().join(format!("pagify-asked-check-memory2-{}", std::process::id())).join("update-folder.txt");
    let _ = std::fs::remove_dir_all(memory.parent().unwrap());
    let _folders = Folders::are(vec![here.clone()]).remembering(memory.clone());
    let mut h = harness("two-column.pdf");
    assert!(!memory.exists());

    h.state_mut().spawn_update_check(false);
    wait_for_an_answer(&mut h);

    assert_eq!(std::fs::read_to_string(&memory).unwrap(), here.to_string_lossy());
}
