//! Pagify, registered with Windows as an app that opens PDF files.
//!
//! **Reported from use (2026-11-12): "i cant set pagify as the default pdf opening
//! app".** Windows' "Select an app to open this .pdf file" offered Pagify only
//! **Just once**, named "Pagify.exe" with no icon of its own. Nothing had registered
//! Pagify: the one entry Windows had was the bare `Applications\Pagify.exe` it makes
//! when an exe is picked by hand (pointing at a development build). An app can be
//! made the default only through a type it registers for `.pdf` (a ProgID), and is
//! listed in Settings ▸ Default apps only through the capabilities it registers.
//!
//! Pagify registers both, for the person running it, at every start: a copy updated
//! by Pagify itself never runs the installer again, and the paths follow wherever
//! this Pagify runs from. Nothing is written when it is already so. Windows does not
//! let a program make itself the default — the person still picks **Always** once.
//! The uninstaller takes the entries away (`pagify.iss`, `[Registry]`).

use std::path::Path;

/// The type Pagify registers PDFs under.
const PROG_ID: &str = "Pagify.Document";
/// Where Pagify says what it can open, for Settings ▸ Default apps.
const CAPABILITIES: &str = r"Software\HSI Lighting\Pagify\Capabilities";

/// What makes it so, under `HKEY_CURRENT_USER`: (key, value name — empty for the
/// key's default — and its text).
fn entries(exe: &Path) -> Vec<(String, &'static str, String)> {
    let exe = exe.to_string_lossy();
    let open = format!("\"{exe}\" \"%1\"");
    let icon = format!("{exe},0");
    let classes = r"Software\Classes";
    vec![
        (format!(r"{classes}\{PROG_ID}"), "", "PDF Document".into()),
        (format!(r"{classes}\{PROG_ID}\DefaultIcon"), "", icon.clone()),
        (format!(r"{classes}\{PROG_ID}\Application"), "ApplicationName", "Pagify".into()),
        (format!(r"{classes}\{PROG_ID}\Application"), "ApplicationIcon", icon),
        (format!(r"{classes}\{PROG_ID}\shell\open\command"), "", open.clone()),
        (format!(r"{classes}\.pdf\OpenWithProgids"), PROG_ID, String::new()),
        // The entry Windows made by itself when the exe was picked by hand: named,
        // and pointed at this Pagify.
        (format!(r"{classes}\Applications\Pagify.exe"), "FriendlyAppName", "Pagify".into()),
        (format!(r"{classes}\Applications\Pagify.exe\SupportedTypes"), ".pdf", String::new()),
        (format!(r"{classes}\Applications\Pagify.exe\shell\open\command"), "", open),
        (CAPABILITIES.into(), "ApplicationName", "Pagify".into()),
        (CAPABILITIES.into(), "ApplicationDescription", "Read, edit, sign and print PDF documents.".into()),
        (format!(r"{CAPABILITIES}\FileAssociations"), ".pdf", PROG_ID.into()),
        (r"Software\RegisteredApplications".into(), "Pagify", CAPABILITIES.into()),
    ]
}

/// Register the Pagify that is running, and tell Explorer when anything changed.
#[cfg(windows)]
pub(crate) fn register() {
    let Ok(exe) = std::env::current_exe() else { return };
    if write(&entries(&exe), "") {
        registry::associations_changed();
    }
}

#[cfg(not(windows))]
pub(crate) fn register() {}

/// Write each entry, under `prefix` (only a test sets one), that is not already so.
/// Whether any was not. One that cannot be written is left: a Pagify that cannot
/// register still opens PDFs.
#[cfg(windows)]
fn write(entries: &[(String, &str, String)], prefix: &str) -> bool {
    let mut changed = false;
    for (key, name, text) in entries {
        let key = format!("{prefix}{key}");
        if registry::get(&key, name).as_deref() != Some(text.as_str()) {
            changed |= registry::set(&key, name, text);
        }
    }
    changed
}

/// The few registry calls this needs, under `HKEY_CURRENT_USER`, as text values.
#[cfg(windows)]
mod registry {
    use std::ffi::c_void;

    type Key = *mut c_void;
    /// `HKEY_CURRENT_USER`: `(HKEY)(ULONG_PTR)(LONG)0x80000001`, sign-extended.
    const CURRENT_USER: Key = 0x8000_0001u32 as i32 as isize as Key;
    const KEY_QUERY_VALUE: u32 = 0x0001;
    const KEY_SET_VALUE: u32 = 0x0002;
    const REG_SZ: u32 = 1;

    #[link(name = "advapi32")]
    extern "system" {
        fn RegCreateKeyExW(
            key: Key,
            subkey: *const u16,
            reserved: u32,
            class: *const u16,
            options: u32,
            access: u32,
            security: *const c_void,
            result: *mut Key,
            disposition: *mut u32,
        ) -> i32;
        fn RegOpenKeyExW(key: Key, subkey: *const u16, options: u32, access: u32, result: *mut Key) -> i32;
        fn RegQueryValueExW(key: Key, name: *const u16, reserved: *mut u32, kind: *mut u32, data: *mut u8, size: *mut u32) -> i32;
        fn RegSetValueExW(key: Key, name: *const u16, reserved: u32, kind: u32, data: *const u8, size: u32) -> i32;
        fn RegCloseKey(key: Key) -> i32;
        #[cfg(test)]
        fn RegDeleteTreeW(key: Key, subkey: *const u16) -> i32;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn SHChangeNotify(event: i32, flags: u32, item1: *const c_void, item2: *const c_void);
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }

