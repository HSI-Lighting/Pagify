//! `pagify-issue` — the offline issuing tool.
//!
//! The trust model has one root, pinned into Pagify, and leaves issued from
//! it for whoever signs. A root with no issuing tool is a root whose private
//! key gets handled by hand, which is how roots leak — so this is the only
//! thing that touches it. It runs on a machine that is not on the network,
//! and it does four things:
//!
//! ```text
//! pagify-issue root new           --dir <root-dir> --subject "CN=HSI Root,O=HSI Lighting" [--years 20]
//! pagify-issue root backup        --dir <root-dir> --to <backup-dir>
//! pagify-issue root restore-check --dir <root-dir> --backup <backup-dir>
//! pagify-issue leaf issue         --dir <root-dir> --subject "CN=…,O=HSI Lighting" --out <file.p12> [--years 10]
//! pagify-issue leaf denylist-line --dir <root-dir> --serial <hex>
//! ```
//!
//! # What is in the root directory
//!
//! | File | What |
//! |---|---|
//! | `root.der` | the root certificate — what goes into `rust/pdf_core/trust/roots.der` |
//! | `root.key` | the private key, PKCS#8 **encrypted** under the passphrase (PBES2) |
//! | `serial` | the next leaf serial, so that no two leaves ever share one |
//! | `issued.txt` | every leaf issued: serial, subject, date — the list a denylist is written from |
//! | `restore-record.txt` | written by `restore-check`: the day the backup was proven to work |
//!
//! # The backup, and why the tool is strict about it
//!
//! Losing the root's key is not a compromise — nobody else has it — but it
//! means no new leaf can ever be issued, and the next signer, laptop or
//! revocation forces a root rotation, which is an app release. So `root new`
//! tells you to back up before anything else; `root backup` copies the two
//! files somewhere else; and `leaf issue` **refuses to issue the first real
//! leaf until `restore-check` has been run against a backup** — restored,
//! used to issue a throwaway leaf, that leaf checked against the root by the
//! same code Pagify runs. A backup that has never been restored is a hope.
//!
//! # Passphrases
//!
//! Prompted, never taken on the command line, never echoed. There is no
//! recovery: a root whose passphrase is lost is a root that is lost.

use std::path::{Path, PathBuf};
use std::time::Duration;

use der::{Decode, Encode};
use pdf_core::pdf::sm::{self, DISTINGUISHING_ID};
use pdf_core::pdf::trust::{self, Anchors, Trust};
use sm2::dsa::signature::Signer;
use sm2::pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey};
use x509_cert::Certificate;
use zeroize::Zeroizing;

type Failure = Box<dyn std::error::Error>;
type Outcome<T> = std::result::Result<T, Failure>;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(problem) = run(&args) {
        eprintln!("pagify-issue: {problem}");
        std::process::exit(1);
    }
}

fn run(args: &[String]) -> Outcome<()> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["root", "new", rest @ ..] => root_new(&Options::parse(rest)?),
        ["root", "backup", rest @ ..] => root_backup(&Options::parse(rest)?),
        ["root", "restore-check", rest @ ..] => root_restore_check(&Options::parse(rest)?),
        ["leaf", "issue", rest @ ..] => leaf_issue(&Options::parse(rest)?),
        ["leaf", "denylist-line", rest @ ..] => leaf_denylist_line(&Options::parse(rest)?),
        _ => Err(USAGE.into()),
    }
}

const USAGE: &str = "usage:
  pagify-issue root new           --dir <root-dir> --subject \"CN=HSI Root,O=HSI Lighting\" [--years 20]
  pagify-issue root backup        --dir <root-dir> --to <backup-dir>
  pagify-issue root restore-check --dir <root-dir> --backup <backup-dir>
  pagify-issue leaf issue         --dir <root-dir> --subject \"CN=…,O=HSI Lighting\" --out <file.p12> [--years 10]
  pagify-issue leaf denylist-line --dir <root-dir> --serial <hex>";

