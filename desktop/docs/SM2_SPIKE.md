# SM2/SM3 signatures — the phase 0 spike, and what it found

Written 15 September 2026 against `pagify-desktop-mac`, answering the checklist in
§5 of `PAGIFY_SM_SIGNATURES_PLAN.md` ("The spike, precisely"). The output the plan
asked for — a fixture signed with SM2 that `validate.rs` verifies, and a note on
which route step 2 needed — is `rust/pdf_core/fixtures/sm2-signed.pdf` and this file.

**The spike passed.** Phases 1–5 can be planned in detail.

*Status:* phases 1 and 2 landed on 15–16 September 2026 — `git log --grep="SM signatures"`
lists the commits. The "what it means for the phases" section below is the note as
written at the end of the spike, kept as the record of what the spike found.

## The checklist, answered

| # | Step | Answer |
|---|---|---|
| 1 | Pin `sm2` 0.13.3 and `sm3` 0.4.2 | Pinned. Both sit on `signature 2.2` / `digest 0.10` / `elliptic-curve 0.13` / `spki 0.7` / `pkcs8 0.10` — the generation the existing `cms` 0.2, `x509-cert` 0.2, `sha2` 0.10 stack is on. `cargo fetch` resolved cleanly with no second copy of any RustCrypto trait crate. The 0.14 release candidate is in the local registry cache but not in the graph. |
| 2 | Check both traits the builder needs | **Both missing → the hand-built `SignerInfo` route.** `sm2::dsa::Signature` implements `SignatureEncoding` only, not `SignatureBitStringEncoding` (no `to_der` either). `sm2::dsa::SigningKey` implements `Signer`, `PrehashSigner`, `RandomizedSigner`, `KeypairRef` — not `DynSignatureAlgorithmIdentifier`. Confirmed by reading `sm2-0.13.3/src/dsa.rs` and `src/dsa/signing.rs`, not by trying to compile against the builder. |
| 3 | Confirm the identifiers, read back from the DER | SM3 `1.2.156.10197.1.401`, SM2-with-SM3 `1.2.156.10197.1.501`, the curve `1.2.156.10197.1.301` in the certificate's key. Read back by `openssl asn1parse` on the finished fixture (it prints them as `sm3`, `SM2-with-SM3`, `sm2`) and by the test `an_sm2_signature_names_the_sm_algorithms_by_their_oids`. |
| 4 | Fix the distinguishing ID and pin it in code | `pdf::sm::DISTINGUISHING_ID = "1234567812345678"`, read by `sign.rs` (`Identity::sm2_key`) and `validate.rs` (`sm2_key_of`) — nothing else names it. The GB/T 32918.5-2017 worked example uses the same ID, so its signature verifies under the constant (`the_standards_sm2_signature_verifies_under_the_pinned_identity`), and fails under `ALICE123@YAHOO.COM`, which is the reason the constant exists. |
| 5 | Make the test `.p12` with OpenSSL 3.6 | `fixtures/test-signer-sm2.p12`, password `pagify`, self-signed, SM2-with-SM3. The command is in `fixtures/README.md`. `Identity::from_pkcs12` reads it unchanged — PKCS#12 carries the key as PKCS#8 with the SM2 curve OID, and `sm2::SecretKey::from_pkcs8_der` checks that OID itself. |

## The route step 2 needed, exactly

`cms::builder::SignerInfoBuilder` is generic over the signer and asks it two things:
how to name its algorithm (`DynSignatureAlgorithmIdentifier`) and how to write its
signature as a `BIT STRING` (`SignatureBitStringEncoding`). The `sm2` crate answers
neither, so `sign.rs` assembles the `SignerInfo` and the `SignedData` around it from
the same `cms` structs the builder fills in — `sm2_detached_signature`, about sixty
lines. What it writes is the builder's shape exactly:

- `SignedData` v1; `digestAlgorithms = { SM3 }`; `id-data` with no content
  (detached); the identity's certificates carried; one `SignerInfo` v1 named by
  issuer and serial.
- The two signed attributes RFC 5652 requires — `contentType` and
  `messageDigest` — made with the builder's own `create_content_type_attribute`
  and `create_message_digest_attribute`, so nothing about them is new.
- The signature is over the signed attributes in DER as a `SET` (what
  `validate.rs` already recomputes for RSA), made with `Signer::try_sign` — the
  crate folds `ZA` in and hashes with SM3 itself. It is written as
  `SEQUENCE { INTEGER r, INTEGER s }` in an `OCTET STRING`, the ECDSA-Sig-Value
  shape; `pdf::sm::signature_to_der` / `signature_from_der` are the only code that
  touches that encoding, and they are strict DER both ways.
