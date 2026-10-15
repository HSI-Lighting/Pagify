//! Looking for a newer build, and saying what was found — including *why not*.
//!
//! **Why this exists.** The launch-time check was silent in every case but one: a
//! newer build in `UPDATE_FOLDER`. A PC that could not see that folder (Dropbox on
//! another drive, a shared folder mounted somewhere else, not yet synced) found
//! "nothing newer" in exactly the same way a PC that was up to date did, so "the
//! update does not work here" could not be told from "there is no update". The
//! Help ▸ **Check for Updates** button was worse: it named a verb the
//! planned-features table still claimed.
//!
//! **Where it looks, cheapest and surest first** — never the whole disk:
//! 1. the folder it last found a build in (remembered in Pagify's own data folder, or
//!    the one the reader chose by hand);
//! 2. the folder the build script publishes to;
//! 3. the same folder under each Dropbox this PC has — Dropbox's own `info.json` says
//!    where that is — as `<root>\YASEEN\pagify desktop` and `<root>\pagify desktop`
//!    (which is where a folder *shared* with this PC appears);
//! 4. only if none of those holds a build: a shallow look, two folder levels down,
//!    under each Dropbox root, for a folder called `pagify desktop`. A few hundred
//!    directory entries, never a walk of the whole Dropbox.
//!
//! [`look`] is the whole decision, with no window and no thread, so each answer is
//! tested against real folders. The app owns running it (`spawn_update_check`) and
//! showing it (`draw_update_report`).

use std::path::{Path, PathBuf};

/// The name of the folder a build is published in, wherever it sits.
const PUBLISHED_NAME: &str = "pagify desktop";

/// The most directories the shallow search will look at, in all. A Dropbox with
/// tens of thousands of folders is not read through to find this one.
const SEARCH_BUDGET: usize = 4000;

/// What a look at the published folder found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UpdateOutcome {
    /// `version` is published in `folder` and is newer than this build.
    Newer { version: String, folder: PathBuf },
    /// `version` is published in `folder` and is this build's, or older. Installing
    /// it anyway is the **force**: the published build goes over this one whatever
    /// its number.
    Current { version: String, folder: PathBuf },
    /// `folder` exists but holds no readable `version.txt`, or no `Pagify.exe` to
    /// install — Dropbox still syncing, or a half-published folder.
    Incomplete { folder: PathBuf },
    /// No folder holding a build exists on this PC: where it looked, and the Dropbox
    /// roots it searched inside (none, if this PC has no Dropbox).
    NoFolder { tried: Vec<PathBuf>, searched: Vec<PathBuf> },
}

impl UpdateOutcome {
    /// The folder a build was found in, for remembering.
    pub(crate) fn found_in(&self) -> Option<&Path> {
        match self {
            Self::Newer { folder, .. } | Self::Current { folder, .. } => Some(folder),
            _ => None,
        }
    }

    /// A content-free line for the session log: what was found, never a document.
    pub(crate) fn log_line(&self, current: &str) -> String {
        let list = |paths: &[PathBuf]| paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("; ");
        match self {
            Self::Newer { version, folder } => format!("current={current} published={version} newer folder={}", folder.display()),
            Self::Current { version, folder } => format!("current={current} published={version} not-newer folder={}", folder.display()),
            Self::Incomplete { folder } => format!("current={current} incomplete folder={}", folder.display()),
            Self::NoFolder { tried, searched } => {
                format!("current={current} no-folder tried=[{}] searched=[{}]", list(tried), list(searched))
            }
        }
    }
}

/// What this PC offers to look in. Built from the real PC by [`Places::this_pc`], or
/// by hand in a test.
#[derive(Debug, Clone, Default)]
pub(crate) struct Places {
    /// Where a build was last found, or where the reader pointed.
    pub remembered: Option<PathBuf>,
    /// Folders to look in as they are (the build script's own).
    pub fixed: Vec<PathBuf>,
    /// The per-user data folders Dropbox keeps its `info.json` in.
    pub app_data: Vec<PathBuf>,
    /// The user's home, for a Dropbox at its default place.
    pub home: Option<PathBuf>,
}

