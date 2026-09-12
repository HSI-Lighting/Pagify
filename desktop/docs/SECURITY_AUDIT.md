# Security Audit — Pagify Desktop

- **Date:** 2026-09-12
- **Target:** `pagify-desktop-mac` @ `1b36bee` plus the uncommitted Linux portability changes, working tree as of the audit date.
- **Scope:** `desktop/` (`pagify_app`, `pagify_shell`), reachable `rust/pdf_core` (shared with Android/iOS), vendored `pdfium-render` fork, PDFium loading, packaging/install scripts, committed binaries, dependency lockfile.
- **Method:** source review by five independent tracks; auditors compiled standalone probes against the repo's fixtures (in `/tmp`, no repo changes); `cargo audit` run against `desktop/Cargo.lock` (521 deps, RustSec DB f1243 advisories); committed PDFium/font/OCR binaries hashed and compared to upstream; every headline finding re-verified against source.
- **Not covered:** fuzzing PDFium or the custom lexer, dynamic testing of the packaged/signed app, penetration test of macOS entitlements, kernel/OS-level sandboxing.

## Executive summary

| Severity | Count | Headline |
|---|---|---|
| Critical | 2 | Signature "valid" is forgeable; automatic redaction can claim success while the secret stays in the file |
| High | 5 | `hiddendata` claims removals it doesn't make; signed documents can't actually be saved; 6 crash/DoS triggers on malformed PDFs with no panic containment; untrusted Argon2 costs; PDFium downloaded without integrity checks |
| Medium | 9 | `unsecure` doesn't do what it says, save/extract file handling, Secure Plus structural gaps, `rsa` timing side channel, unpinned `cad_kernel`, +3 |
| Low / Info | 11 | path writes via `record`, signing password in argv, installer `rm -rf`, unzeroized secrets, unmaintained font parsers, etc. |

The cryptographic *primitives* are in good shape (AES-256 R6 verified, Argon2id vault sound, FF1 passes NIST vectors, OCR models hash-pinned, no network code at all). The serious problems are in the **security features' claims and failure handling**: signatures aren't cryptographically checked, redaction/sanitization can lie, and malformed PDFs reliably kill the process. Most pdf_core findings also affect the Android/iOS builds.

## Critical

### C1 — `validate` never verifies the signature; "unchanged since it was signed" is forgeable without a key

`rust/pdf_core/src/pdf/validate.rs:104-157`, UI at `pagify_app/src/main.rs:3392-3427`

`verdict_for` compares a SHA-256 of the byte range to the `messageDigest` attribute inside the **unauthenticated CMS blob**. No `VerifyingKey::verify` call exists in production code (`grep` finds it only in `tests/signing.rs:137`). An attacker can edit the PDF, repoint `/ByteRange`, and paste any CMS whose `messageDigest` matches — Pagify reports `Unaltered`. The shipped test does exactly this and observes the RSA signature is invalid while Pagify says "unchanged".

**Fix:** verify the CMS signature over `signedAttrs` with the certificate's public key; report signer trust separately; never emit "unchanged" unless the signature verified.

### C2 — `smartredact` can report "gone for good" while the sensitive text remains

`pagify_app/src/main.rs:3603-3642`; gate at `document/redact.rs:291-297`; `pdfium_doc.rs:6049-6185`

Text inside a Form XObject is detected as `Uncleared::Form`, but `would_only_draw_a_mark()` only accounts for `Uncleared::Image`, and `smartredact` passes `allow_incomplete: true` and ignores `RedactionReport.uncleared` entirely. A probe redacting `4111 1111 1111 1111` drawn in a Form reported "1 redacted — gone for good" while the number was still extractable from the saved file. The interactive dialog does surface blockers; the automated path does not.

**Fix:** make `smartredact` refuse or surface any non-empty `uncleared`; include `Form`/`OutlinedText` in the only-a-mark gate; claim destruction only when `is_complete()`.

## High

### H1 — `hiddendata clean` claims embedded files and JavaScript were removed; both survive

`rust/pdf_core/src/pdf/hidden.rs:177-201`; UI message `main.rs:3662-3666`

`strip` removes only `/Metadata`, `/Info` and unreachable objects. `/Names/EmbeddedFiles`, `/Names/JavaScript` and `/OpenAction` are reachable from the catalogue, so they are copied through unchanged, while the UI prints `removed: 1 embedded file(s); JavaScript`. Probe: attachment payload and `app.alert` both still present after "clean".

**Fix:** delete EmbeddedFiles/JavaScript/`/AA` during `strip`; add a save-and-reopen test asserting `hidden_data().is_empty()`.

### H2 — `certify`/`timestamp` signatures are never persisted; `save` breaks them and `close` discards them silently

`document/pdfium_doc.rs:2245-2258`, `2924-2943`, `2982-2990`; `main.rs:2644-2655`