    /// The text value `name` of `key`, if it is one.
    pub(super) fn get(key: &str, name: &str) -> Option<String> {
        let mut handle: Key = std::ptr::null_mut();
        if unsafe { RegOpenKeyExW(CURRENT_USER, wide(key).as_ptr(), 0, KEY_QUERY_VALUE, &mut handle) } != 0 {
            return None;
        }
        let mut data = vec![0u16; 2048];
        let (mut kind, mut size) = (0u32, (data.len() * 2) as u32);
        let read = unsafe {
            RegQueryValueExW(handle, wide(name).as_ptr(), std::ptr::null_mut(), &mut kind, data.as_mut_ptr().cast(), &mut size)
        };
        unsafe { RegCloseKey(handle) };
        if read != 0 || kind != REG_SZ {
            return None;
        }
        data.truncate(size as usize / 2);
        while data.last() == Some(&0) {
            data.pop();
        }
        Some(String::from_utf16_lossy(&data))
    }

    /// Set the text value `name` of `key`, making the key if need be. Whether it was set.
    pub(super) fn set(key: &str, name: &str, text: &str) -> bool {
        let mut handle: Key = std::ptr::null_mut();
        let made = unsafe {
            RegCreateKeyExW(
                CURRENT_USER,
                wide(key).as_ptr(),
                0,
                std::ptr::null(),
                0,
                KEY_QUERY_VALUE | KEY_SET_VALUE,
                std::ptr::null(),
                &mut handle,
                std::ptr::null_mut(),
            )
        };
        if made != 0 {
            return false;
        }
        let data = wide(text);
        let set = unsafe { RegSetValueExW(handle, wide(name).as_ptr(), 0, REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32) };
        unsafe { RegCloseKey(handle) };
        set == 0
    }

    /// Tell Explorer that what opens which file has changed.
    pub(super) fn associations_changed() {
        const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
        const SHCNF_IDLIST: u32 = 0;
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, std::ptr::null(), std::ptr::null()) };
    }

    #[cfg(test)]
    pub(super) fn delete_tree(key: &str) {
        unsafe { RegDeleteTreeW(CURRENT_USER, wide(key).as_ptr()) };
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// Written under a key of the test's own, never over this PC's real entries.
    struct Scratch(String);
    impl Scratch {
        fn new(tag: &str) -> Scratch {
            Scratch(format!(r"Software\PagifyTest-{tag}-{}", std::process::id()))
        }
        fn prefix(&self) -> String {
            format!(r"{}\", self.0)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            registry::delete_tree(&self.0);
        }
    }

    #[test]
    fn pagify_registers_a_pdf_type_that_opens_it_and_its_capabilities() {
        let scratch = Scratch::new("register");
        let exe = Path::new(r"C:\Users\Someone\AppData\Local\Programs\Pagify\Pagify.exe");
        assert!(write(&entries(exe), &scratch.prefix()), "nothing was written");

        let at = |key: &str, name: &str| registry::get(&format!("{}{key}", scratch.prefix()), name);
        let open = Some(format!("\"{}\" \"%1\"", exe.display()));
        assert_eq!(at(r"Software\Classes\.pdf\OpenWithProgids", "Pagify.Document"), Some(String::new()));
        assert_eq!(at(r"Software\Classes\Pagify.Document\shell\open\command", ""), open);
        assert_eq!(at(r"Software\Classes\Pagify.Document\Application", "ApplicationName").as_deref(), Some("Pagify"));
        assert_eq!(at(r"Software\Classes\Applications\Pagify.exe\shell\open\command", ""), open);
        assert_eq!(at(r"Software\Classes\Applications\Pagify.exe", "FriendlyAppName").as_deref(), Some("Pagify"));
        assert_eq!(at(r"Software\HSI Lighting\Pagify\Capabilities\FileAssociations", ".pdf").as_deref(), Some("Pagify.Document"));
        assert_eq!(at(r"Software\RegisteredApplications", "Pagify").as_deref(), Some(CAPABILITIES));
    }

    /// By hand, on a PC with Pagify installed for this user: register the installed
    /// copy as it registers itself at every start, without starting it — no window
    /// (`cargo test --release -p pagify_app --bins registers_the_installed -- --ignored`).
    #[test]
    #[ignore]
    fn registers_the_installed_pagify_here() {
        let exe = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join(r"Programs\Pagify\Pagify.exe");
        assert!(exe.is_file(), "no Pagify installed at {}", exe.display());
        write(&entries(&exe), "");
        registry::associations_changed();
    }

    /// Every start registers again; only a change is written, and only then is
    /// Explorer told.
    #[test]
    fn a_second_start_writes_nothing_and_a_moved_pagify_is_followed() {
        let scratch = Scratch::new("again");
        let here = Path::new(r"C:\Program Files\Pagify\Pagify.exe");
        assert!(write(&entries(here), &scratch.prefix()));
        assert!(!write(&entries(here), &scratch.prefix()), "an unchanged registration was written again");
        let moved = Path::new(r"D:\Apps\Pagify\Pagify.exe");
        assert!(write(&entries(moved), &scratch.prefix()), "a Pagify that moved was not followed");
        assert_eq!(
            registry::get(&format!(r"{}Software\Classes\Pagify.Document\shell\open\command", scratch.prefix()), ""),
            Some(r#""D:\Apps\Pagify\Pagify.exe" "%1""#.to_string())
        );
    }
}