impl Places {
    /// This PC.
    pub(crate) fn this_pc() -> Places {
        #[cfg(test)]
        if let Some(mut places) = TEST_PLACES.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            places.remembered = remembered();
            return places;
        }
        Places {
            remembered: remembered(),
            fixed: vec![PathBuf::from(crate::UPDATE_FOLDER)],
            app_data: ["LOCALAPPDATA", "APPDATA"].iter().filter_map(std::env::var_os).map(PathBuf::from).collect(),
            home: std::env::var_os("USERPROFILE").map(PathBuf::from),
        }
    }

    /// The Dropbox roots on this PC that exist: its own `info.json` first (personal,
    /// business), then the default place under the home.
    fn dropbox_roots(&self) -> Vec<PathBuf> {
        let from_info = self.app_data.iter().flat_map(|base| dropbox_roots(&base.join("Dropbox").join("info.json")));
        let by_default = self.home.iter().map(|home| home.join("Dropbox"));
        let mut roots: Vec<PathBuf> = from_info.chain(by_default).filter(|r| r.is_dir()).collect();
        dedup_paths(&mut roots);
        roots
    }

    /// The places worth a plain look, in order: what is remembered, the fixed folders,
    /// then the two spots a published folder sits in under each Dropbox root.
    fn known(&self, roots: &[PathBuf]) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = self.remembered.iter().chain(self.fixed.iter()).cloned().collect();
        for root in roots {
            found.push(root.join("YASEEN").join(PUBLISHED_NAME));
            found.push(root.join(PUBLISHED_NAME));
        }
        dedup_paths(&mut found);
        found
    }
}

/// Windows folders are case-insensitive: the same place spelt twice is one place.
fn dedup_paths(paths: &mut Vec<PathBuf>) {
    let mut seen: Vec<String> = Vec::new();
    paths.retain(|p| {
        let key = p.to_string_lossy().to_lowercase();
        let new = !seen.contains(&key);
        seen.push(key);
        new
    });
}

/// Look for a published build, as described at the top of this file.
pub(crate) fn look(current: &str, places: &Places) -> UpdateOutcome {
    let roots = places.dropbox_roots();
    let mut folders = places.known(&roots);
    let first = check(current, &folders);
    if matches!(first, UpdateOutcome::Newer { .. } | UpdateOutcome::Current { .. }) {
        return first;
    }
    // Nothing in the usual places: the shallow search, inside Dropbox only.
    let mut budget = SEARCH_BUDGET;
    let searched: Vec<PathBuf> = roots.iter().flat_map(|root| search_under(root, &mut budget)).collect();
    folders.extend(searched);
    dedup_paths(&mut folders);
    match check(current, &folders) {
        UpdateOutcome::NoFolder { tried, .. } => UpdateOutcome::NoFolder { tried, searched: roots },
        other => other,
    }
}

/// Folders named `pagify desktop` within two levels of `root`.
fn search_under(root: &Path, budget: &mut usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for first in subdirs(root, budget) {
        if is_published_name(&first) {
            found.push(first.clone());
        }
        for second in subdirs(&first, budget) {
            if is_published_name(&second) {
                found.push(second);
            }
        }
    }
    found
}

fn is_published_name(dir: &Path) -> bool {
    dir.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(PUBLISHED_NAME))
}

/// The directories directly inside `dir`, as many as `budget` still allows. A
/// directory entry already says whether it is one, so nothing is `stat`ed.
fn subdirs(dir: &Path, budget: &mut usize) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        if *budget == 0 {
            break;
        }
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            *budget -= 1;
            found.push(entry.path());
        }
    }
    found
}