`sign_document` keeps the exact signed bytes only in `self.written`, then sets `dirty = false`. `save_full_copy` clears `written` and re-saves via PDFium (objects relocate); `save_incremental` stores the *new* bytes. Neither writes the signed bytes to disk. Probe: after `certify`, a full-copy save covers only 1 458 of 17 826 bytes; closing loses the signature with no prompt because `would_lose_work()` doesn't consider it.

**Fix:** write `self.written` verbatim on save after signing/timestamping (and include that state in the unsaved-changes guard).

### H3 — Crash cluster: six malformed-PDF triggers, and no panic containment on desktop

- Out-of-bounds slice in `mark_is_ours`: 64-element buffer, unclamped length from `FPDFPageObjMark_GetName` (`pdfium_doc.rs:7580-7592`) — a >64-char mark name collapses on open (`main.rs:2103` restores markup for every page).
- Char-boundary panic in the vault hex decoder `envelope.rs:168` (also `ledger.rs:95`): any multi-byte char at an even offset in `pagify-lock.json` panics on the first drawn frame, before any click.
- Char-boundary panic in `/PagifyColor` decoding `pdfium_doc.rs:7154`: an 8-byte value like `0é00000` panics `marks`, `status`, or any page click.
- Unbounded recursion in the custom lexer `pdf/object.rs:164` (no depth counter): a deeply nested array/dict causes stack overflow (SIGSEGV, not catchable).
- Unchecked `usize` adds in `validate.rs:112-125`: `/ByteRange [0 0 1e308 <len+1>]` wraps, passes the bounds guard, then panics on the slice.
- Unbounded FlateDecode inflation (`content.rs:690`, ~1000:1 measured): a 1 MB stream becomes ~1 GB on the first edit/redact/lock; font and `/ToUnicode` reads share this.

`grep catch_unwind desktop/crates` → **0**: unlike the Android JNI layer, a panic in the desktop path terminates the app with unsaved edits — a reliable remotely-triggerable DoS per crafted document.

**Fix:** clamp all six (`.min(buffer.len())`, ASCII/byte-decoding, depth cap, checked arithmetic, bounded inflate), and wrap document operations in `catch_unwind` with an error surfaced like the C ABI does.

### H4 — Document-supplied Argon2 parameters allow pre-authentication 4 TiB allocation

`pdf/secure_plus.rs:104-119`, `crypto/kdf.rs:82-96`, `crypto/envelope.rs:90-100`

`/M`, `/T`, `/P` are read from the untrusted `/Encrypt` dict (or lock attachment) and passed straight to Argon2id **before** `/Verify` is checked; `/M 4294967295` → 4 TiB, `/T 4294967295` → 4 billion passes. The `argon2` crate accepts these as valid params. Any typed password triggers it.

**Fix:** clamp/reject cost parameters (memory ≤ 1 GiB, time ≤ 10, lanes ≤ 16) before deriving.

### H5 — PDFium native libraries are fetched without integrity verification

`desktop/tools/fetch_pdfium.sh:39-45`, `rust/tools/fetch_pdfium.ps1:46-51`; loader `pagify_shell/src/pdfium.rs:66-99`

`curl | tar -xzf` with no checksum/signature check; `PDFIUM_TAG` can redirect to an arbitrary release; the loaded `.so/.dylib/.dll` then executes in-process with full document access. The release ships a Sigstore attestation that none of the scripts use. Mitigating fact: the four committed slices **do** hash-match the official `chromium/7881` assets (verified), so this is a re-fetch risk, not a present compromise. The stale `PAGIFY_ROOT` fallback (`pdfium.rs:54-57`) additionally points inside a user-writable repo path with no hash check.

**Fix:** pin per-asset SHA-256 or verify the attestation; fail closed on mismatch; drop the out-of-repo fallback.

## Medium

| # | Issue | Location |
|---|---|---|
| M1 | `unsecure` leaves the old password applied on the default (incremental) save; `rearm_security` can re-arm it | `pdfium_doc.rs:2190-2208, 2541-2546, 2924-2930, 4462-4473` |
| M2 | Save staging path is predictable (`name.pdf.pagify-save`) and opened with `File::create` (follows symlinks); saved file loses original restrictive mode (0600 → 0644) | `session.rs:39-43, 288-309` |
| M3 | `extract_to` truncates the destination before validation; failure leaves a zero-byte/partial file, no atomicity | `session.rs:948-963` |
| M4 | Secure Plus binds ciphertext only to object number: page references/value swaps inside one file are undetected, contradicting its authentication claim | `secure_plus.rs:152-171, 196-221` |
| M5 | Vault parse **errors** aren't cached: a large `pagify-lock.json` with bad magic is re-read and re-parsed every frame (`?` before caching) | `pdfium_doc.rs:4871-4880` |
| M6 | `rsa` 0.9.10: RUSTSEC-2023-0071 (Marvin timing side channel, CVSS 5.9, **no fix available**); live in the signing path. Local-only exposure lowers practical risk | `sign.rs:91, 402`; `cargo audit` |
| M7 | `cad_kernel` consumed from an unpinned sibling checkout (lockfile records no source/commit); the working tree already drifts | `desktop/Cargo.toml:41` |
| M8 | Timestamp uses plain HTTP and the returned RFC 3161 token is parsed for shape only, never verified — the module comment assumes a check that doesn't happen | `pdf/timestamp.rs:236-277` |
| M9 | `secure readonly` + Secure Plus silently drops the requested permissions (stores `Permissions::all()`); dialog doesn't say so | `pdfium_doc.rs:2179`; `main.rs:5092-5112` |

