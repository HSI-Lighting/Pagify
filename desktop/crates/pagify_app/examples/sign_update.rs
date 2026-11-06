//! Sign a Pagify build for GitHub with Pagify's update key (`make_update_key.rs`), as
//! `tauri signer sign` signs the other HSI apps' builds: prints the signature the feed
//! carries, which `update::signed_by` checks. `build.ps1 -Publish` runs it.
//!
//!     cargo run -p pagify_app --example sign_update -- <pagify-updater.key> <pagify_X.Y.Z_windows.zip>
//!
//! The key's password comes from `PAGIFY_UPDATE_KEY_PASSWORD` (none if unset), never
//! from the command line, where every process list reads it. The update tests sign
//! through [`sign`] too.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let (Some(key), Some(file)) = (args.next(), args.next()) else {
        return Err("usage: sign_update <pagify-updater.key> <file>".into());
    };
    let password = std::env::var("PAGIFY_UPDATE_KEY_PASSWORD").unwrap_or_default();
    println!("{}", sign(&std::fs::read_to_string(&key)?, password, std::path::Path::new(&file))?);
    Ok(())
}

/// `file` signed with `key` — a key file as `tauri signer generate` writes one — as
/// Tauri writes a signature: base64 of minisign's own text, its signed comment naming
/// the file.
pub fn sign(key: &str, password: String, file: &std::path::Path) -> Result<String, Box<dyn std::error::Error>> {
    let key = String::from_utf8(STANDARD.decode(key.trim())?)?;
    let key = minisign::SecretKeyBox::from_string(&key)?.into_secret_key(Some(password))?;
    let name = file.file_name().ok_or("no file name")?.to_string_lossy().into_owned();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let signature = minisign::sign(
        None,
        &key,
        std::fs::File::open(file)?,
        Some(&format!("timestamp:{now}\tfile:{name}")),
        Some("signature from tauri secret key"),
    )?;
    Ok(STANDARD.encode(signature.to_string()))
}
