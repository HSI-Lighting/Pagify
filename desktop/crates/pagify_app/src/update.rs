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
//!
//! **And GitHub** (the owner's ask, 2026-11-06: "the app has to check where the latest
//! release is, github or dropbox"). [`look_everywhere`] also reads Pagify's feed in the
//! HSI release repo, as every HSI app does, and where GitHub has the newer build it is
//! downloaded, checked against Pagify's update key and unpacked before it is offered.
//! Pagify itself still opens no connection: Windows' own `curl.exe` fetches the two
//! files, and `tar.exe` unpacks the zip (`tools/no_sockets.sh`).

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
    /// GitHub has `version`, newer than this build and than any in Dropbox: downloaded,
    /// its signature checked, and unpacked into `folder`, ready to install.
    Downloaded { version: String, folder: PathBuf },
    /// GitHub's newest is `version` — this build's or older — and no folder on this PC
    /// holds a build.
    Released { version: String },
    /// GitHub has `version`, newer than this build, but it could not be downloaded or
    /// its signature did not hold: `why`.
    Unfetched { version: String, why: String },
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
            Self::Downloaded { version, folder } => format!("current={current} github={version} downloaded folder={}", folder.display()),
            Self::Released { version } => format!("current={current} github={version} not-newer"),
            Self::Unfetched { version, why } => format!("current={current} github={version} unfetched: {why}"),
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

// ---- GitHub ----

/// The newest build Pagify's feed names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Release {
    pub version: String,
    /// The zip, an asset of the release `pagify-v<version>`.
    pub url: String,
    /// Its signature, as the feed carries it.
    pub signature: String,
}

/// Pagify's feed in the HSI release repo — the `<app>/latest.json` every HSI app updates
/// from (that repo's README). Read from raw.githubusercontent.com, never GitHub's API:
/// the API allows an address 60 calls an hour, and the office's was found spent
/// (2026-11-06).
#[cfg(windows)]
const FEED_URL: &str = "https://raw.githubusercontent.com/HSI-Lighting/release/main/pagify/latest.json";

/// The public half of Pagify's own update key (`examples/make_update_key.rs`, made
/// 2026-11-06; the private half is kept on the PC that publishes, in
/// `E:\KEYS\Pagify`), as Tauri writes one: base64 of minisign's public-key text. A
/// build from GitHub is unpacked only if this key signed it. The owner chose the HSI
/// apps' shared key first; it is not on the PC Pagify is published from. Key id
/// `B318C8916BCECD5B`.
const UPDATE_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEIzMThDODkxNkJDRUNENUIKUldSYnpjNXJrY2dZczJJMHB3NFlBREtpQ2tCYXZDV05GQklEUU5NNm1IQXJZY2FTelBYUldHbEUK";

/// What Windows' curl is always told: HTTPS only, redirects included (a release asset
/// is served from another host), and a network that blocks certificate revocation
/// lists is not a failed check.
#[cfg(windows)]
const CURL: [&str; 9] =
    ["--fail", "--silent", "--show-error", "--location", "--proto", "=https", "--proto-redir", "=https", "--ssl-revoke-best-effort"];

/// Look in Dropbox ([`look`]) and on GitHub, and take whichever holds the newer build
/// ([`choose`]). With it, a line for the session log saying what GitHub answered.
pub(crate) fn look_everywhere(current: &str, places: &Places) -> (UpdateOutcome, String) {
    let local = look(current, places);
    let online = latest_release();
    let github = match &online {
        Ok(release) => format!("github-feed={}", release.version),
        Err(why) => format!("github-feed-unread: {why}"),
    };
    (choose(current, local, online.ok(), fetch), github)
}

/// Dropbox's answer, or GitHub's where GitHub has the newer build. A build is fetched
/// only when it is newer than this one and than Dropbox's: a tie goes to Dropbox, which
/// is already on this PC. A GitHub build that cannot be fetched leaves a newer one in
/// Dropbox to be offered, and is otherwise said.
fn choose(
    current: &str,
    local: UpdateOutcome,
    online: Option<Release>,
    fetch: impl FnOnce(&Release) -> Result<PathBuf, String>,
) -> UpdateOutcome {
    use UpdateOutcome::*;
    let Some(release) = online else { return local };
    let in_dropbox = match &local {
        Newer { version, .. } | Current { version, .. } => Some(version.as_str()),
        _ => None,
    };
    let newest = crate::newer_version(current, &release.version)
        && in_dropbox.is_none_or(|version| crate::newer_version(version, &release.version));
    if newest {
        return match fetch(&release) {
            Ok(folder) => Downloaded { version: release.version, folder },
            Err(_) if matches!(local, Newer { .. }) => local,
            Err(why) => Unfetched { version: release.version, why },
        };
    }
    match local {
        Newer { .. } | Current { .. } => local,
        _ => Released { version: release.version },
    }
}

