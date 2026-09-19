# Security Audit — Pagify Desktop

- **Date:** 2026-09-19
- **Target:** `pagify-desktop-mac` @ `c478790` (working tree as of the audit date).
- **Supersedes:** `SECURITY_AUDIT.md` (2026-09-12, @ `1b36bee`), which remains as the baseline record of what was found and fixed.
- **Scope:** `desktop/` (`pagify_app`, `pagify_shell`, `pagify_issue`), reachable `rust/pdf_core` (shared with Android/iOS), vendored `pdfium-render` fork, PDFium loading and packaging scripts, committed binaries, dependency lockfiles.
- **Method:** the previous audit's findings were used as a regression baseline; four parallel source-review workstreams (cryptographic/signature core, untrusted-PDF parsing, filesystem/session/app surface, supply chain/packaging) plus the coordinator's independent re-verification of every Critical/High claim. Findings marked **reproduced** were demonstrated with throwaway probes under `/tmp` (no repository files were modified). Crypto unit and integration suites were executed against the vendored Linux PDFium (102 unit + 46 integration tests pass); `cargo audit` run against both lockfiles; every shipped binary hashed and compared with upstream; all four PDFium slices diffed byte-for-byte against the official `chromium/7881` assets.
- **Not covered:** fuzzing campaign of the custom lexer/reader or PDFium; dynamic testing of the packaged/signed app; macOS entitlement penetration test; mobile (Android/iOS) runtime testing.

## Executive summary

| Severity | Count | Headline |
|---|---|---|
| Critical | 1 | Signature "unchanged since it was signed" is still forgeable — `/ByteRange` is never bound to the signature's own `/Contents` |
| High | 2 | Attacker-chosen object number drives unbounded xref emission (uncatchable abort); OCR rasterises untrusted page dimensions with no cap |
| Medium | 6 | Exponential page-tree traversal; non-atomic redaction apply; unbounded `/Filter` chain; `hiddendata` misses XFA/AcroForm; signing placeholder located by scanning untrusted text; Windows/Linux releases bypass every supply-chain gate and there is no CI |
| Low / Info | 15 | Script replay overwrites files unconfirmed; transient world-readable writes; unauthenticated vault metadata; residual unzeroized secrets; and similar |

**The previous audit's two Criticals are addressed as specific bugs, but the signature-forgery class is reopened by a different gap (C-1).** The cryptographic *primitives* remain in excellent shape: SM2/SM3 verified against GB/T standard vectors, SM4-GCM through RustCrypto's generic AEAD with fresh random nonces and tag-before-plaintext, Argon2id costs clamped before derivation, Secure Plus now binds whole-file structure, no network code anywhere, and no process execution in any production path. The serious problems have moved from *false security claims* to **input-driven DoS in the custom PDF writer and the paths added since the last audit**, plus release-process gaps.

---

## Critical

### C-1 — `Unaltered` is forgeable: `/ByteRange` is never tied to the signature's own `/Contents`

**Where:** `rust/pdf_core/src/pdf/validate.rs:260-277`; `rust/pdf_core/src/pdf/sign.rs:187-192`.

`validate` takes `/ByteRange` from the parsed signature dictionary, checks only that the numbers do not overflow and are ordered, then digests `bytes[..first] ++ bytes[second_at..second_at + second_len]` and verifies the CMS signature over the signed attributes. It never checks the ISO 32000-2 rule that the skipped hole `[first, second_at)` must be the `/Contents` value of *that same* signature dictionary, nor that the dictionary itself lies in the covered bytes. `sign.rs::covers_everything()` enforces exactly that at signing time but is never called from `validate` (it is referenced only in `sign.rs`).

**Attack (no key required):** take any validly signed Pagify PDF, split it at the start of the `/Contents` hole into `C1` and `C2`, and insert an attacker region `H` between them containing (a) a shadow `/Sig` dictionary carrying the *original* CMS and a `/ByteRange` that skips `H`, and (b) a new xref/trailer positioned at the offset the unchanged final `startxref` still points to. The covered bytes are byte-identical to the original `C1 ++ C2`, so digest and signature still verify, while the parsed object graph and the rendered document are attacker-chosen.

