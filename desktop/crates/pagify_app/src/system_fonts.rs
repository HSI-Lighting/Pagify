//! Fonts installed on this machine, for the text-edit font picker.
//!
//! A plain filesystem scan of the platform's font directories plus this crate's
//! own name-table read (`pdf_core::pdf::embed::face_name`) — the same reader
//! already trusted for the bundled/`outlinedfont add`-ed fonts, so a system
//! font and a manually-added one are found and embedded identically once
//! picked. No GDI/DirectWrite/CoreText/Fontconfig: those fonts are just files
//! on disk.
//!
//! `.ttc` collections are skipped — `face_name` parses index 0 of whatever it
//! is given, and a collection's header at that offset is not a single font,
//! so it quietly finds no name and the file is left out. Worth widening if a
//! wanted font turns out to live only in one.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct SystemFont {
    pub name: String,
    pub path: PathBuf,
}

/// How many directory levels the scan may descend. Windows keeps its fonts
/// flat; macOS has one level of supplemental folders; Linux distributions
/// nest them two or three deep (`/usr/share/fonts/truetype/dejavu/…`).
const MAX_DEPTH: usize = 4;

/// The directories this platform actually keeps fonts in.
///
/// **All three desktop platforms, not just Windows.** The picker used to read
/// `WINDIR`/`LOCALAPPDATA` only, so on macOS and Linux it was simply empty and
/// said nothing — a silent missing feature. The directories here are the
/// standard per-machine and per-user ones; a distribution that puts fonts
/// somewhere else can still be served by picking a file explicitly.
#[cfg(target_os = "windows")]
fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(windir) = std::env::var("WINDIR") {
        dirs.push(PathBuf::from(windir).join("Fonts"));
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join("Microsoft").join("Windows").join("Fonts"));
    }
    dirs
}

#[cfg(target_os = "macos")]
fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/System/Library/Fonts"),
        PathBuf::from("/System/Library/Fonts/Supplemental"),
        PathBuf::from("/Library/Fonts"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join("Library").join("Fonts"));
    }
    dirs
}

#[cfg(all(unix, not(target_os = "macos")))]
fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join(".local").join("share").join("fonts"));
        dirs.push(home.join(".fonts"));
    }
    dirs
}

/// Every `.ttf`/`.otf` under `dir`, descending at most [`MAX_DEPTH`] levels.
fn collect_font_files(dir: &std::path::Path, depth: usize, into: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_font_files(&path, depth + 1, into);
            continue;
        }
        let is_font = matches!(
            path.extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
                .as_deref(),
            Some("ttf") | Some("otf")
        );
        if is_font {
            into.push(path);
        }
    }
}

/// Every `.ttf`/`.otf` this machine has installed, named by its own PostScript
/// or full name and deduplicated by that name (the same face often exists in
/// both directories, or under more than one file).
///
/// A slow, one-time scan — reads and parses every font file it finds, which
/// on a few hundred installed fonts is a real, multi-second cost. Call it
/// once and cache the result; nothing here is fit to run per frame.
pub fn list() -> Vec<SystemFont> {
    let mut paths = Vec::new();
    for dir in font_dirs() {
        collect_font_files(&dir, 0, &mut paths);
    }
    // Two user directories can hold the same file; read each path once.
    paths.sort();
    paths.dedup();

    let mut found = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for path in paths {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Some(name) = pdf_core::pdf::embed::face_name(&bytes) else { continue };
        if seen.insert(name.clone()) {
            found.push(SystemFont { name, path });
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

#[cfg(test)]
mod tests {
    /// Whatever the machine has, the picker gets a stable list: sorted by name
    /// and with no face listed twice. (An empty list is legitimate — a bare CI
    /// container may have no fonts — so only the ordering and uniqueness are
    /// asserted here.)
    #[test]
    fn the_scan_returns_sorted_unique_names() {
        let fonts = super::list();
        let names: Vec<&str> = fonts.iter().map(|font| font.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "font names are not sorted");
        let unique: std::collections::HashSet<&&str> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "a font name is listed twice");
    }
}