/// Look in each of `folders`, in order, for a published build.
///
/// The first folder with a readable `version.txt` **and** a `Pagify.exe` beside it
/// decides. `version.txt` is a few bytes and reaches a PC long before a 30 MB exe
/// does, so offering an update on the strength of the number alone would install
/// whatever old exe was still there — and offer the same update again.
fn check(current: &str, folders: &[PathBuf]) -> UpdateOutcome {
    let mut incomplete: Option<&PathBuf> = None;
    for folder in folders.iter().filter(|f| f.is_dir()) {
        match crate::read_update_manifest(folder) {
            Some(version) if folder.join("Pagify.exe").is_file() => {
                let folder = folder.clone();
                return if crate::newer_version(current, &version) {
                    UpdateOutcome::Newer { version, folder }
                } else {
                    UpdateOutcome::Current { version, folder }
                };
            }
            _ => {
                incomplete.get_or_insert(folder);
            }
        }
    }
    match incomplete {
        Some(folder) => UpdateOutcome::Incomplete { folder: folder.clone() },
        None => UpdateOutcome::NoFolder { tried: folders.to_vec(), searched: Vec::new() },
    }
}

/// The folders Dropbox's own `info.json` says it syncs into (personal, business).
fn dropbox_roots(info: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(info) else { return Vec::new() };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{FEFF}')) else {
        return Vec::new();
    };
    json.as_object()
        .into_iter()
        .flat_map(|accounts| accounts.values())
        .filter_map(|account| account.get("path")?.as_str().map(PathBuf::from))
        .collect()
}

// ---- Remembering where a build was found ----