**Reproduced:** signing `two-column.pdf` with the test leaf, replacing the page's `/MediaBox` with 400×400 in `H`:

```
validate on forged file: 1 signature(s)
  verdict=Unaltered signer=Some("CN=Pagify Test Leaf (leaf),O=Pagify") trust=Some(Pinned) is_good=true
PDFium opened it: 1 page(s)
PDFium page 0 size = PageSize { width_pt: 400.0, height_pt: 400.0 }
```

In the shipped build `trust/roots.der` is empty, so a forged document reads `Unaltered` + `Unrecognised` rather than a full green tick; once HSI pins its production root, the same file earns `Unaltered` + `Pinned` and `is_good()` returns true. In both cases the UI's sentence "unchanged since it was signed" (`validate.rs:91`) is false.

**Fix:** require the hole to be exactly the `/Contents` token of the dictionary being checked — locate that object's byte span via the parser's xref offsets and compare — and require the dictionary itself to lie inside the covered spans. Any `/ByteRange` that does not satisfy the ISO rule for its own signature must be refused, not judged.

---

## High

### H-1 — Rewrite cost is controlled by the highest object number, with unchecked `u32` adds

**Where:** `rust/pdf_core/src/pdf/reader.rs:294-310`; same pattern at `reader.rs:146,392`, `encrypt.rs:492`, `secure_plus.rs:201`, `embed.rs:147`, `pdfium_doc.rs:1500,3706,9878`.

```rust
let highest = written.keys().copied().max().unwrap_or(0);
out.extend_from_slice(format!("xref\n0 {}\n", highest + 1).as_bytes());
for number in 1..=highest { /* one ~21-byte row per number */ }
```

The highest object number comes from the file. **Reproduced:** a 148-byte crafted file produced 200,000,137 bytes in 0.6 s; a 426-byte file through the real `hiddendata clean` entry point (`hidden.rs:476`) produced 200,000,335 bytes in 7.7 s and reported "there was nothing hidden to remove". With one object numbered `4294967295`, the release build wraps `highest + 1` to 0 (overflow checks off) and enters a 4.29-billion-iteration loop; the probe aborted with `memory allocation of 5033164800 bytes failed` (exit 134) — an allocation failure is an abort, not a panic, so `catch_unwind` cannot contain it. Debug builds panic at `reader.rs:296`. Reachable from every custom edit save (`write_edit`), native redaction, lock/secure/unsecure, and `hiddendata clean` — i.e. editing a crafted document is enough.

**Fix:** bound emitted object numbers (reject or renumber absurd gaps), use `checked_add` throughout, and refuse a rewrite that would exceed a size ceiling derived from the input.

### H-2 — OCR rasterises untrusted page dimensions with no cap

**Where:** `rust/pdf_core/src/ocr/pipeline.rs:112-120`.

`rasterise` computes `width`/`height` from the page's own size and allocates `vec![0u8; width * height * 4]` **before** `RenderTarget::new` can enforce `MAX_DIMENSION_PX`/`MAX_PIXELS` (`render/bitmap.rs:55-68`). Unlike the on-screen and export paths (`render/region.rs:125-142`, `render/viewport.rs:149-155`), nothing clamps here. **Confirmed with a probe against the vendored PDFium:** a hand-written PDF with `/MediaBox [0 0 20000 20000]` opens with `PageSize { 20000.0, 20000.0 }` unclamped; at the default 300 dpi that is ~83k × 83k px ≈ **27.8 GB**, an allocation failure that aborts the process and loses unsaved edits. Triggered by "Extract Text / Make searchable" on a crafted page.

**Fix:** call `validate_dimensions` (or clamp through the same `MAX_PIXELS` budget) before allocating, and refuse a page whose raster would exceed it.

---

## Medium

### M-1 — Exponential page-tree traversal (DAG, no visited set)

**Where:** `rust/pdf_core/src/document/pdfium_doc.rs:8685-8711` (`collect_pages`) and `:7003-7029` (`page_object_number::walk`). Both recurse per `/Kids` entry with only a depth cap (64); a node listing the same child twice at every level costs 2^64 visits or exhausts `out`. Reachable from any path that resolves a page: `page_object`, form-cut planning/apply, annotation counts. The `hidden.rs` walkers use a `seen` set; these do not.

