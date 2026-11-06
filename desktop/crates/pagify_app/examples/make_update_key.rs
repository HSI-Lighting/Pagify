//! Make Pagify's update key: the private half into `<key file>` (as `tauri signer
//! generate` writes one, sealed with `PAGIFY_UPDATE_KEY_PASSWORD`, none if unset), and
//! the public half printed, for `UPDATE_KEY` in `update.rs`. Made once, on 2026-11-06
//! (`E:\KEYS\Pagify\pagify-updater.key`); a new one means every installed copy has to
//! be updated through Dropbox before it trusts GitHub again, so an existing file is
//! never overwritten.
//!
//!     cargo run -p pagify_app --example make_update_key -- <key file>

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = std::path::PathBuf::from(std::env::args_os().nth(1).ok_or("usage: make_update_key <key file>")?);
    if file.exists() {
        return Err(format!("{} exists: a key is never overwritten", file.display()).into());
    }
    let password = std::env::var("PAGIFY_UPDATE_KEY_PASSWORD").unwrap_or_default();
    let minisign::KeyPair { pk, sk } = minisign::KeyPair::generate_encrypted_keypair(Some(password))?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&file, STANDARD.encode(sk.to_box(None)?.to_string()))?;
    println!("{}", STANDARD.encode(pk.to_box()?.to_string()));
    Ok(())
}