## Low / Info

| # | Issue | Location |
|---|---|---|
| L1 | Replay can write arbitrary paths: `record ../../x` → `fs::write("../../x.json")` (also `saveas`/`extract` from scripts, no confirmation) | `main.rs:6772-6773` |
| L2 | Windows `signtool` receives the PFX password in argv (visible in process list/logs); macOS flow correctly uses a keychain profile | `packaging/windows/build.ps1:27-30` |
| L3 | `install.sh` runs `rm -rf "$APP"` on an env-derived path without validation | `packaging/macos/install.sh:15-16, 33-34` |
| L4 | Document passwords, PKCS#8 keys and the p12 passphrase are held unzeroized (`Option<Vec<u8>>`, `String::clear()`), inconsistent with `crypto::cipher::Secret` | `pdfium_doc.rs:168-180`; `sign.rs:43-88`; `main.rs:607` |
| L5 | Every open reads the entire file into RAM for the 7-byte `/Pagify` probe before PDFium streams it | `pdfium_doc.rs:219` |
| L6 | `rustybuzz` 0.20.1 and `ttf-parser` 0.25.1 are unmaintained (RUSTSEC-2026-0206/-0192) but parse document fonts | `cargo audit` |
| L7 | No `cargo-deny`/CI gate, no `third_party/CHECKSUMS.sha256`; committed fonts/icons/PDFium have no in-repo digest | repo-wide |
| L8 | `recent.json`, `predefined.json`, `signatures.json` persist paths, snippets and drawn signatures in plaintext, no 0600, not removed on uninstall (Info) | `recent.rs`, `predefined.rs`, `signatures.rs` |
| L9 | PDFium pin `151.0.7881.0` (released 2026-06-08, ~96 days old) — reasonable for a pinned release line; revisit when `pdfium-render` adds a newer feature (Info) | `third_party/pdfium/*/VERSION` |

## Verified sound

- **No injection/network:** no `Command`/shell/`osascript` anywhere; no socket except `timestamp.rs`; no `reqwest`/`hyper`/`rustls` in the lock; no telemetry.
- **Standard PDF encryption:** R6/AES-256 only (RC4 and AES-128 not offered); Algorithm 2.B matches the qpdf reference; empty and >127-byte passwords refused; permissions honestly documented as advisory.
- **Lock/vault:** Argon2id + random DEK, SM4-GCM with fresh nonce, header authenticated as AAD, tag checked before returning plaintext.
- **FF1:** NIST SP 800-38G samples pass; input floors enforced; non-standard use documented.
- **Vendored `pdfium-render` fork:** exactly one changed file (`pub(crate) fn handle` → `pub fn handle` + doc comment); all 4 committed PDFium slices byte-identical to the official release; OCR `.rten` models match pinned SHA-256s and reject substituted copies; no registry crate except `rsa` matches any active RustSec range; no build script downloads or shells out.
- **`mac_open.rs`**, registry lock ordering, render size caps, and the temp+fsync+rename save design are sound; redaction of native text does genuinely remove content and forces a full-copy save (the gap is only the Form/automatic path, C2/H1).

## Prioritized remediation

1. **P0 — stop the false assurances:** verify CMS signatures (C1); make redaction and `hiddendata` fail closed and never report removal unless proven (C2, H1); make `certify` save the signed bytes or say plainly it can't (H2); fix or remove `secure readonly` in Secure Plus (M9) and `unsecure` persistence (M1).
2. **P1 — crash and DoS hardening:** apply the six clamps (H3), add `catch_unwind` around document operations, clamp Argon2 params from documents (H4).
3. **P2 — supply chain and file safety:** checksum/attestation verification for PDFium (H5), pin `cad_kernel` to a commit (M7), `create_new`+randomized staging and mode preservation (M2), atomic `extract_to` (M3), negative vault caching (M5), `rsa` mitigation policy (M6), add `cargo-audit`/`cargo-deny` to CI (L6/L7).
4. **P3 — hygiene:** validate `record` names (L1), fix Windows signing argv (L2), guard install paths (L3), zeroize secrets (L4), bounded open probe (L5), HTTPS/verify timestamps (M8), document state-file locations and add a clear-history control (L8).

## Caveat

This is a source-level audit with targeted proof-of-concept probes, not a fuzzing campaign or a full penetration test — several findings (C1, C2, H2, H4) were reproduced against fixtures by the auditors, and the code paths for all Critical/High items were re-verified statically. No repository files were modified during the audit itself.