- SM3's `AlgorithmIdentifier` carries an explicit `NULL` parameter, SM2-with-SM3's
  carries none — as OpenSSL writes them. `validate.rs` matches on the OID alone.

Two things the plan did not say that phase 1 needs to know:

1. **The file digest changes too, not only the signature.** The `messageDigest`
   attribute must be under the digest algorithm the `SignerInfo` names, so an SM2
   signer commits to **SM3** over the byte range, not SHA-256. `sign::sign` now
   picks `sm3_of` or `digest_of` by the key; phase 1 collapses that to SM3.
2. **The `sm2` crate's `Signer` is deterministic** (RFC 6979 nonces), so signing
   the same bytes twice gives the same blob. Harmless — and it is why a sign-side
   known answer from the standard, which uses a fixed random `k`, cannot be
   reproduced; the standard's vector is checked verify-side only.

## What verifies what

| Made by | Checked by | Result |
|---|---|---|
| `sign::sign` with the SM2 identity | `validate::check` | `Unaltered`, signer `CN=Pagify SM2 Test Signer,O=Pagify` |
| `sign::sign` with the SM2 identity | OpenSSL 3.6 `pkeyutl -verify -digest sm3 -pkeyopt distid:1234567812345678` over the extracted signed attributes; Python `hashlib.new('sm3')` over the covered bytes | Verified; fails under any other `distid` |
| OpenSSL 3.6 `cms -sign -md sm3 -keyopt distid:…` (`fixtures/openssl-sm2.cms`) | `validate::check` | `Unaltered`, same signer |
| GB/T 32918.5-2017 Annex A.2 (the standard's own signature) | the `sm2` crate under the pinned ID | Verifies |
| GB/T 32905-2016 Annex A (`abc`, `abcd`×16) | the `sm3` crate | The standard's digests |

Each direction is a test. The last two are the known-answer tests §10 of the plan
asks to keep permanently; they live in `pdf/sm.rs`.

**One tool that cannot be used:** `openssl cms -verify` hashes SM2 with an *empty*
distinguishing ID — there is no option to set one on the verify side — and fails
even OpenSSL's own `cms -sign` output. Its failure means nothing about a blob; the
primitive-level check above is the independent one.

## What changed in the code, and what it means for the phases

Added, all under `rust/pdf_core`:

- `src/pdf/sm.rs` — the three constants, the two encoders, the known-answer tests.
- `sign.rs` — `Identity::sm2_key`, `Identity::is_sm2`, `sm2_detached_signature`,
  `sm3_of`; `detached_signature` and `sign` route SM2 keys to them. **RSA is
  untouched** — the spike proves the SM2 path beside it; phase 1 removes RSA.
- `validate.rs` — `Hash::Sm3`, `Scheme::Sm2`, `sm2_key_of`; `verify` now asks the
  scheme whether a certificate's key can be tried, so an SM2 signature under an
  RSA certificate is `Unreadable("… not an SM2 key")`, never `Invalid`. **RSA
  verification is untouched**; phase 2 removes it and maps the leftover OIDs to
  words.
- Fixtures: `test-signer-sm2.p12`, `sm2-signed.pdf`, `openssl-sm2.cms`, and
  `rsa-signed.pdf` — the last frozen now, before phase 1, because it is the
  third-party document phase 2's fail-closed test needs and nothing here will be
  able to make one afterwards.
- `examples/sign_probe.rs` takes `P12=<path>` and `P12_PASSWORD`.

Phase 1 is therefore smaller than estimated: the SM2 signing path exists and is
tested; what remains is deletion (RSA, `timestamp.rs`, the verb, `check_token`,
`Signature.timestamp`, the `rsa` crate, the Marvin entry in `.cargo/audit.toml`),
the identity swap in the existing tests, and the no-socket build check. Phase 2's
new work is the OID-to-words map and the fail-closed test over `rsa-signed.pdf`.
Phase 3 gets the primitive it needs for the leaf-under-root check for free:
`sm2_key_of` on the root plus `Verifier::verify` over the leaf's `tbsCertificate`.

## The one thing to keep in view

`sm2` 0.13.3 is on the `signature 2` generation on purpose. The day `cms` moves to
`signature 3` / `digest 0.11` (`cms` 0.3, `x509-cert` 0.3), `sm2` has to move with
it in the same commit — 0.14 is that generation — and `sm3` to whichever release
sits on `digest 0.11`. Mixing the generations does not fail loudly; it fails as
trait bounds that look unrelated.