/// `--name value` pairs, and nothing else.
struct Options(Vec<(String, String)>);

impl Options {
    fn parse(words: &[&str]) -> Outcome<Options> {
        let mut pairs = Vec::new();
        let mut at = 0;
        while at < words.len() {
            let name = words[at]
                .strip_prefix("--")
                .ok_or_else(|| format!("expected an option, got {:?}\n{USAGE}", words[at]))?;
            let value = words
                .get(at + 1)
                .ok_or_else(|| format!("--{name} needs a value\n{USAGE}"))?;
            pairs.push((name.to_string(), value.to_string()));
            at += 2;
        }
        Ok(Options(pairs))
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    fn need(&self, name: &str) -> Outcome<&str> {
        self.get(name).ok_or_else(|| format!("--{name} is required\n{USAGE}").into())
    }

    fn dir(&self) -> Outcome<PathBuf> {
        Ok(PathBuf::from(self.need("dir")?))
    }

    fn years(&self, default: u64) -> Outcome<u64> {
        match self.get("years") {
            None => Ok(default),
            Some(text) => text.parse().map_err(|_| format!("--years {text:?} is not a number").into()),
        }
    }
}

// ---------------------------------------------------------------- root --

/// `root new`: a keypair, a self-signed certificate, and an empty ledger.
fn root_new(options: &Options) -> Outcome<()> {
    let dir = options.dir()?;
    let subject = options.need("subject")?;
    let years = options.years(20)?;
    if dir.join("root.key").exists() || dir.join("root.der").exists() {
        return Err(format!("{} already holds a root — this will not overwrite one", dir.display()).into());
    }
    std::fs::create_dir_all(&dir)?;

    println!("The root's passphrase. There is no recovery: lose it and the root is lost.");
    let passphrase = prompt_twice("root passphrase")?;

    let secret = sm2::SecretKey::random(&mut rand_core::OsRng);
    let key = sm2::dsa::SigningKey::new(DISTINGUISHING_ID, &secret)?;
    let name = parse_name(subject)?;
    let certificate = make_certificate(
        &name,
        &name,
        &key,
        &secret.public_key(),
        &random_serial(),
        years,
        true,
    )?;
    let encrypted = secret.to_pkcs8_encrypted_der(&mut rand_core::OsRng, passphrase.as_bytes())?;

    write_new(&dir.join("root.der"), &certificate.to_der()?)?;
    write_new(&dir.join("root.key"), encrypted.as_bytes())?;
    write_new(&dir.join("serial"), b"1001\n")?;
    write_new(&dir.join("issued.txt"), b"# serial  subject  issued-on (UTC)\n")?;

    println!("root made in {}", dir.display());
    println!("  subject : {}", certificate.tbs_certificate.subject);
    println!("  serial  : {}", hex(certificate.tbs_certificate.serial_number.as_bytes()));
    println!("  valid   : {} years, from today", years);
    println!();
    println!("Next, before anything else:");
    println!("  1. pagify-issue root backup --dir {} --to <removable media>", dir.display());
    println!("  2. on another machine: pagify-issue root restore-check --dir <a scratch dir> --backup <that media>");
    println!("     — and bring restore-record.txt back here.");
    println!("  3. copy root.der to rust/pdf_core/trust/roots.der and release Pagify with it.");
    println!("No leaf will be issued until step 2 has been recorded.");
    Ok(())
}

/// `root backup`: the two files that are the root, copied somewhere else.
fn root_backup(options: &Options) -> Outcome<()> {
    let dir = options.dir()?;
    let to = PathBuf::from(options.need("to")?);
    let key = std::fs::read(dir.join("root.key"))?;
    let der = std::fs::read(dir.join("root.der"))?;
    std::fs::create_dir_all(&to)?;
    write_new(&to.join("root.key"), &key)?;
    write_new(&to.join("root.der"), &der)?;
    // The copy is checked byte for byte, because a backup that was not
    // read back is a backup that was assumed.
    if std::fs::read(to.join("root.key"))? != key || std::fs::read(to.join("root.der"))? != der {
        return Err("the copy does not read back as what was written".into());
    }
    println!("backed up to {}: root.key (encrypted) and root.der", to.display());
    println!("Keep it in a different place from {}, and write down where.", dir.display());
    println!("Then prove it: pagify-issue root restore-check --dir <scratch dir> --backup {}", to.display());
    Ok(())
}

/// `root restore-check`: the backup, restored and used — a throwaway leaf is
/// issued from it and checked by the code Pagify runs — and a record written
/// that says so, which is what `leaf issue` looks for.
fn root_restore_check(options: &Options) -> Outcome<()> {
    let dir = options.dir()?;
    let backup = PathBuf::from(options.need("backup")?);
    std::fs::create_dir_all(&dir)?;

    let der = std::fs::read(backup.join("root.der"))?;
    let encrypted = std::fs::read(backup.join("root.key"))?;
    let root = Certificate::from_der(&der)?;
    let passphrase = prompt_once("root passphrase (from the backup)")?;
    let secret = sm2::SecretKey::from_pkcs8_encrypted_der(&encrypted, passphrase.as_bytes())
        .map_err(|_| "the backup's key did not open — wrong passphrase, or the backup is damaged")?;
    let key = sm2::dsa::SigningKey::new(DISTINGUISHING_ID, &secret)?;

    // The key must be the certificate's.
    if secret.public_key().to_public_key_der()?.as_bytes()
        != root.tbs_certificate.subject_public_key_info.to_der()?
    {
        return Err("the backup's key is not the key of the backup's certificate".into());
    }

    // A throwaway leaf from the restored root, checked the way Pagify checks.
    let throwaway = sm2::SecretKey::random(&mut rand_core::OsRng);
    let leaf = make_certificate(
        &root.tbs_certificate.subject,
        &parse_name("CN=restore check (throwaway),O=Pagify")?,
        &key,
        &throwaway.public_key(),
        &[0x7f],
        1,
        false,
    )?;
    let anchors = Anchors::new(&der, "")?;
    if trust::trust_in(&leaf, &anchors) != Trust::Pinned {
        return Err("a leaf issued from the restored root does not chain to it — the restore failed".into());
    }

    let record = format!(
        "restore checked on {} (UTC)\n  backup: {}\n  root subject: {}\n  root serial: {}\n  machine: {}\n",
        today(),
        backup.display(),
        root.tbs_certificate.subject,
        hex(root.tbs_certificate.serial_number.as_bytes()),
        std::env::var("HOSTNAME").or_else(|_| std::env::var("COMPUTERNAME")).unwrap_or_else(|_| "(unknown)".into()),
    );
    std::fs::write(dir.join("restore-record.txt"), &record)?;
    print!("{record}");
    println!("written to {} — put restore-record.txt beside the working root", dir.join("restore-record.txt").display());
    Ok(())
}

// ---------------------------------------------------------------- leaf --

/// `leaf issue`: a keypair and a certificate signed by the root, in a `.p12`.
fn leaf_issue(options: &Options) -> Outcome<()> {
    let dir = options.dir()?;
    let subject = options.need("subject")?;
    let out = PathBuf::from(options.need("out")?);
    let years = options.years(10)?;
    if out.exists() {
        return Err(format!("{} exists — this will not overwrite an identity", out.display()).into());
    }
    if !dir.join("restore-record.txt").is_file() {
        return Err(format!(
            "no restore-record.txt in {} — no leaf is issued until the root's backup has been \
             restored and checked: pagify-issue root restore-check",
            dir.display()
        )
        .into());
    }

    let root_der = std::fs::read(dir.join("root.der"))?;
    let root = Certificate::from_der(&root_der)?;
    let encrypted = std::fs::read(dir.join("root.key"))?;
    let passphrase = prompt_once("root passphrase")?;
    let secret = sm2::SecretKey::from_pkcs8_encrypted_der(&encrypted, passphrase.as_bytes())
        .map_err(|_| "the root key did not open — wrong passphrase, or the file is damaged")?;
    let key = sm2::dsa::SigningKey::new(DISTINGUISHING_ID, &secret)?;

    let serial = next_serial(&dir)?;
    let leaf_secret = sm2::SecretKey::random(&mut rand_core::OsRng);
    let leaf = make_certificate(
        &root.tbs_certificate.subject,
        &parse_name(subject)?,
        &key,
        &leaf_secret.public_key(),
        &serial,
        years,
        false,
    )?;
    // Checked before it is written, by the code that will check it in use.
    let anchors = Anchors::new(&root_der, "")?;
    if trust::trust_in(&leaf, &anchors) != Trust::Pinned {
        return Err("the leaf does not chain to the root it was just issued from".into());
    }

    println!("The identity's password — what the signer types into Pagify.");
    let password = prompt_twice("identity password")?;
    let key_der = leaf_secret.to_pkcs8_der()?;
    let pfx = p12::PFX::new(&leaf.to_der()?, key_der.as_bytes(), None, &password, subject)
        .ok_or("the PKCS#12 file could not be built")?;
    write_new(&out, &pfx.to_der())?;

    // Read back through the reader Pagify uses, so what was written is what
    // will be loaded.
    let identity = pdf_core::pdf::sign::Identity::from_pkcs12(&std::fs::read(&out)?, &password)
        .map_err(|e| format!("the identity written does not read back: {e}"))?;
    if identity.subject()? != leaf.tbs_certificate.subject.to_string() {
        return Err("the identity written names a different subject".into());
    }

    let line = format!("{}  {}  {}\n", hex(&serial), leaf.tbs_certificate.subject, today());
    append(&dir.join("issued.txt"), line.as_bytes())?;
    println!("issued {}", out.display());
    println!("  subject: {}", leaf.tbs_certificate.subject);
    println!("  serial : {}", hex(&serial));
    println!("  issuer : {}", leaf.tbs_certificate.issuer);
    println!("  valid  : {years} years — not checked by Pagify; revocation is the denylist");
    println!("To revoke it later: pagify-issue leaf denylist-line --dir {} --serial {}", dir.display(), hex(&serial));
    Ok(())
}

/// `leaf denylist-line`: the line to add to `trust/denylist.txt`.
fn leaf_denylist_line(options: &Options) -> Outcome<()> {
    let dir = options.dir()?;
    let serial = options.need("serial")?;
    if serial.is_empty() || !serial.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("--serial {serial:?} is not hex").into());
    }
    let root = Certificate::from_der(&std::fs::read(dir.join("root.der"))?)?;
    // The issuer as Pagify prints it — the same `Name` display, so the line
    // matches what the check compares.
    println!("{}  {}", serial.to_ascii_uppercase().trim_start_matches('0'), root.tbs_certificate.subject);
    Ok(())
}