/// The release the feed names for Windows — `None` for a feed without one, or for
/// what is not a feed at all.
fn read_feed(text: &str) -> Option<Release> {
    let feed: serde_json::Value = serde_json::from_str(text.trim_start_matches('\u{FEFF}')).ok()?;
    let windows = feed.get("platforms")?.get("windows-x86_64")?;
    let field = |value: &serde_json::Value, key: &str| value.get(key)?.as_str().map(str::to_string);
    Some(Release { version: field(&feed, "version")?, url: field(windows, "url")?, signature: field(windows, "signature")? })
}

/// What the feed says now. No test reaches GitHub, but the one run by hand.
fn latest_release() -> Result<Release, String> {
    #[cfg(test)]
    let text = String::from("no network in tests");
    #[cfg(all(windows, not(test)))]
    let text = feed()?;
    #[cfg(all(not(windows), not(test)))]
    let text = String::from("builds on GitHub are for Windows");
    read_feed(&text).ok_or_else(|| format!("not a feed with a Windows build: {}", text.chars().take(80).collect::<String>()))
}

#[cfg(windows)]
fn feed() -> Result<String, String> {
    let mut args: Vec<std::ffi::OsString> = CURL.iter().map(Into::into).collect();
    args.extend(["--max-time", "20", "--max-filesize", "1000000", FEED_URL].map(Into::into));
    Ok(String::from_utf8_lossy(&windows_tool("curl.exe", &args)?).into_owned())
}

/// Fetch `release` into a folder of its own under the temp folder — once: a zip
/// already there that holds is not fetched again — and unpack it.
fn fetch(release: &Release) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("pagify-download-{}", release.version));
    let zip = dir.join(zip_name(&release.version));
    let have = std::fs::read(&zip).is_ok_and(|bytes| signed_by(UPDATE_KEY, &bytes, &release.signature, &zip_name(&release.version)).is_ok());
    if !have {
        download(&release.url, &dir, &zip)?;
    }
    unpack_signed(UPDATE_KEY, release, &zip, &dir)
}

#[cfg(windows)]
fn download(url: &str, dir: &Path, zip: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut args: Vec<std::ffi::OsString> = CURL.iter().map(Into::into).collect();
    args.extend(["--max-time", "900", "--max-filesize", "500000000", "--output"].map(Into::into));
    args.extend([zip.as_os_str().to_owned(), url.into()]);
    windows_tool("curl.exe", &args).map(drop)
}

#[cfg(not(windows))]
fn download(_url: &str, _dir: &Path, _zip: &Path) -> Result<(), String> {
    Err("builds on GitHub are for Windows".into())
}

/// Unpack `zip` into `dir` if `key` signed it as `release`'s zip. One that does not
/// hold is deleted, never unpacked. The folder holding its `Pagify.exe`, for the
/// update script to install from as it would from Dropbox.
fn unpack_signed(key: &str, release: &Release, zip: &Path, dir: &Path) -> Result<PathBuf, String> {
    let bytes = std::fs::read(zip).map_err(|e| format!("{}: {e}", zip.display()))?;
    if let Err(why) = signed_by(key, &bytes, &release.signature, &zip_name(&release.version)) {
        let _ = std::fs::remove_file(zip);
        return Err(why);
    }
    unpack(zip, dir)?;
    let build = dir.join("Pagify");
    if build.join("Pagify.exe").is_file() {
        Ok(build)
    } else {
        Err(format!("{} holds no Pagify\\Pagify.exe", zip.display()))
    }
}

#[cfg(windows)]
fn unpack(zip: &Path, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    windows_tool("tar.exe", &["-xf".into(), zip.into(), "-C".into(), dir.into()]).map(drop)
}

#[cfg(not(windows))]
fn unpack(_zip: &Path, _dir: &Path) -> Result<(), String> {
    Err("builds on GitHub are for Windows".into())
}

