use super::update_script;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

struct Scratch {
    dir: PathBuf,
    install: PathBuf,
    source: PathBuf,
    log: PathBuf,
    other_window: Option<Child>,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Some(window) = &mut self.other_window {
            let _ = window.kill();
            let _ = window.wait();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn system32() -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot")).join("System32")
}

/// Start a helper program with no console of its own. With Windows Terminal
/// as the default terminal, each console program a test starts opens a tab,
/// and one that is killed leaves it open — a few hundred runs of this suite
/// left a hundred empty terminals on the owner's screen.
fn quiet(command: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000) // CREATE_NO_WINDOW
}

/// An install folder whose `Pagify.exe` is `ping.exe` — kept running for
/// half a minute when `with_other_window` — and a "Dropbox" folder holding
/// a different `Pagify.exe` and a new `pdfium.dll`. The new one is
/// `rundll32.exe`, which exits at once as the relaunch at the end of the
/// script starts it — and which is a *window* program: a console one (the
/// first choice was `hostname.exe`) is given a terminal of its own by
/// `start`, and every run of the suite left one open.
fn scratch(name: &str, with_other_window: bool) -> Scratch {
    let dir = std::env::temp_dir().join(format!("pagify-update-script-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (install, source) = (dir.join("install"), dir.join("dropbox"));
    std::fs::create_dir_all(&install).expect("install dir");
    std::fs::create_dir_all(&source).expect("source dir");
    // Only the test that keeps "another window" running needs `ping.exe` (a
    // program that stays up). In the others the old exe can be relaunched —
    // when the update fails, the script starts what is left — and a console
    // program started that way gets a terminal of its own.
    let old = if with_other_window { "ping.exe" } else { "rundll32.exe" };
    std::fs::copy(system32().join(old), install.join("Pagify.exe")).expect("old exe");
    std::fs::write(install.join("pdfium.dll"), b"old pdfium").expect("old dll");
    std::fs::copy(system32().join("rundll32.exe"), source.join("Pagify.exe")).expect("new exe");
    std::fs::write(source.join("pdfium.dll"), b"new pdfium").expect("new dll");
    let other_window = with_other_window.then(|| {
        quiet(Command::new(install.join("Pagify.exe")).args(["-n", "30", "127.0.0.1"]))
            .stdout(Stdio::null())
            .spawn()
            .expect("the other window")
    });
    Scratch { log: dir.join("update.log"), dir, install, source, other_window }
}

/// Run the script the way `spawn_update_script` does, for a process that
/// has already exited (the one that would have asked for the update).
fn run_the_update(s: &Scratch) {
    let mut gone = quiet(Command::new("cmd").args(["/C", "exit"])).spawn().expect("a short-lived process");
    let pid = gone.id();
    gone.wait().expect("it exits");
    let bat = s.dir.join("update.bat");
    std::fs::write(&bat, update_script(pid, &s.source, &s.install, &s.log, false)).expect("script");
    let status = quiet(Command::new("cmd").arg("/C").arg(&bat)).status().expect("cmd");
    assert!(status.success(), "the update script itself failed: {status}");
}

fn leftovers(s: &Scratch) -> Vec<String> {
    std::fs::read_dir(&s.install)
        .expect("install dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".old-"))
        .collect()
}

/// **Reported from use: "when I click Update now it brings the same update
/// window back".** Another Pagify window was running from the same folder,
/// so its `Pagify.exe` could not be overwritten and the script's `copy`
/// failed without a word.
#[test]
fn the_update_replaces_an_exe_that_another_window_is_running() {
    let mut s = scratch("running", true);
    // Setup check, and the old script's failure itself: the running file
    // really cannot be overwritten.
    assert!(
        std::fs::copy(s.source.join("Pagify.exe"), s.install.join("Pagify.exe")).is_err(),
        "the running exe could be overwritten, so this test proves nothing"
    );

    run_the_update(&s);

    assert_eq!(
        std::fs::read(s.install.join("Pagify.exe")).expect("installed exe"),
        std::fs::read(s.source.join("Pagify.exe")).expect("source exe"),
        "the installed Pagify.exe is not the new one"
    );
    assert_eq!(std::fs::read(s.install.join("pdfium.dll")).expect("dll"), b"new pdfium");
    assert_eq!(leftovers(&s).len(), 1, "the old exe should be renamed aside: {:?}", leftovers(&s));
    assert!(
        s.other_window.as_mut().expect("window").try_wait().expect("wait").is_none(),
        "the other window should still be running"
    );
    assert!(std::fs::read_to_string(&s.log).expect("log").contains("updated"), "the log does not say so");
}

/// With no other window open the files are simply copied over, and what an
/// earlier update had to rename aside is cleared away.
#[test]
fn the_update_with_no_other_window_copies_over_and_clears_old_leftovers() {
    let s = scratch("alone", false);
    std::fs::write(s.install.join("Pagify.exe.old-12345"), b"left by an earlier update").expect("leftover");

    run_the_update(&s);

    assert_eq!(
        std::fs::read(s.install.join("Pagify.exe")).expect("installed exe"),
        std::fs::read(s.source.join("Pagify.exe")).expect("source exe")
    );
    assert_eq!(std::fs::read(s.install.join("pdfium.dll")).expect("dll"), b"new pdfium");
    assert!(leftovers(&s).is_empty(), "an old leftover was not cleared: {:?}", leftovers(&s));
}

/// An update that cannot be finished (here the new exe is not there) must
/// not leave Pagify without an exe, and must say it failed.
#[test]
fn an_update_that_cannot_be_finished_leaves_the_old_exe_in_place_and_says_so() {
    // No other window: after a failed update the script starts what is left,
    // and a console program started that way opens a terminal that stays.
    let s = scratch("failing", false);
    std::fs::remove_file(s.source.join("Pagify.exe")).expect("remove the new exe");

    run_the_update(&s);

    assert_eq!(
        std::fs::read(s.install.join("Pagify.exe")).expect("the installed exe must still exist"),
        std::fs::read(system32().join("rundll32.exe")).expect("rundll32.exe"),
        "the old exe was not put back"
    );
    assert!(leftovers(&s).is_empty(), "a renamed copy was left behind: {:?}", leftovers(&s));
    assert!(std::fs::read_to_string(&s.log).expect("log").contains("FAILED"), "the failure was not logged");
}
