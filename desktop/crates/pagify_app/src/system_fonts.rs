//! Fonts installed on this machine, for the text-edit font picker.
//!
//! A plain filesystem scan of the Windows font directories plus this crate's
//! own name-table read (`pdf_core::pdf::embed::face_name`) — the same reader
//! already trusted for the bundled/`outlinedfont add`-ed fonts, so a system
//! font and a manually-added one are found and embedded identically once
//! picked. No GDI/DirectWrite: those fonts are just files on disk.
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

/// The directories Windows actually keeps fonts in: machine-wide, and the
/// per-user "installed for me only" one Windows 10+ writes to.
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

/// Every `.ttf`/`.otf` Windows has installed, named by its own PostScript or
/// full name and deduplicated by that name (the same face often exists in
/// both directories, or under more than one file).
///
/// A slow, one-time scan — reads and parses every font file it finds, which
/// on a few hundred installed fonts is a real, multi-second cost. Call it
/// once and cache the result; nothing here is fit to run per frame.
pub fn list() -> Vec<SystemFont> {
    let mut found = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for dir in font_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_font = matches!(
                path.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_lowercase())
                    .as_deref(),
                Some("ttf") | Some("otf")
            );
            if !is_font {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let Some(name) = pdf_core::pdf::embed::face_name(&bytes) else { continue };
            if seen.insert(name.clone()) {
                found.push(SystemFont { name, path });
            }
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}
