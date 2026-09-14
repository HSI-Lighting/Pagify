//! Where Pagify keeps what is its own — never beside the document, never
//! beside the executable.
//!
//! Under the platform's own per-user configuration directory: a bundle should
//! be read-only, and on Windows it is in Program Files where it certainly is.
//! The recent list, the predefined texts, the drawn signatures and recorded
//! scripts all live here, and `state_dir` is the one answer to where.

use std::path::{Path, PathBuf};

/// Pagify's own directory under the platform's per-user configuration root,
/// or `None` where no such root can be found.
pub fn state_dir() -> Option<PathBuf> {
    let base = if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.map(|b| b.join("Pagify"))
}

/// A name that is only a file name: something typed as the name of a recording
/// or a signature, which is written to disk under [`state_dir`].
///
/// **A path is not a name.** `record ../../x` used to write `../../x.json`
/// relative to wherever the process was — found by audit. Anything that would
/// leave the directory is refused: separators, `..`, a leading dot, an empty
/// name, or characters a filesystem may not take.
pub fn file_name_only(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a name is needed".into());
    }
    if name == "." || name == ".." || name.starts_with('.') {
        return Err(format!("`{name}` is not a name for a file"));
    }
    if name.chars().any(|c| matches!(c, '/' | '\\' | ':' | '\0') || c.is_control()) {
        return Err(format!("`{name}` is not a name — a name has no path in it"));
    }
    if name.len() > 120 {
        return Err("that name is too long".into());
    }
    Ok(name.replace(' ', "-"))
}

/// Write a file that is Pagify's own: created with the directory it needs,
/// readable by the user alone where the platform can say so.
pub fn write_own(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Found by audit: `record ../../x` wrote outside the directory.
    #[test]
    fn a_name_with_a_path_in_it_is_refused() {
        for bad in ["../../x", "a/b", "a\\b", "..", ".", ".hidden", "", "   ", "c:d", "a\u{0}b"] {
            assert!(file_name_only(bad).is_err(), "{bad:?} was accepted");
        }
        assert_eq!(file_name_only("stamp every drawing").unwrap(), "stamp-every-drawing");
        assert_eq!(file_name_only("  script  ").unwrap(), "script");
    }

    #[test]
    fn the_state_directory_is_pagifys_own() {
        if let Some(dir) = state_dir() {
            assert_eq!(dir.file_name().and_then(|n| n.to_str()), Some("Pagify"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_own_file_is_readable_by_its_owner_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("pagify-own-{}", std::process::id()));
        let path = dir.join("nested").join("thing.json");
        write_own(&path, b"{}").expect("write");
        let mode = std::fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