### M-2 — Redaction apply is not atomic; the full-copy flag is set only on success

**Where:** `rust/pdf_core/src/document/pdfium_doc.rs:9373-9444`. Objects are removed/written one by one with `?` propagation; `FPDFPage_GenerateContent` failure returns at `:9429-9431`; `self.redacted = true` only at `:9444`. On a mid-apply error the command layer drops the pre-redaction snapshot with no undo record (`command/mod.rs:507-533`), `must_save_full_copy()` is still false (`:3995-4002`), and `save_incremental` is therefore permitted — leaving the removed content recoverable from the file's earlier revision exactly when redaction is supposed to have destroyed it.

### M-3 — `/Filter` chain length is unbounded; each stage only individually capped

**Where:** `rust/pdf_core/src/pdf/content.rs:788-839`. `filters` is built from the whole `/Filter` array with no count cap; the 128 MiB `INFLATED_LIMIT` is checked after each stage. Nested-deflate input (stage *n*'s output feeding stage *n+1*) lets each of N stages legitimately inflate, so total work is ~N × 128 MiB from a small file. Hit while decoding page content during edit/redaction and font `/ToUnicode` reads.

### M-4 — `hiddendata clean` leaves XFA and AcroForm field values intact

**Where:** `rust/pdf_core/src/pdf/hidden.rs` — there is no `/AcroForm`/`/XFA` handling anywhere in `survey` (`:175-227`) or `strip` (`:409-481`). A file with `/AcroForm << /XFA <stream> /Fields [ … /V (secret) …] >>` survives "clean", and the UI reports the document clean. The surviving script is inert in Pagify (no form-fill/JS environment is ever initialised — verified: no `FPDFDOC_InitFormFillEnvironment`/`FORM_*` calls), but the bytes travel on to readers that do act on them. Either handle AcroForm/XFA or scope the claim honestly.

### M-5 — Signing locates the placeholder and `/ByteRange` by scanning untrusted document text

**Where:** `rust/pdf_core/src/pdf/sign.rs:200-227` (`find_placeholder`: the first `<`, ≥1024 zeros, `>` anywhere), `:881-911` (`write_byte_range`: the first literal `/ByteRange` in the whole prepared file), `:862-867`. **Reproduced:** a crafted file with a decoy `/Type /Sig` dictionary (zeroed `/ByteRange`, 16384-zero hex run) earlier in the file captures both: the finished CMS and the patched numbers are written into the attacker's dictionary, while Pagify's own appended `/Sig` is left zeroed. The user is told the document was signed; the artifact's Pagify-made signature is empty, and the live signature sits in an attacker-shaped dictionary. **Fix:** record the byte offsets when writing them and use those, never rescan.

### M-6 — Windows/Linux releases bypass every gate; no CI exists

- `packaging/windows/build.ps1:13-23` and `packaging/linux/appimage.sh:15-22` copy `pdfium.dll`/`libpdfium.so` with only an existence check. Only macOS `bundle.sh:41-53` runs `verify_third_party.sh` / `audit.sh` / `no_sockets.sh`.
- `tools/fetch_pdfium.sh`: `verify_slice` hashes `$ROOT/third_party/$rel` (`:63,68`) even when extraction went to a custom `$DEST` (`:103`), so a custom-destination fetch prints "verified" while leaving unverified files; a slice whose directory already exists is skipped without hashing (`:90-93`). The downloaded archive itself is extracted before any hash check (`:97-104`).
- `bundle.sh:47` audits `Cargo.lock`, then `:67` builds with no `--locked`, so a manifest change can pass the gate and update the lock during the build. Same in `build.ps1:17`, `appimage.sh:17`.
- No workflow/CI configuration exists anywhere in the repository, so none of these gates runs on a pull request or on Windows/Linux at all.
- `bundle.sh:121-124` exits 0 "signed but not notarized"; `PAGIFY_LOCAL_UNSIGNED_BUILD=1` inherited in a release shell produces a fully unsigned bundle.
- `no_sockets.sh:46-47` fails open when `cargo tree` errors (empty output is treated as success) and is a name blocklist only.

---

## Low / Info

| # | Severity | Where | Issue |
|---|---|---|---|
| L-1 | Low | `pagify_app/src/main.rs:8596-8619` | A hand-written `replay` script runs `saveas`/`extract` with no confirmation (unlike the file dialog) — a script can silently overwrite arbitrary files — and `replay` of a script naming itself recurses with no depth guard. |
| L-2 | Low | `pagify_shell/src/session.rs:91-99`, `state.rs:56-60`, `outlined_fonts.rs:109` | New-destination saves land at `0666 & ~umask` (0644) even from a 0600 source, because permissions are copied only when overwriting; staging files and first-created state files are 0644 for the duration of the write (`signatures.json` holds signature pixels). `outlined_fonts.json` uses a bare `fs::write`, is not 0600, and is missed by `clearhistory`. Reproduced under `umask 022`. |
| L-3 | Low | `pagify_app/src/main.rs:2878-2906` | Signature-image upload decodes PNG/JPEG with no dimension limits; `image` 0.25 caps decoder allocation at 512 MB but not dimensions, and post-decode orientation/RGBA copies add more. |
| L-4 | Low | `rust/pdf_core/src/pdf/cmap.rs:86-91` | `out.insert(low + offset as u32, text)` with `low` up to `0xFFFFFFFF`: debug panics (contained), release wraps and can skew redaction alignment. |
| L-5 | Low | `rust/pdf_core/src/pdf/hidden.rs:262-275` + `reader.rs:469-478` | A stream whose dictionary carried a script action is collected as changed and rewritten as a dictionary only — the bytes are dropped. Sanitising can blank page content while reporting clean (fail-safe direction, corrupting outcome). |
| L-6 | Low | `rust/pdf_core/src/document/pdfium_doc.rs:3413-3431` | `validate_signatures` reports on `self.written` or the on-disk file; annotations and PDFium edits do not refresh it, so the pre-save readout can describe a stale file. |
| L-7 | Low | `rust/pdf_core/src/crypto/vault.rs:120-175`, `crypto/ledger.rs` | Vault/ledger JSON metadata (`pages`, `items`, `document_id`, `written`) is unauthenticated; only the sealed blobs are AEAD-bound (to `v, page_index`). A file-write attacker can delete recovery copies or make a wrong-value restore look like a normal unlock. Secure Plus got `/Bind`; the vault did not. |
| L-8 | Low | `crypto/envelope.rs:48-56`, `crypto/kdf.rs:115-117` | `Envelope.salt` length is unbounded (only `< 8` rejected); a multi-megabyte hex salt is hashed by Argon2 on every unlock. |
| L-9 | Low | `crates/pagify_issue/src/main.rs:447-457,406` | The issuing tool writes the root key, issued `.p12`s, serial and ledger with `create_new` but no narrowed mode, and no `sync_all` before the "backup verified" read-back. Dev-only tool; not shipped in the app. |
| L-10 | Low | `pagify_shell/src/session.rs:341`, `pagify_app/src/main.rs:2354,6857-6863`; `pdf/encrypt.rs:136`; `sign.rs:57-58` | Residual plain-`String` copies of passwords/passcodes, `Security.file_key` without zeroize, and `zeroize::Zeroizing` **derives `Debug`**, so `Wanted`/`opened_with` would print raw password bytes if ever formatted. No live print site found — hibernating exposure. |
| I-1 | Info | `rust/pdf_core/src/pdf/reader.rs:36-46` | `/Prev` chain cycle check is `Vec::contains` — quadratic on a file with many xref sections. |
| I-2 | Info | `rust/pdf_core/src/document/pdfium_doc.rs:480-496` | Any file whose last 1 MiB contains `/Pagify` is read whole at open (memory-pressure nuisance, no amplification). |
| I-3 | Info | `pagify_shell/src/pdfium.rs:76-77` | The loader still falls back to the compile-time dev tree when no library sits beside the executable; nothing verifies that path at load time. Same-user attack at worst; hardened runtime blocks unsigned overrides in signed builds. |
| I-4 | Info | `desktop/packaging/` | No uninstaller exists; deleting `Pagify.app` leaves `~/Library/Application Support/Pagify` (recent paths, snippets, signatures, scripts) behind. `clearhistory` covers `recent.json` only. |
| I-5 | Info | `rust/pdf_core/src/pdf/trust.rs:46-54,205-219` | No validity-date or extension (CA/KeyUsage) checks on the signer chain — documented as deliberate (no trusted time), but the residual risk should accompany any green tick. |
| I-6 | Info | `rust/pdf_core/src/pdf/validate.rs:153-168,222` | Exact `/Type /Sig` matching and first-`SignerInfo`-only can disagree with PDFium's `signature_count()`. Not a false "valid". |
| I-7 | Info | `app/src/main/AndroidManifest.xml` | `android:allowBackup="true"` on Android (shared `pdf_core` only; no network permission, FileProvider not exported). |
| I-8 | Info | `rust/pdf_core/Cargo.toml`, both lockfiles | `rustybuzz 0.20.1` / `ttf-parser 0.25.1` are unmaintained (RUSTSEC-2026-0206/-0192) but parse document fonts; accepted with a documented migration plan, no time bound. |
| I-9 | Info | `rust/pdf_core/src/document/pdfium_doc.rs:304-312` | With a memory-backed document source, `write_exact_if_unchanged` cannot preserve signed bytes — Android/iOS latent gap; desktop always opens by path. |

---

## Verified fixed (previous audit regression status)

| ID | Status | Evidence |
|---|---|---|
| C1 | **Mechanism fixed; property reopened as C-1** | `validate.rs:280,358-409`: real SM2 verify over re-encoded signed attributes; OpenSSL-generated blob verifies; re-hash forgery now `Verdict::Invalid` (`tests/signing.rs:384-418`). The `/ByteRange` binding gap is C-1. |
| C2 | Fixed | `redact.rs:377-387` includes Form/OutlinedText and is non-overridable; `smartredact` counts only blocker-free items (`main.rs:4828-4882`); save-reopen-byte-scan tests (`tests/redaction.rs:311-350`). |
| H1 | Fixed | `hidden.rs:345-367,421-437,473-480` strips Names/JS/EmbeddedFiles/AA/OpenAction/Metadata/Info and re-surveys the cleaned bytes; acceptance test scans saved bytes (`tests/hidden.rs:86-142`). |
| H2 | Fixed | `write_exact_if_unchanged` / `save_full_copy` (`pdfium_doc.rs:295-318,4382`); reproduced: `a_signed_document_saved_is_the_signed_bytes`, `an_edit_after_signing_is_saved_as_a_revision…` (`tests/signing.rs:575-645`). |
| H3 | Fixed (all six) | mark-name clamp `:11942`; byte-based hex decoding `envelope.rs:161-180`; colour hex check `:11473-11484`; lexer `MAX_DEPTH=256` `object.rs:110-127`; `checked_add` byte range `validate.rs:264-272`; 128 MiB inflate cap `content.rs:829-862`. Panic containment moved into the shared core: `registry::contained` wraps every session call (`registry.rs:152-166`); `panic = "unwind"` retained. |
| H4 | Fixed | `KdfParams::check` inside `Argon2id::derive` (`kdf.rs:64-81,118-121`); `/M 4294967295` refused in <2 s (test). |
| H5 | Fixed (desktop) | `fetch_pdfium.sh:22-33,54-75` pins the tag and hashes each library against `third_party/CHECKSUMS.sha256`; all four slices byte-identical to upstream `chromium/7881`; `PAGIFY_ROOT` fallback gone; OCR models SHA-256-pinned at runtime (`models.rs:110-130`). Mobile fetch scripts remain unverified (out of desktop scope). |
| M1 | Fixed | `unsecure_document` / `rearm_security` (`pdfium_doc.rs:3335-3353,6586-6601`); incremental save refused while the password is coming off (`:4447-4453`); tests `tests/secure.rs:162-218`. |
| M2 | Fixed | Random sibling staging with `create_new`, symlink refused, permissions copied from an existing target (`session.rs:46-115`); reproduced (`round_trip.rs:253-279` + probe). Residual mode gap for *new* destinations is L-2. |
| M3 | Fixed | `extract_to` goes through `write_then_rename` (`session.rs:1317-1334`); `round_trip.rs:283-301`. |
| M4 | Fixed | SHA-256 over every object's written bytes plus trailer `/Root`, sealed as `/Bind`, verified before trust (`secure_plus.rs:229-278`); alteration test passes. |
| M5 | Fixed | `read_vault` caches both success and error results (`pdfium_doc.rs:7060-7075`). |
| M6 | Fixed | `rsa` absent from both lockfiles; `cargo audit` clean; `sign.rs` is SM2-only. |
| M7 | Fixed | `cad_kernel` pinned to `rev = "2f6c8a9e…"` in `desktop/Cargo.toml:45` and recorded in `Cargo.lock`. |
| M8 | Fixed by removal | No timestamp module and no socket anywhere (source + dependency graph); TSTInfo tokens are declined as `Unreadable`, never judged (`validate.rs:225-229`). |
| M9 | Fixed | Secure Plus carries no permissions (`pdfium_doc.rs:3306-3329`); the UI refuses `secure readonly` under it (`main.rs:6754-6765`); test at `main.rs:16937`. |
| L1 | Fixed (validation); replay confirmation deliberately omitted | `state::file_name_only` (`state.rs:33-48`), validated at start and at write; the replay-overwrite behaviour is tracked as L-1. |
| L2 | Fixed | Certificate store + `SIGNING_THUMBPRINT` (`build.ps1:25-48`); PFX password flows refused. |
| L3 | Fixed | `install.sh:21-55` validates an absolute destination and only removes a Pagify bundle. |
| L4 | Partially fixed | Keys/`Wanted` zeroized (`sign.rs:58`, `pdfium_doc.rs:99-110`); residual copies are L-10. |
| L5 | Fixed | `tail_says_ours` probes the last 1 MiB (`pdfium_doc.rs:448-496`). |
| L7 | Partial | `third_party/CHECKSUMS.sha256` + `verify_third_party.sh` fail closed, but nothing runs them in CI and only macOS invokes them (M-6). |
| L8 | Partial | State files under one folder at 0600 (`state.rs:52-63`) except `outlined_fonts.json`; no uninstaller (L-2, I-4). |

**Baseline checks re-confirmed sound:** no network code in source or in the dependency graph of the shipped binaries (`no_sockets.sh` pass, lock has no network/TLS/async crates); no `Command`/shell execution in production code; no document-driven file writes or script execution; models pinned; every FFI/JNI export panic-contained; pixel-format handling, registry lock ordering and cache caps intact.

### Cryptography notes

- **SM2/SM3:** standard algorithm with the pinned distinguishing ID `1234567812345678`; `ZA` computed per GB/T 32918.5; zero/out-of-range `r,s` and loose DER rejected (`sm.rs:73-95`); standard worked examples pass (`sm.rs:117-159`, `validate.rs:670-696`).
- **Trust:** issuer-and-serial-keyed denylist, chain checked by real SM2 signature over `tbsCertificate`, lookalike root rejected, and the empty shipped `roots.der` fails closed to `Unrecognised` (`trust.rs:128-219`).
- **SM4-GCM:** RustCrypto generic AEAD, 96-bit nonce from `OsRng` per message, header authenticated as AAD, tag verified before any plaintext is returned (`cipher.rs:46-103`, `envelope.rs:68-106`); SM4 known-answer and million-iteration vectors pass.
- **Argon2id:** costs clamped (1 GiB / 10 passes / 16 lanes) in one place before any allocation; KDF identity written and never inferred.
- **FF1:** NIST SP 800-38G samples pass; tweak/key handling reviewed.
- **No RSA**, no sockets, no timestamps.

---

## Priorities

1. **P0 — signature correctness:** bind `/ByteRange` to the signature dictionary's own `/Contents` (C-1); stop locating the placeholder and `/ByteRange` by scanning (M-5).
2. **P0 — input-driven DoS:** bound xref emission and object numbers in the custom writer (H-1); cap OCR raster dimensions before allocating (H-2).
3. **P1 — feature integrity and process:** visited-set in page-tree walks (M-1); atomic redaction with the full-copy flag set on any failure (M-2); cap `/Filter` stage count (M-3); handle or refuse AcroForm/XFA in `hiddendata` (M-4); run the integrity/advisory/socket gates on Windows and Linux, add CI, and build with `--locked` (M-6).
4. **P2 — hardening:** confirm or constrain script-driven `saveas`/`extract` and depth-cap `replay` (L-1); `create_new` + 0600 staging and mode inheritance for new destinations, and 0600 for `outlined_fonts.json` (L-2); image decoder limits (L-3); checked arithmetic in `cmap.rs` (L-4); MAC vault/ledger metadata or document the threat (L-7); bound the envelope salt (L-8); `pagify_issue` modes and fsync (L-9); finish the zeroization tail (L-10).
5. **P3:** fuzzing harness (cargo-fuzz/AFL++) for the custom lexer, reader, content decoder and redaction paths; `cargo-deny`; uninstaller/`clearhistory` completeness; a plan with dates for the unmaintained font parsers.

## Caveats

This is a source-level audit with targeted proof-of-concept probes, not a fuzzing campaign or a full penetration test. Critical/High items were re-verified by the coordinator against source; where a finding is marked reproduced it was demonstrated under `/tmp` against the repository's fixtures and vendored PDFium. The PDF parsing workstream read but did not execute all PDFium-backed integration suites; the crypto workstream executed the signing/trust/secure suites against the Linux PDFium (148 tests passing). The C-1 reproduction used test anchors to reach `Pinned`; a shipped build with an empty `roots.der` reports `Unaltered` + `Unrecognised`, which is still a false statement about the file's contents and becomes a full green tick once a production root is pinned. No repository files were modified during the audit itself.

---

# Remediation status — branch `security-fixes-2026-09-19`

Every Critical, High and Medium finding above was fixed, together with most Low items. Each fix carries a regression test where one is possible. Nothing was committed; the work is in the working tree of the branch.

| Finding | Fix | Regression test |
|---|---|---|
| C-1 | `validate.rs`: the `/ByteRange` hole must be exactly the signature dictionary's own `/Contents` (object span from the file's xref; hex token compared to the parsed value; bytes on both sides). `HoleBinding` threaded through `check_with`. | `tests/signing.rs::a_signature_spliced_around_an_inserted_region_is_refused` (fails with the binding disabled, passes with it) + two `validate.rs` unit tests |
| H-1 | `reader.rs`: xref emitted as one subsection per run of consecutive numbers (object 0 free, gaps absent); `first.checked_add(index)`; `append_revision` and all `max()+1` object-number sites (`sign`, `encrypt`, `secure_plus`, `embed`, five in `pdfium_doc`) use `checked_add`. | `reader.rs`: huge-object-number rewrite stays under 4 KiB; `4294967295 2` subsection refused, not wrapped |
| H-2 | `ocr/pipeline.rs`: `validate_dimensions` before the raster allocation. | `ocr::pipeline` unit test with a 20 000 pt page → `RenderTooLarge` |
| M-1 | `pdfium_doc.rs`: `HashSet<u32>` visited sets in both page-tree walks. | Unit tests: 41-level doubling tree visits once; shared-subtree DAG yields each page once |
| M-2 | `pdfium_doc.rs`: `redacted = true` + `touch()` before the first removal, so any mid-apply failure still forces a full copy and marks the document dirty. No rollback (snapshot lives in the command layer). | Existing `undoing_a_redaction_does_not_re_permit_an_incremental_save`; no deterministic error-path trigger exists |
| M-3 | `content.rs`: `MAX_FILTER_STAGES = 8` and a cumulative inflated-byte budget across stages (`decode_checked`; `decode` maps refusals to `None` as before). | Stage-count and cumulative-budget unit tests |
| M-4 | `hidden.rs`: survey reports `/XFA` and field `/V` values; strip removes `/XFA` and every `/V` reachable from `/Fields`/`Kids`, keeping the fields; the post-clean survey still measures what survived. | `tests/hidden.rs` XFA/fixture test; suite 30/30 |
| M-5 | `sign.rs`: placeholder and `/ByteRange` located strictly inside the signature object, via its xref span. | `tests/signing.rs::a_decoy_signature_dictionary_does_not_capture_the_signature` |
| M-6 | `fetch_pdfium.sh` hashes the extracted file (custom `$DEST` included) and verifies pre-existing slices; `build.ps1` verifies each packaged DLL against `CHECKSUMS` with `Get-FileHash`; `appimage.sh` runs verify/audit/no_sockets; `bundle.sh`, `appimage.sh`, `build.ps1` build `--locked`; `no_sockets.sh` fails closed; new `.github/workflows/desktop-security.yml` runs all gates plus unit tests, fetching the pinned slices (cache keyed on `CHECKSUMS`) before strict verification. | Stub-`curl` reproduction (old accepts tampered custom DEST, new refuses and deletes only its own slice); `bash -n`; YAML parse; action pins checked against the GitHub API |
| L-1 | `main.rs`: `replaying` re-entrancy guard; a nested or self `replay` is refused and the script stops; `saveas`/`extract` still run unconfirmed, as designed. | Self-referencing script test |
| L-2 | `session.rs`: staging opened 0600; final mode = existing target's, else the source document's, else 0600; `state.rs` and `outlined_fonts.rs` create 0600 from the first byte and share the state directory. | Unix mode tests in both crates (probe previously showed 0644) |
| L-3 | `main.rs`: `image::Limits` (16 384 px, 256 MiB) applied before decoding an uploaded picture. | Existing picture/not-a-picture tests |
| L-4 | `cmap.rs`: `checked_add` on both `bfrange` branches; overflow refuses the map. | Unit test with `low = 0xFFFFFFFE` |
| L-5 | `hidden.rs`: changed streams are re-emitted with `write_stream`, keeping their bytes. | `tests/hidden.rs` stream-survives-clean test |
| L-6 | `pdfium_doc.rs`: `touch()` drops `written`; `touch_annotation()` clears `exact_pending`; `validate_signatures` answers only when the bytes are current (`exact_pending` or `!dirty`), otherwise "unsaved changes". Sign→save still writes the signed bytes verbatim. | New `tests/signature_freshness.rs`; signing suite 22/22 |
| L-8 | `kdf.rs`: salt ceiling of 1024 bytes before derivation. | Unit test |
| L-9 | `pagify_issue`: root key, `.p12`, serial and ledger created 0600 with `sync_all`. | Unix mode test |
| L-10 | `main.rs`: `SecretText` (zeroizing, redacting `Debug`) for confirmation state and the typed passcode; `session.rs` wipes its password buffer; **`encrypt.rs` `Security` now zeroizes `file_key`, wrapped keys and verifiers on drop** (fixed during remediation). | Debug-redaction test; `cargo check` |
| I-1 | `reader.rs`: `/Prev` cycle set is a `BTreeSet`. | Existing loop test |