// ---------------------------------------------------------- certificates --

/// A certificate: SM2 over SM3 under the pinned distinguishing ID, which is
/// what `pdf_core::pdf::trust` checks and what GM/T 0015 prescribes.
fn make_certificate(
    issuer: &x509_cert::name::Name,
    subject: &x509_cert::name::Name,
    signer: &sm2::dsa::SigningKey,
    public: &sm2::PublicKey,
    serial: &[u8],
    years: u64,
    is_root: bool,
) -> Outcome<Certificate> {
    use x509_cert::ext::pkix::{BasicConstraints, KeyUsage, KeyUsages};
    use x509_cert::ext::AsExtension;
    use x509_cert::serial_number::SerialNumber;
    use x509_cert::time::Validity;
    use x509_cert::TbsCertificate;

    let algorithm = spki::AlgorithmIdentifierOwned { oid: sm::ID_SM2_WITH_SM3, parameters: None };
    let spki_der = public.to_public_key_der()?;
    let subject_public_key_info = spki::SubjectPublicKeyInfoOwned::from_der(spki_der.as_bytes())?;

    let basic = BasicConstraints { ca: is_root, path_len_constraint: None };
    let usage = if is_root {
        KeyUsage(KeyUsages::KeyCertSign | KeyUsages::CRLSign)
    } else {
        KeyUsage(KeyUsages::DigitalSignature | KeyUsages::NonRepudiation)
    };
    let extensions = vec![
        basic.to_extension(subject, &[])?,
        usage.to_extension(subject, &[])?,
    ];

    let tbs = TbsCertificate {
        version: x509_cert::Version::V3,
        serial_number: SerialNumber::new(serial)?,
        signature: algorithm.clone(),
        issuer: issuer.clone(),
        validity: Validity::from_now(Duration::from_secs(years * 365 * 24 * 60 * 60))?,
        subject: subject.clone(),
        subject_public_key_info,
        issuer_unique_id: None,
        subject_unique_id: None,
        extensions: Some(extensions),
    };
    let signature = signer.try_sign(&tbs.to_der()?)?;
    Ok(Certificate {
        tbs_certificate: tbs,
        signature_algorithm: algorithm,
        signature: der::asn1::BitString::from_bytes(&sm::signature_to_der(&signature)?)?,
    })
}