/// Whether the holder of `key` signed `bytes` as the file `name`. `key` and
/// `signature` are as Tauri writes them: base64 of minisign's own text. The name is
/// in the signed comment, so an older build signed in its day cannot be passed off
/// under a newer number — installed, and then offered again forever.
fn signed_by(key: &str, bytes: &[u8], signature: &str, name: &str) -> Result<(), String> {
    let text = |base64: &str| unbase64(base64).and_then(|bytes| String::from_utf8(bytes).ok());
    let key = text(key)
        .and_then(|key| minisign_verify::PublicKey::decode(&key).ok())
        .ok_or("the update key built into Pagify is not a key")?;
    let signature = text(signature)
        .and_then(|signature| minisign_verify::Signature::decode(&signature).ok())
        .ok_or("the feed's signature cannot be read")?;
    key.verify(bytes, &signature, false).map_err(|_| "the download is not signed with Pagify's update key")?;
    if signature.trusted_comment().split('\t').any(|part| part == format!("file:{name}")) {
        Ok(())
    } else {
        Err(format!("the download is signed as another file than {name}"))
    }
}

/// Standard base64, padding and line breaks allowed — all a key and a signature need,
/// which is not worth a crate.
fn unbase64(text: &str) -> Option<Vec<u8>> {
    const DIGITS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let (mut bytes, mut bits, mut held) = (Vec::new(), 0u32, 0u32);
    for c in text.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        held = held << 6 | DIGITS.iter().position(|&d| d == c)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((held >> bits) as u8);
        }
    }
    Some(bytes)
}

/// The zip's name, which its signature names too.
fn zip_name(version: &str) -> String {
    format!("pagify_{version}_windows.zip")
}