/// The file Pagify keeps the update folder's path in: its own data folder, never
/// beside the exe (which may be in Program Files). A test names its own.
fn remembered_file() -> Option<PathBuf> {
    #[cfg(test)]
    {
        TEST_REMEMBERED.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    #[cfg(not(test))]
    {
        pagify_shell::state::state_dir().map(|dir| dir.join("update-folder.txt"))
    }
}

/// The folder a build was last found in, or chosen by hand — if it is still there.
pub(crate) fn remembered() -> Option<PathBuf> {
    let text = std::fs::read_to_string(remembered_file()?).ok()?;
    let path = PathBuf::from(text.trim().trim_start_matches('\u{FEFF}'));
    path.is_dir().then_some(path)
}

/// Keep `folder` as the first place to look next time. Quiet when it already is,
/// and on any failure: a check that cannot remember is still a check.
pub(crate) fn remember(folder: &Path) {
    let Some(file) = remembered_file() else { return };
    if remembered().is_some_and(|known| known == folder) {
        return;
    }
    let _ = pagify_shell::state::write_own(&file, folder.to_string_lossy().as_bytes());
}

// ---- Test seams: no test reads or writes this PC's own Dropbox or data folder ----

/// What the tests say this PC is, in place of the real one.
#[cfg(test)]
pub(crate) static TEST_PLACES: std::sync::Mutex<Option<Places>> = std::sync::Mutex::new(None);
/// Where the tests say the remembered folder is kept (none: nothing is).
#[cfg(test)]
pub(crate) static TEST_REMEMBERED: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);
/// Held for the whole of any test that sets one of the above.
#[cfg(test)]
pub(crate) static TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pagify-update-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A folder as the build script leaves one, at `dir`, with `version` published in it.
    fn publish(dir: &Path, version: &str, exe: bool) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("version.txt"), version).unwrap();
        if exe {
            std::fs::write(dir.join("Pagify.exe"), b"MZ").unwrap();
        }
        dir.to_path_buf()
    }

    /// A PC with a Dropbox at `root`, which its `info.json` names.
    fn pc_with_dropbox(tag: &str) -> (Places, PathBuf) {
        let base = scratch(&format!("{tag}-data"));
        let root = scratch(&format!("{tag}-dropbox"));
        std::fs::create_dir_all(base.join("Dropbox")).unwrap();
        let info = serde_json::json!({ "personal": { "path": root.to_string_lossy() } });
        std::fs::write(base.join("Dropbox").join("info.json"), info.to_string()).unwrap();
        (Places { app_data: vec![base], ..Default::default() }, root)
    }

    fn only(folder: &Path) -> Places {
        Places { fixed: vec![folder.to_path_buf()], ..Default::default() }
    }

    // ---- what a folder says ----

    #[test]
    fn a_newer_published_build_is_offered() {
        let dir = publish(&scratch("newer"), "0.1.9", true);
        assert_eq!(look("0.1.8", &only(&dir)), UpdateOutcome::Newer { version: "0.1.9".into(), folder: dir });
    }

    #[test]
    fn the_same_build_is_current_and_can_still_be_installed_anyway() {
        let dir = publish(&scratch("same"), "0.1.8", true);
        assert_eq!(look("0.1.8", &only(&dir)), UpdateOutcome::Current { version: "0.1.8".into(), folder: dir });
    }

    #[test]
    fn an_older_published_build_is_current_not_newer() {
        let dir = publish(&scratch("older"), "0.1.2", true);
        assert!(matches!(look("0.1.8", &only(&dir)), UpdateOutcome::Current { .. }));
    }

    #[test]
    fn a_version_with_no_exe_beside_it_is_not_offered() {
        // version.txt has synced; the 30 MB exe has not.
        let dir = publish(&scratch("noexe"), "0.1.9", false);
        assert_eq!(look("0.1.8", &only(&dir)), UpdateOutcome::Incomplete { folder: dir });
    }

    #[test]
    fn a_folder_with_no_version_is_incomplete_not_missing() {
        let dir = scratch("empty");
        assert_eq!(look("0.1.8", &only(&dir)), UpdateOutcome::Incomplete { folder: dir });
    }

    #[test]
    fn a_complete_folder_wins_over_an_incomplete_one_before_it() {
        let half = publish(&scratch("half"), "0.1.9", false);
        let whole = publish(&scratch("whole"), "0.1.9", true);
        let places = Places { fixed: vec![half, whole], ..Default::default() };
        assert!(matches!(look("0.1.8", &places), UpdateOutcome::Newer { .. }));
    }

    #[test]
    fn no_folder_at_all_says_where_it_looked_and_that_there_is_no_dropbox() {
        let nowhere = std::env::temp_dir().join("pagify-update-does-not-exist");
        let _ = std::fs::remove_dir_all(&nowhere);
        assert_eq!(
            look("0.1.8", &only(&nowhere)),
            UpdateOutcome::NoFolder { tried: vec![nowhere], searched: Vec::new() }
        );
    }

    // ---- finding Dropbox, and the folder in it ----

    #[test]
    fn dropbox_says_where_it_lives() {
        let dir = scratch("info");
        let info = dir.join("info.json");
        std::fs::write(&info, "\u{FEFF}{\"personal\":{\"path\":\"E:\\\\Work\\\\Dropbox\",\"host\":1},\"business\":{\"path\":\"F:\\\\Dropbox (HSI)\"}}").unwrap();
        let roots = dropbox_roots(&info);
        assert!(roots.contains(&PathBuf::from(r"E:\Work\Dropbox")) && roots.contains(&PathBuf::from(r"F:\Dropbox (HSI)")), "{roots:?}");
        assert!(dropbox_roots(&dir.join("not-there.json")).is_empty());
        std::fs::write(&info, "not json").unwrap();
        assert!(dropbox_roots(&info).is_empty());
    }

    #[test]
    fn a_dropbox_that_is_not_on_d_is_found_through_its_own_info() {
        // The reported failure: this PC's Dropbox is somewhere the build script's folder is not.
        let (places, root) = pc_with_dropbox("elsewhere");
        let places = Places { fixed: vec![PathBuf::from(r"Z:\no\such\drive")], ..places };
        publish(&root.join("YASEEN").join("pagify desktop"), "0.1.9", true);
        assert!(matches!(look("0.1.8", &places), UpdateOutcome::Newer { .. }));
    }

    #[test]
    fn a_folder_shared_into_the_dropbox_root_is_found_there() {
        let (places, root) = pc_with_dropbox("shared");
        publish(&root.join("PAGIFY Desktop"), "0.1.9", true);
        assert!(matches!(look("0.1.8", &places), UpdateOutcome::Newer { .. }));
    }

    #[test]
    fn a_folder_somewhere_else_in_the_dropbox_is_found_by_the_shallow_search() {
        let (places, root) = pc_with_dropbox("deeper");
        let dir = publish(&root.join("Team").join("PAGIFY Desktop"), "0.1.9", true);
        assert_eq!(look("0.1.8", &places), UpdateOutcome::Newer { version: "0.1.9".into(), folder: dir });
    }

    #[test]
    fn the_search_goes_two_levels_below_the_dropbox_root_and_no_further() {
        let (places, root) = pc_with_dropbox("toodeep");
        publish(&root.join("a").join("b").join("pagify desktop"), "0.1.9", true);
        match look("0.1.8", &places) {
            UpdateOutcome::NoFolder { searched, .. } => assert_eq!(searched, vec![root]),
            other => panic!("searched deeper than it should: {other:?}"),
        }
    }

    #[test]
    fn the_search_stops_when_its_budget_is_spent() {
        let root = scratch("budget");
        for i in 0..50 {
            std::fs::create_dir_all(root.join(format!("d{i:02}"))).unwrap();
        }
        let mut budget = 10;
        let looked = subdirs(&root, &mut budget);
        assert_eq!((looked.len(), budget), (10, 0));
        assert!(search_under(&root, &mut budget).is_empty(), "an empty budget still looked");
    }

    #[test]
    fn the_same_place_spelt_two_ways_is_looked_in_once() {
        let mut paths = vec![PathBuf::from(r"D:\Dropbox\YASEEN"), PathBuf::from(r"d:\dropbox\yaseen"), PathBuf::from(r"E:\x")];
        dedup_paths(&mut paths);
        assert_eq!(paths, vec![PathBuf::from(r"D:\Dropbox\YASEEN"), PathBuf::from(r"E:\x")]);
    }

    // ---- remembering ----

    /// The remembered folder lives in `file` for the length of a test.
    struct Remembering(std::sync::MutexGuard<'static, ()>);
    impl Remembering {
        fn in_file(file: PathBuf) -> Remembering {
            let guard = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
            *TEST_REMEMBERED.lock().unwrap() = Some(file);
            Remembering(guard)
        }
    }
    impl Drop for Remembering {
        fn drop(&mut self) {
            *TEST_REMEMBERED.lock().unwrap() = None;
        }
    }

    #[test]
    fn a_folder_that_worked_is_remembered_and_looked_in_first() {
        let file = scratch("memory").join("update-folder.txt");
        let _remembering = Remembering::in_file(file);
        assert_eq!(remembered(), None);

        let found = publish(&scratch("kept"), "0.1.9", true);
        remember(&found);
        assert_eq!(remembered(), Some(found.clone()));

        // On a PC whose usual places hold nothing, it is the remembered folder that answers.
        let places = Places { remembered: remembered(), fixed: vec![PathBuf::from(r"Z:\no\such\drive")], ..Default::default() };
        assert_eq!(look("0.1.8", &places), UpdateOutcome::Newer { version: "0.1.9".into(), folder: found });
    }

    #[test]
    fn a_remembered_folder_that_is_gone_is_forgotten() {
        let file = scratch("memory-gone").join("update-folder.txt");
        let _remembering = Remembering::in_file(file);
        let gone = scratch("gone-folder");
        remember(&gone);
        assert_eq!(remembered(), Some(gone.clone()));
        std::fs::remove_dir_all(&gone).unwrap();
        assert_eq!(remembered(), None);
    }
}