fn parse_name(text: &str) -> Outcome<x509_cert::name::Name> {
    text.parse::<x509_cert::name::Name>()
        .map_err(|e| format!("{text:?} is not a name like \"CN=…,O=…\": {e}").into())
}

/// A root's serial: 16 random bytes with the top bit clear, so that it is
/// positive and unlike anybody else's.
fn random_serial() -> Vec<u8> {
    use rand_core::RngCore;
    let mut bytes = [0u8; 16];
    rand_core::OsRng.fill_bytes(&mut bytes);
    bytes[0] &= 0x7f;
    bytes[0] |= 0x40;
    bytes.to_vec()
}

/// The next leaf serial from the ledger, and the ledger moved on.
fn next_serial(dir: &Path) -> Outcome<Vec<u8>> {
    let path = dir.join("serial");
    let text = std::fs::read_to_string(&path)?;
    let current = u64::from_str_radix(text.trim(), 16)
        .map_err(|_| format!("{} does not hold a hex serial", path.display()))?;
    write_private(&path, format!("{:X}\n", current + 1).as_bytes(), false)?;
    let mut bytes = current.to_be_bytes().to_vec();
    while bytes.len() > 1 && bytes[0] == 0 {
        bytes.remove(0);
    }
    Ok(bytes)
}