**Verified locally after the fixes:** `pdf_core` lib 660 passed / 1 pre-existing environment failure (`crypto::tweak` font path) / 1 ignored; all PDFium integration suites green (signing 22, signatures 8, trust 8, secure 18, redaction 35, hidden 30, hostile 2, round_trip 25, layers 14, moving 14, transform 3, signature_freshness 2, lock 6, region_export 9, text_extraction 24, text_selection 6, typing 3, outlined_type 8, ocr_engine 4) except two pre-existing `image_signature` rotation failures, confirmed identical on the baseline file; `pagify_shell` lib 190 passed; `pagify_issue` 6 unit + 6 CLI passed; `pagify_app` checks with tests.

## Deferred

- **L-7 — vault/ledger metadata authentication.** Deferred with a specific blocker rather than half-fixed. A key-bound MAC over `pages`/`items`/`fields` can only be produced when the data key is at hand, but `repair_locks` legitimately writes the vault **without a passcode** (it drops stale badges), and an attacker who can write the file can strip an optional MAC and the `v` marker with it — an optional integrity field is no field at all. The proper fix needs a product decision: either require the passcode for `repair_locks`, or carry the metadata inside something the envelope already authenticates. The ledger half is latent in any case: `crypto::ledger` is not yet used by any product path. Recommended follow-up, with a format version bump and migration test.
- **I-level items** (full-file read on a `/Pagify` tail marker, dev-tree loader fallback, uninstaller/state retention, trust-chain date/extension policy, exact `/Type /Sig` matching, mobile fetch scripts, `allowBackup`) are unchanged and remain accepted risks as documented above.
- **P3** (fuzzing harness, `cargo-deny`, unmaintained font-parser plan) is not started.