/// Run one of Windows' own tools — `curl.exe`, `tar.exe` — from System32, never
/// whatever of that name is first on the PATH (Git's GNU tar cannot read a zip), and
/// with no console window. What it printed, or why it failed.
#[cfg(windows)]
fn windows_tool(tool: &str, args: &[std::ffi::OsString]) -> Result<Vec<u8>, String> {
    use std::os::windows::process::CommandExt;
    let system = std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    let out = std::process::Command::new(system.join("System32").join(tool))
        .args(args)
        .stdin(std::process::Stdio::null())
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .map_err(|e| format!("{tool}: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!("{tool}: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
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

    // ---- GitHub: the feed ----

    #[test]
    fn the_feed_names_the_windows_build() {
        let feed = r#"{ "version": "0.1.118", "notes": "Pagify 0.1.118", "pub_date": "2026-11-06T09:00:00Z",
            "platforms": { "windows-x86_64": { "signature": "c2ln",
                "url": "https://github.com/HSI-Lighting/release/releases/download/pagify-v0.1.118/pagify_0.1.118_windows.zip" } } }"#;
        assert_eq!(
            read_feed(feed),
            Some(Release {
                version: "0.1.118".into(),
                url: "https://github.com/HSI-Lighting/release/releases/download/pagify-v0.1.118/pagify_0.1.118_windows.zip".into(),
                signature: "c2ln".into(),
            })
        );
    }

    #[test]
    fn a_feed_with_no_windows_build_or_no_feed_at_all_offers_nothing() {
        assert_eq!(read_feed(r#"{"version":"0.1.118","platforms":{"android-aarch64":{"signature":"s","url":"u"}}}"#), None);
        // What raw.githubusercontent.com answers for a feed not published yet.
        assert_eq!(read_feed("404: Not Found"), None);
    }

    // ---- GitHub: the signature ----

    fn b64(bytes: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    /// A key pair as `tauri signer generate` writes one: the public half as base64 of
    /// minisign's own text.
    fn test_key() -> (String, minisign::SecretKey) {
        let minisign::KeyPair { pk, sk } = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        (b64(pk.to_box().unwrap().to_string().as_bytes()), sk)
    }

    /// `bytes` signed as `tauri signer sign` signs the file `name`.
    fn sign(sk: &minisign::SecretKey, bytes: &[u8], name: &str) -> String {
        let comment = format!("timestamp:1792489463\tfile:{name}");
        let signature = minisign::sign(None, sk, std::io::Cursor::new(bytes), Some(&comment), Some("signature from tauri secret key")).unwrap();
        b64(signature.to_string().as_bytes())
    }

    #[test]
    fn base64_reads_what_the_base64_crate_writes() {
        for text in ["", "a", "ab", "abc", "untrusted comment: minisign public key\nRWQ+/x=="] {
            assert_eq!(unbase64(&b64(text.as_bytes())), Some(text.as_bytes().to_vec()), "{text:?}");
        }
        assert_eq!(unbase64("not*base64"), None);
    }

    #[test]
    fn a_build_signed_with_the_key_holds() {
        let (key, sk) = test_key();
        let zip = b"PK a build";
        assert_eq!(signed_by(&key, zip, &sign(&sk, zip, "pagify_0.1.118_windows.zip"), "pagify_0.1.118_windows.zip"), Ok(()));
    }

    #[test]
    fn a_changed_byte_or_another_key_does_not_hold() {
        let (key, sk) = test_key();
        let (other_key, _) = test_key();
        let signature = sign(&sk, b"PK a build", "pagify_0.1.118_windows.zip");
        assert!(signed_by(&key, b"PK a bui1d", &signature, "pagify_0.1.118_windows.zip").is_err());
        assert!(signed_by(&other_key, b"PK a build", &signature, "pagify_0.1.118_windows.zip").is_err());
    }

    #[test]
    fn a_build_signed_as_another_version_does_not_hold() {
        // An older signed build put under a newer number would be installed, and then offered again forever.
        let (key, sk) = test_key();
        let signature = sign(&sk, b"PK old", "pagify_0.1.117_windows.zip");
        assert!(signed_by(&key, b"PK old", &signature, "pagify_0.1.118_windows.zip").is_err());
    }

    #[test]
    fn what_the_publishing_tool_signs_with_a_key_as_tauri_writes_one_holds() {
        let minisign::KeyPair { pk, sk } = minisign::KeyPair::generate_encrypted_keypair(Some("a password".into())).unwrap();
        // As `tauri signer generate` writes it: base64 of minisign's text, sealed with the password.
        let key_file = b64(sk.to_box(None).unwrap().to_string().as_bytes());
        let zip = scratch("signing").join("pagify_0.1.118_windows.zip");
        std::fs::write(&zip, b"PK a build").unwrap();
        let signature = crate::sign_update::sign(&key_file, "a password".into(), &zip).unwrap();
        let key = b64(pk.to_box().unwrap().to_string().as_bytes());
        assert_eq!(signed_by(&key, b"PK a build", &signature, "pagify_0.1.118_windows.zip"), Ok(()));
        assert!(crate::sign_update::sign(&key_file, "not it".into(), &zip).is_err(), "signed with the wrong password");
    }

    /// By hand, after a publish — it reaches GitHub: the feed as published, the zip it
    /// names downloaded, checked against the key built in and unpacked, as an installed
    /// copy does it (`cargo test -p pagify_app --bins the_published_build -- --ignored`).
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn the_published_build_downloads_holds_and_unpacks() {
        let release = read_feed(&feed().unwrap()).expect("the published feed names a Windows build");
        let folder = fetch(&release).unwrap();
        assert!(folder.join("Pagify.exe").is_file() && folder.join("pdfium.dll").is_file(), "{}", folder.display());
    }

    #[test]
    fn the_key_built_in_is_a_public_key() {
        let key = unbase64(UPDATE_KEY).and_then(|text| String::from_utf8(text).ok());
        assert!(
            key.is_some_and(|key| minisign_verify::PublicKey::decode(&key).is_ok()),
            "UPDATE_KEY is not a public key"
        );
    }

    // ---- GitHub or Dropbox ----

    fn release(version: &str) -> Release {
        Release { version: version.into(), url: String::new(), signature: String::new() }
    }

    fn newer_in_dropbox(version: &str) -> UpdateOutcome {
        UpdateOutcome::Newer { version: version.into(), folder: PathBuf::from(r"D:\Dropbox\YASEEN\pagify desktop") }
    }

    fn downloaded(release: &Release) -> Result<PathBuf, String> {
        Ok(PathBuf::from(format!(r"T:\pagify-download-{}\Pagify", release.version)))
    }

    fn not_downloaded(_: &Release) -> Result<PathBuf, String> {
        panic!("downloaded a build that is not the newest")
    }

    #[test]
    fn a_github_build_newer_than_dropbox_is_downloaded_and_offered() {
        assert_eq!(
            choose("0.1.8", newer_in_dropbox("0.1.9"), Some(release("0.1.10")), downloaded),
            UpdateOutcome::Downloaded { version: "0.1.10".into(), folder: PathBuf::from(r"T:\pagify-download-0.1.10\Pagify") }
        );
    }

    #[test]
    fn a_tie_or_an_older_github_build_leaves_dropbox_and_downloads_nothing() {
        for github in ["0.1.9", "0.1.7"] {
            assert_eq!(choose("0.1.8", newer_in_dropbox("0.1.9"), Some(release(github)), not_downloaded), newer_in_dropbox("0.1.9"));
        }
        let current = UpdateOutcome::Current { version: "0.1.8".into(), folder: PathBuf::from(r"D:\x") };
        assert_eq!(choose("0.1.8", current.clone(), Some(release("0.1.8")), not_downloaded), current);
    }

    #[test]
    fn with_no_build_on_this_pc_github_decides() {
        let none = UpdateOutcome::NoFolder { tried: vec![], searched: vec![] };
        assert!(matches!(choose("0.1.8", none.clone(), Some(release("0.1.9")), downloaded), UpdateOutcome::Downloaded { .. }));
        assert_eq!(choose("0.1.8", none, Some(release("0.1.8")), not_downloaded), UpdateOutcome::Released { version: "0.1.8".into() });
        let syncing = UpdateOutcome::Incomplete { folder: PathBuf::from(r"D:\x") };
        assert_eq!(choose("0.1.8", syncing, Some(release("0.1.7")), not_downloaded), UpdateOutcome::Released { version: "0.1.7".into() });
    }

    #[test]
    fn github_out_of_reach_leaves_the_dropbox_answer() {
        let none = UpdateOutcome::NoFolder { tried: vec![PathBuf::from(r"D:\x")], searched: vec![] };
        assert_eq!(choose("0.1.8", none.clone(), None, not_downloaded), none);
        assert_eq!(choose("0.1.8", newer_in_dropbox("0.1.9"), None, not_downloaded), newer_in_dropbox("0.1.9"));
    }

    #[test]
    fn a_download_that_fails_is_said_unless_dropbox_has_a_newer_build() {
        let failed = |_: &Release| Err("curl.exe: Could not resolve host".to_string());
        let none = UpdateOutcome::NoFolder { tried: vec![], searched: vec![] };
        assert_eq!(
            choose("0.1.8", none, Some(release("0.1.10")), failed),
            UpdateOutcome::Unfetched { version: "0.1.10".into(), why: "curl.exe: Could not resolve host".into() }
        );
        assert_eq!(choose("0.1.8", newer_in_dropbox("0.1.9"), Some(release("0.1.10")), failed), newer_in_dropbox("0.1.9"));
    }

    #[test]
    fn a_downloaded_build_is_not_remembered_as_the_update_folder() {
        assert_eq!(UpdateOutcome::Downloaded { version: "0.1.9".into(), folder: PathBuf::from(r"T:\x") }.found_in(), None);
    }

    // ---- GitHub: unpacking ----

    /// A zip as `build.ps1` makes one: a `Pagify` folder, signed as `version`'s.
    #[cfg(windows)]
    fn signed_zip(tag: &str, version: &str) -> (PathBuf, String, Release) {
        let dir = scratch(tag);
        let build = dir.join("src").join("Pagify");
        std::fs::create_dir_all(&build).unwrap();
        std::fs::write(build.join("Pagify.exe"), b"MZ new").unwrap();
        std::fs::write(build.join("pdfium.dll"), b"MZ dll").unwrap();
        let zip = dir.join(zip_name(version));
        let os = |s: &str| std::ffi::OsString::from(s);
        windows_tool("tar.exe", &[os("-a"), os("-cf"), zip.clone().into(), os("-C"), dir.join("src").into(), os("Pagify")]).unwrap();
        let (key, sk) = test_key();
        let signature = sign(&sk, &std::fs::read(&zip).unwrap(), &zip_name(version));
        (zip, key, Release { version: version.into(), url: String::new(), signature })
    }

    #[cfg(windows)]
    #[test]
    fn a_signed_zip_is_unpacked_into_a_folder_the_update_script_installs_from() {
        let (zip, key, release) = signed_zip("unpack", "0.1.9");
        let out = zip.with_file_name("out");
        let folder = unpack_signed(&key, &release, &zip, &out).unwrap();
        assert_eq!(folder, out.join("Pagify"));
        assert_eq!(std::fs::read(folder.join("Pagify.exe")).unwrap(), b"MZ new");
        assert!(folder.join("pdfium.dll").is_file());
    }

    #[cfg(windows)]
    #[test]
    fn a_zip_whose_signature_does_not_hold_is_deleted_and_not_unpacked() {
        let (zip, _, release) = signed_zip("tampered", "0.1.9");
        let (other_key, _) = test_key();
        let out = zip.with_file_name("out");
        assert!(unpack_signed(&other_key, &release, &zip, &out).is_err());
        assert!(!zip.exists(), "a zip that failed its check was kept");
        assert!(!out.join("Pagify").exists(), "a zip that failed its check was unpacked");
    }
}