// ------------------------------------------------------------------ io --

/// A passphrase: from the terminal without echo when there is one, from
/// standard input — one line — when there is not, which is how a test or a
/// script drives this. Never from the command line.
fn read_secret(prompt: &str) -> Outcome<Zeroizing<String>> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        return Ok(Zeroizing::new(rpassword::prompt_password(prompt)?));
    }
    let mut line = Zeroizing::new(String::new());
    std::io::stdin().read_line(&mut line)?;
    let trimmed = line.trim_end_matches(['\r', '\n']).to_string();
    Ok(Zeroizing::new(trimmed))
}

fn prompt_once(what: &str) -> Outcome<Zeroizing<String>> {
    let typed = read_secret(&format!("{what}: "))?;
    if typed.is_empty() {
        return Err("an empty passphrase is not one".into());
    }
    Ok(typed)
}

fn prompt_twice(what: &str) -> Outcome<Zeroizing<String>> {
    let first = prompt_once(what)?;
    let again = read_secret(&format!("{what}, again: "))?;
    if *first != *again {
        return Err("the two do not match".into());
    }
    Ok(first)
}

/// Write `bytes` at `path` with the narrowest mode the platform has, and get
/// them onto the disk before returning.
///
/// **The root key, an issued identity and the serial are private from the
/// moment the file exists.** `create_new` alone leaves the mode to the umask —
/// 0644 under the usual one — and `std::fs::write` was no better, so a key
/// written on a shared machine was readable while the tool still had it open.
/// `sync_all` matters for the same reason the backup reads itself back: a copy
/// that is only in the page cache has not been proven, and `root backup` says
/// it has. Found by audit.
fn write_private(path: &Path, bytes: &[u8], create_new: bool) -> Outcome<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    if create_new {
        options.create_new(true);
    } else {
        options.create(true).truncate(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

/// Written only where nothing is: a key or an identity is never overwritten.
fn write_new(path: &Path, bytes: &[u8]) -> Outcome<()> {
    write_private(path, bytes, true)
}

fn append(path: &Path, bytes: &[u8]) -> Outcome<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.append(true).create(true);
    // The ledger names every leaf issued, so it is kept as private as the
    // identities themselves. Found by audit.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    let text: String = bytes.iter().map(|b| format!("{b:02X}")).collect();
    let trimmed = text.trim_start_matches('0');
    if trimmed.is_empty() { "0".into() } else { trimmed.to_string() }
}

/// Today, as `YYYY-MM-DD`, from the system clock — the signer's own, as the
/// plan accepts everywhere else.
fn today() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = seconds / 86_400;
    // Civil-from-days, Howard Hinnant's algorithm.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root_and_key() -> (Certificate, sm2::dsa::SigningKey, Vec<u8>) {
        let secret = sm2::SecretKey::random(&mut rand_core::OsRng);
        let key = sm2::dsa::SigningKey::new(DISTINGUISHING_ID, &secret).unwrap();
        let name = parse_name("CN=Test Root,O=Pagify").unwrap();
        let root = make_certificate(&name, &name, &key, &secret.public_key(), &random_serial(), 20, true).unwrap();
        let der = root.to_der().unwrap();
        (root, key, der)
    }

    /// **A leaf this issues chains, by the code Pagify runs** — and a leaf
    /// from another root with the same name does not.
    #[test]
    fn an_issued_leaf_is_pinned_under_its_root_and_not_under_a_lookalike() {
        let (root, key, der) = root_and_key();
        let leaf_secret = sm2::SecretKey::random(&mut rand_core::OsRng);
        let leaf = make_certificate(
            &root.tbs_certificate.subject,
            &parse_name("CN=Alice,O=Pagify").unwrap(),
            &key,
            &leaf_secret.public_key(),
            &[0x10, 0x01],
            10,
            false,
        )
        .unwrap();
        let anchors = Anchors::new(&der, "").unwrap();
        assert_eq!(trust::trust_in(&leaf, &anchors), Trust::Pinned);
        assert_eq!(trust::trust_in(&leaf, &Anchors::new(&der, "1001 CN=Test Root,O=Pagify").unwrap()), Trust::Revoked);

        let (_, _, other) = root_and_key();
        assert_eq!(trust::trust_in(&leaf, &Anchors::new(&other, "").unwrap()), Trust::Unrecognised);
        // And the root itself reads back as a CA with the right key usage.
        let again = Certificate::from_der(&der).unwrap();
        assert_eq!(again.tbs_certificate.subject.to_string(), "CN=Test Root,O=Pagify");
        assert_eq!(again.signature_algorithm.oid, sm::ID_SM2_WITH_SM3);
    }

    /// The identity written is one Pagify's reader opens and signs with.
    #[test]
    fn the_identity_written_reads_back_through_pagify() {
        let (root, key, _) = root_and_key();
        let leaf_secret = sm2::SecretKey::random(&mut rand_core::OsRng);
        let leaf = make_certificate(
            &root.tbs_certificate.subject,
            &parse_name("CN=Bob,O=Pagify").unwrap(),
            &key,
            &leaf_secret.public_key(),
            &[0x10, 0x02],
            10,
            false,
        )
        .unwrap();
        let key_der = leaf_secret.to_pkcs8_der().unwrap();
        let pfx = p12::PFX::new(&leaf.to_der().unwrap(), key_der.as_bytes(), None, "pw", "Bob").unwrap();
        let identity = pdf_core::pdf::sign::Identity::from_pkcs12(&pfx.to_der(), "pw").expect("reads back");
        assert_eq!(identity.subject().unwrap(), "CN=Bob,O=Pagify");
        identity.sm2_key().expect("signs");
        assert!(pdf_core::pdf::sign::Identity::from_pkcs12(&pfx.to_der(), "not pw").is_err());
    }

    /// The encrypted root key opens with its passphrase and not without.
    #[test]
    fn the_root_key_is_encrypted_under_its_passphrase() {
        let secret = sm2::SecretKey::random(&mut rand_core::OsRng);
        let encrypted = secret.to_pkcs8_encrypted_der(&mut rand_core::OsRng, b"correct horse").unwrap();
        let back = sm2::SecretKey::from_pkcs8_encrypted_der(encrypted.as_bytes(), b"correct horse").unwrap();
        assert_eq!(back.to_bytes(), secret.to_bytes());
        assert!(sm2::SecretKey::from_pkcs8_encrypted_der(encrypted.as_bytes(), b"wrong").is_err());
        // And the plaintext key is not in the file.
        let plain = secret.to_pkcs8_der().unwrap();
        assert!(!encrypted.as_bytes().windows(32).any(|w| plain.as_bytes().windows(32).any(|p| p == w)));
    }

    #[test]
    fn serials_are_hex_without_leading_zeros_and_dates_are_civil() {
        assert_eq!(hex(&[0x10, 0x01]), "1001");
        assert_eq!(hex(&[0x00, 0x7f]), "7F");
        assert_eq!(hex(&[0x00]), "0");
        assert_eq!(today().len(), 10);
        assert!(today().starts_with("20"));
    }

    #[test]
    fn options_are_name_value_pairs() {
        let options = Options::parse(&["--dir", "/x", "--years", "5"]).unwrap();
        assert_eq!(options.get("dir"), Some("/x"));
        assert_eq!(options.years(20).unwrap(), 5);
        assert!(Options::parse(&["--dir"]).is_err());
        assert!(Options::parse(&["dir", "/x"]).is_err());
        assert!(Options::parse(&["--years", "five"]).unwrap().years(20).is_err());
    }

    /// **A key, an identity and the ledger are readable by their owner
    /// alone.** Found by audit: `create_new` without a mode left them at the
    /// umask's 0644. Unix-only, because the mode is.
    #[cfg(unix)]
    #[test]
    fn keys_identities_and_the_ledger_are_readable_by_their_owner_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("pagify-issue-modes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mode_of =
            |path: &Path| std::fs::metadata(path).expect("metadata").permissions().mode() & 0o777;

        for name in ["root.key", "leaf.p12", "issued.txt"] {
            let path = dir.join(name);
            write_new(&path, b"x").unwrap();
            assert_eq!(mode_of(&path), 0o600, "{name} was written at {:o}", mode_of(&path));
        }

        // The ledger is appended to after it exists; that must not widen it.
        let ledger = dir.join("issued.txt");
        append(&ledger, b"line\n").unwrap();
        assert_eq!(mode_of(&ledger), 0o600);

        // The serial exists already when it is advanced; that must not widen
        // it either.
        let serial = dir.join("serial");
        write_new(&serial, b"1001\n").unwrap();
        assert_eq!(next_serial(&dir).unwrap(), vec![0x10, 0x01]);
        assert_eq!(mode_of(&serial), 0o600, "advancing the serial widened it");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
