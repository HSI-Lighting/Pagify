//! What Obfuscate puts in the file, and takes back out.
//!
//! Redact destroys, Lock hides, and Obfuscate **replaces**: a ten-digit part
//! number becomes a different ten-digit part number, so a partner's pipeline
//! still parses the catalogue while the real figures stay in-house. This module
//! is the record of what was replaced, and the only thing that makes it
//! reversible.
//!
//! # It stores no original values, and that is not an oversight
//!
//! [`Vault`](super::vault::Vault) seals whole pages because a redaction is not
//! reversible arithmetic — the bytes are gone and something has to hold them.
//! FF1 is different: the same key, tweak and alphabet that encrypted a value
//! decrypt it again. So a ledger entry records **where a value sits and how it
//! was encrypted**, never what it said, and a whole obfuscated catalogue costs a
//! few hundred bytes rather than a copy of every page.
//!
//! The consequence to state plainly: **the passcode is the only way back.**
//! There is no copy of the original anywhere in the file. Lock can at least be
//! reasoned about as "the page is in there, sealed"; here there is nothing to
//! recover but the key.
//!
//! # The document id is random, not derived
//!
//! Every tweak starts with an id that separates this document from every other
//! one, and it is generated once and stored here. The tempting alternatives are
//! both wrong: a hash of the content changes the first time somebody edits a
//! page and takes every reversal with it, and a path or filename changes when
//! the file is moved or emailed, which is the normal life of a catalogue. It is
//! not a secret — a tweak is a domain separator, not a key — so storing it in
//! the clear costs nothing and buys reproducibility that survives editing.
//!
//! # Object indices move, so what was written is recorded too
//!
//! [`super::tweak`] is explicit that a tweak binds to a page and object index,
//! that re-ordering a page's objects changes those indices, and that the
//! constraint "belongs in the caller's design rather than being papered over
//! here". This is that caller. Each entry carries the string that was actually
//! written to the page, and [`Ledger::reverse`] refuses when the run it is
//! pointed at no longer holds it — the alternative is decrypting whatever
//! happens to sit at that index now and writing the resulting nonsense over
//! somebody's document, with no way back.
//!
//! Recording it leaks nothing. That string is printed on the page in plain
//! sight; a reader who can open the ledger can already read it.

use serde::{Deserialize, Serialize};

use super::cipher::{self, Secret};
use super::envelope::Envelope;
use super::ff1::{Alphabet, Ff1Sm4};
use super::kdf::KdfParams;
use super::tweak::{self, FieldKind};
use crate::error::{PdfError, Result};

/// Marks a blob as ours. See [`super::vault::MAGIC`] — a document carries
/// attachments for all sorts of reasons and one that is not ours is left alone.
pub const MAGIC: &str = "pagify.obfuscation";

/// The name the ledger is attached under.
pub const ATTACHMENT: &str = "pagify-obfuscation.json";

/// Bumped only for a change a previous build could not read. Adding a field
/// with a `serde` default is not such a change.
pub const FORMAT_VERSION: u32 = 1;

/// One value that was replaced, and everything needed to put it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObfuscatedField {
    pub page_index: usize,
    /// Position in the page's object list, as [`crate::document::TextRun`]
    /// reports it.
    pub object: usize,
    pub kind: FieldKind,
    /// The symbols this field is written in, stored rather than inferred: see
    /// [`Alphabet`], which is explicit that inferring it per value decrypts the
    /// all-digits entries of an alphanumeric field to the wrong thing.
    pub alphabet: String,
    /// What was written to the page. Checked before any reversal — see this
    /// module's own notes on why, and why recording it gives nothing away.
    pub written: String,
}

/// Every replacement made in one document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ledger {
    pub magic: String,
    pub v: u32,
    /// Where the data key lives, wrapped under the passcode.
    pub envelope: Envelope,
    /// Separates this document's tweaks from every other document's. Public by
    /// design — see the note at the top of this module.
    #[serde(with = "super::envelope::hex_bytes")]
    pub document_id: Vec<u8>,
    pub fields: Vec<ObfuscatedField>,
    /// A digest over `document_id` and `fields`, sealed under the data key.
    ///
    /// Every field here is plaintext bookkeeping — `written` is what
    /// `reverse` actually decrypts, and `page_index`/`object`/`kind`/
    /// `alphabet` all feed the tweak that has to match what encrypted it —
    /// with nothing but this binding to stop a file-write attacker (no
    /// passcode needed) editing one, or deleting it outright, and a right
    /// passcode afterwards reading as a normal unlock over a ledger that no
    /// longer describes what was actually done. Checked by [`Ledger::unlock`].
    /// `None` for a ledger written before this existed — still opens, and
    /// gains one the next time anything about it is saved. Found by audit.
    #[serde(default)]
    pub bind: Option<String>,
}

impl Ledger {
    /// Start a ledger with a fresh data key and a fresh document id.
    pub fn create(passcode: &[u8], params: KdfParams) -> Result<(Ledger, Secret)> {
        let (envelope, dek) = Envelope::create(passcode, params)?;
        Ok((
            Ledger {
                magic: MAGIC.to_string(),
                v: FORMAT_VERSION,
                envelope,
                document_id: cipher::random::<16>()?.to_vec(),
                fields: Vec::new(),
                bind: None,
            },
            dek,
        ))
    }

    /// Read a ledger out of an attachment's bytes.
    ///
    /// **Refuses rather than guesses**, on the same two grounds
    /// [`super::vault::Vault::parse`] does: a blob that is not ours belongs to
    /// somebody else, and a version this build does not know was written by a
    /// newer one. Half-understanding either would report that a field was never
    /// obfuscated, which here means telling somebody their original is gone.
    pub fn parse(bytes: &[u8]) -> Result<Ledger> {
        let ledger: Ledger = serde_json::from_slice(bytes).map_err(|e| {
            PdfError::InvalidArgument(format!("this is not an obfuscation record: {e}"))
        })?;
        if ledger.magic != MAGIC {
            return Err(PdfError::InvalidArgument(
                "that attachment belongs to something else".into(),
            ));
        }
        if ledger.v > FORMAT_VERSION {
            return Err(PdfError::Unsupported(
                "this document was obfuscated by a newer version of Pagify",
            ));
        }
        Ok(ledger)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(|e| {
            PdfError::Pdfium(format!("the obfuscation record could not be written: {e}"))
        })
    }

    /// The data key, or an error if the passcode is wrong — or if
    /// `document_id` or `fields` have been tampered with since the ledger was
    /// last saved.
    ///
    /// Checked here, once, before anything else trusts either: `reverse` only
    /// checks that a field's own `written` still matches the page, which says
    /// nothing about whether `written` itself, or `alphabet`, or `kind`, has
    /// been edited to something that still opens under the real key but is
    /// not what was actually encrypted.
    pub fn unlock(&self, passcode: &[u8]) -> Result<Secret> {
        let dek = self.envelope.unwrap_dek(passcode)?;
        if let Some(bind) = &self.bind {
            let sealed = unhex(bind).ok_or_else(|| {
                PdfError::InvalidArgument("this record's binding is not readable".into())
            })?;
            let digest = cipher::open(&dek, &sealed, BIND).map_err(|_| {
                PdfError::InvalidArgument(
                    "this record's binding does not open — it has been altered".into(),
                )
            })?;
            if digest.as_slice() != structure_digest(&self.document_id, &self.fields) {
                return Err(PdfError::InvalidArgument(
                    "this record has been altered since it was made".into(),
                ));
            }
        }
        Ok(dek)
    }

    /// Recompute and seal the binding over the current `document_id` and
    /// `fields`.
    ///
    /// The caller's job to run before the ledger is written anywhere — see
    /// [`super::vault::Vault::seal_structure`], which this mirrors exactly.
    pub fn seal_structure(&mut self, dek: &Secret) -> Result<()> {
        let digest = structure_digest(&self.document_id, &self.fields);
        self.bind = Some(hex(&cipher::seal(dek, &digest, BIND)?));
        Ok(())
    }

    /// Encrypt one value and record where it went.
    ///
    /// Returns the replacement to write to the page. The caller writes it; this
    /// only decides what it should be and remembers enough to undo it.
    ///
    /// **The encryption and the record are made together, here**, for the same
    /// reason [`super::vault::Vault::seal_page`] seals rather than taking a
    /// sealed blob: a record built separately from the encryption it describes
    /// can disagree with it, and a tweak that disagrees decrypts to nothing
    /// anybody wants.
    ///
    /// Short fields are refused by [`super::ff1`] itself — SP 800-38G's
    /// `radix^len ≥ 1,000,000` floor — rather than silently weakened here.
    pub fn obfuscate(
        &mut self,
        dek: &Secret,
        page_index: usize,
        object: usize,
        kind: FieldKind,
        alphabet: &str,
        plaintext: &str,
    ) -> Result<String> {
        // Obfuscating twice would encrypt the replacement, leaving a value that
        // needs two reversals when the ledger only ever describes one. Nothing
        // legitimate wants it — the value is already hidden — so it is refused
        // rather than chained.
        if self.field_at(page_index, object).is_some() {
            return Err(PdfError::InvalidArgument(format!(
                "page {}, object {object} is already obfuscated",
                page_index + 1
            )));
        }

        let ff1 = Ff1Sm4::new(&dek.0, Alphabet::new(alphabet)?)?;
        let written = ff1.encrypt(
            plaintext,
            &tweak::for_field(&self.document_id, page_index, object, &kind),
        )?;

        self.fields.push(ObfuscatedField {
            page_index,
            object,
            kind,
            alphabet: alphabet.to_string(),
            written: written.clone(),
        });
        self.fields.sort_by_key(|f| (f.page_index, f.object));
        Ok(written)
    }

    /// Recover one original value.
    ///
    /// `on_the_page` is what the run at that object says **now**. It is checked
    /// against what was written before anything is decrypted: an object index
    /// that has shifted since obfuscation points at a different run, and
    /// decrypting that one would write nonsense over real content with no way
    /// back.
    pub fn reverse(
        &self,
        dek: &Secret,
        field: &ObfuscatedField,
        on_the_page: &str,
    ) -> Result<String> {
        if on_the_page != field.written {
            return Err(PdfError::InvalidArgument(format!(
                "page {} has changed since it was obfuscated — the text at object {} is no \
                 longer the value that was written there",
                field.page_index + 1,
                field.object
            )));
        }
        let ff1 = Ff1Sm4::new(&dek.0, Alphabet::new(&field.alphabet)?)?;
        ff1.decrypt(
            &field.written,
            &tweak::for_field(&self.document_id, field.page_index, field.object, &field.kind),
        )
    }

    pub fn field_at(&self, page_index: usize, object: usize) -> Option<&ObfuscatedField> {
        self.fields
            .iter()
            .find(|f| f.page_index == page_index && f.object == object)
    }

    /// Stop tracking one field — for after it has been reversed and the page
    /// carries its original again.
    pub fn forget(&mut self, page_index: usize, object: usize) {
        self.fields
            .retain(|f| !(f.page_index == page_index && f.object == object));
    }

    /// Sorts rather than assuming `fields` is in order: `obfuscate` keeps it
    /// sorted, but `parse` takes whatever the file says, and `dedup` alone
    /// would report a page twice for a hand-edited record.
    pub fn obfuscated_pages(&self) -> Vec<usize> {
        let mut pages: Vec<usize> = self.fields.iter().map(|f| f.page_index).collect();
        pages.sort_unstable();
        pages.dedup();
        pages
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

/// What [`Ledger::seal_structure`] binds against, so it cannot be mistaken
/// for anything else sealed under the same key.
const BIND: &[u8] = b"pagify obfuscation bind";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The inverse of [`hex`]. `None` for anything that is not an even run of hex
/// digits, rather than silently taking what it can.
fn unhex(text: &str) -> Option<Vec<u8>> {
    let digits = text.as_bytes();
    if digits.len() % 2 != 0 || !digits.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    Some(
        digits
            .chunks(2)
            .map(|pair| {
                let nibble = |b: u8| (b as char).to_digit(16).unwrap_or(0) as u8;
                nibble(pair[0]) << 4 | nibble(pair[1])
            })
            .collect(),
    )
}

/// A digest over `document_id` and every field — the plaintext bookkeeping
/// nothing else authenticates. `written` is what `reverse` actually decrypts,
/// and `page_index`/`object`/`kind`/`alphabet` all feed the tweak that has to
/// match what encrypted it, so every one of them has to be in here for any of
/// them to be protected.
fn structure_digest(document_id: &[u8], fields: &[ObfuscatedField]) -> [u8; 32] {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update((document_id.len() as u64).to_le_bytes());
    hasher.update(document_id);
    hasher.update((fields.len() as u64).to_le_bytes());
    for field in fields {
        hasher.update((field.page_index as u64).to_le_bytes());
        hasher.update((field.object as u64).to_le_bytes());
        hasher.update((field.kind.id.len() as u64).to_le_bytes());
        hasher.update(field.kind.id.as_bytes());
        hasher.update([match field.kind.source {
            tweak::Source::TextRun => 0u8,
            tweak::Source::FormField => 1,
            tweak::Source::Annotation => 2,
        }]);
        hasher.update([u8::from(field.kind.deterministic)]);
        hasher.update((field.alphabet.len() as u64).to_le_bytes());
        hasher.update(field.alphabet.as_bytes());
        hasher.update((field.written.len() as u64).to_le_bytes());
        hasher.update(field.written.as_bytes());
    }
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::tweak::Source;

    /// Cheap on purpose — the cost of the derivation is tested where the
    /// derivation is, and here it would only make every test slow.
    fn cheap() -> KdfParams {
        KdfParams { memory_kib: 8, time: 1, lanes: 1 }
    }

    fn ledger() -> (Ledger, Secret) {
        Ledger::create(b"open sesame", cheap()).expect("create")
    }

    fn part_number() -> FieldKind {
        FieldKind::opaque("partNumber", Source::TextRun)
    }

    // -- the round trip ----------------------------------------------------

    /// **The whole feature in one test.** A part number becomes a different
    /// part number of the same shape, and the passcode brings the real one
    /// back.
    #[test]
    fn an_obfuscated_value_keeps_its_shape_and_comes_back_exactly() {
        let (mut ledger, dek) = ledger();
        let original = "4821000734";

        let written = ledger
            .obfuscate(&dek, 0, 7, part_number(), Alphabet::DIGITS, original)
            .expect("obfuscate");

        assert_eq!(written.len(), original.len(), "the shape changed");
        assert!(written.chars().all(|c| c.is_ascii_digit()), "not digits any more: {written}");
        assert_ne!(written, original, "it did not actually change");

        let field = ledger.field_at(0, 7).expect("recorded");
        assert_eq!(ledger.reverse(&dek, field, &written).expect("reverse"), original);
    }

    /// The point of the whole exercise: no copy of the original is anywhere in
    /// the record. If this fails, obfuscation is hiding a value in the page and
    /// publishing it in the attachment beside it.
    #[test]
    fn the_record_does_not_contain_the_original_anywhere() {
        let (mut ledger, dek) = ledger();
        let original = "4821000734";
        ledger
            .obfuscate(&dek, 0, 7, part_number(), Alphabet::DIGITS, original)
            .expect("obfuscate");

        let bytes = ledger.to_bytes().expect("write");
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains(original),
            "the original value is sitting in the record: {text}"
        );
    }

    #[test]
    fn the_wrong_passcode_recovers_nothing() {
        let (mut ledger, dek) = ledger();
        ledger
            .obfuscate(&dek, 0, 7, part_number(), Alphabet::DIGITS, "4821000734")
            .expect("obfuscate");

        assert!(ledger.unlock(b"open sesame").is_ok());
        assert!(ledger.unlock(b"open sesam").is_err(), "a near miss opened it");
        assert!(ledger.unlock(b"").is_err());
    }

    /// A wrong key does not fail — FF1 has no tag, by design, and this is the
    /// honest shape of that. It returns a *different plausible value*, which is
    /// exactly why [`super::super::ff1`] says anything needing integrity gets
    /// Lock or Redact instead.
    #[test]
    fn a_wrong_key_gives_a_plausible_wrong_answer_rather_than_an_error() {
        let (mut ledger, dek) = ledger();
        let written = ledger
            .obfuscate(&dek, 0, 7, part_number(), Alphabet::DIGITS, "4821000734")
            .expect("obfuscate");
        let field = ledger.field_at(0, 7).expect("recorded").clone();

        let (_, other_dek) = Ledger::create(b"a different passcode", cheap()).expect("create");
        let wrong = ledger.reverse(&other_dek, &field, &written).expect("no tag to fail on");
        assert_ne!(wrong, "4821000734");
        assert_eq!(wrong.len(), 10, "still the right shape, which is the danger");
    }

    // -- what the tweak binds to -------------------------------------------

    /// The same value at two places encrypts differently, which is what stops a
    /// reader learning which catalogue rows share a part number without
    /// decrypting anything.
    #[test]
    fn the_same_value_in_two_places_is_written_differently() {
        let (mut ledger, dek) = ledger();
        let a = ledger
            .obfuscate(&dek, 0, 7, part_number(), Alphabet::DIGITS, "4821000734")
            .expect("first");
        let b = ledger
            .obfuscate(&dek, 0, 8, part_number(), Alphabet::DIGITS, "4821000734")
            .expect("second");
        assert_ne!(a, b, "an opaque kind repeated itself");

        // And both still reverse to the same original.
        for object in [7, 8] {
            let field = ledger.field_at(0, object).expect("recorded");
            let text = field.written.clone();
            assert_eq!(ledger.reverse(&dek, field, &text).expect("reverse"), "4821000734");
        }
    }

    /// And a joinable kind deliberately does repeat, so a partner can still
    /// match rows — the documented weakening, exercised end to end rather than
    /// only at the tweak.
    #[test]
    fn a_joinable_kind_writes_the_same_replacement_wherever_it_appears() {
        let (mut ledger, dek) = ledger();
        let kind = FieldKind::joinable("partNumber", Source::TextRun);
        let a = ledger
            .obfuscate(&dek, 0, 7, kind.clone(), Alphabet::DIGITS, "4821000734")
            .expect("first");
        let b = ledger
            .obfuscate(&dek, 40, 912, kind, Alphabet::DIGITS, "4821000734")
            .expect("second");
        assert_eq!(a, b, "a joinable field did not join");
    }

    /// Two documents obfuscated under the same passcode must not produce the
    /// same replacements — otherwise the pair gives away that both hold the
    /// same value.
    #[test]
    fn two_documents_write_different_replacements_for_the_same_value() {
        let (mut one, dek_one) = ledger();
        let (mut two, dek_two) = ledger();
        assert_ne!(one.document_id, two.document_id, "two documents share an id");

        let a = one
            .obfuscate(&dek_one, 0, 7, part_number(), Alphabet::DIGITS, "4821000734")
            .expect("one");
        let b = two
            .obfuscate(&dek_two, 0, 7, part_number(), Alphabet::DIGITS, "4821000734")
            .expect("two");
        assert_ne!(a, b);
    }

    // -- refusals ----------------------------------------------------------

    /// **The page-changed guard.** An object index that has shifted points at a
    /// different run, and decrypting that one writes nonsense over real content
    /// with nothing to undo it.
    #[test]
    fn a_run_that_no_longer_holds_what_was_written_refuses_to_reverse() {
        let (mut ledger, dek) = ledger();
        ledger
            .obfuscate(&dek, 0, 7, part_number(), Alphabet::DIGITS, "4821000734")
            .expect("obfuscate");
        let field = ledger.field_at(0, 7).expect("recorded");

        let problem = ledger
            .reverse(&dek, field, "9999999999")
            .expect_err("should refuse");
        assert!(
            format!("{problem}").contains("has changed"),
            "unhelpful message: {problem}"
        );
    }

    /// Obfuscating an already-obfuscated field would need two reversals where
    /// the ledger records one.
    #[test]
    fn a_field_cannot_be_obfuscated_twice() {
        let (mut ledger, dek) = ledger();
        ledger
            .obfuscate(&dek, 0, 7, part_number(), Alphabet::DIGITS, "4821000734")
            .expect("first");

        let problem = ledger
            .obfuscate(&dek, 0, 7, part_number(), Alphabet::DIGITS, "5000000001")
            .expect_err("a second should be refused");
        assert!(format!("{problem}").contains("already obfuscated"), "{problem}");
        assert_eq!(ledger.fields.len(), 1, "a second record was kept anyway");
    }

    /// SP 800-38G's own floor, reaching the caller as a refusal. A four-digit
    /// field has ten thousand values and encrypting it only rearranges which
    /// one you see.
    #[test]
    fn a_field_too_short_to_encrypt_meaningfully_is_refused() {
        let (mut ledger, dek) = ledger();
        assert!(ledger
            .obfuscate(&dek, 0, 7, part_number(), Alphabet::DIGITS, "4821")
            .is_err());
        assert!(ledger.is_empty(), "a refused field was recorded anyway");
    }

    /// A value carrying something the declared alphabet does not have is a
    /// mis-declared field, and guessing at it would decrypt to something else
    /// entirely.
    #[test]
    fn a_value_outside_its_declared_alphabet_is_refused() {
        let (mut ledger, dek) = ledger();
        assert!(ledger
            .obfuscate(&dek, 0, 7, part_number(), Alphabet::DIGITS, "4821-000734")
            .is_err());
        assert!(ledger.is_empty());
    }

    // -- bookkeeping -------------------------------------------------------

    #[test]
    fn forgetting_a_reversed_field_takes_it_off_the_record() {
        let (mut ledger, dek) = ledger();
        ledger
            .obfuscate(&dek, 0, 7, part_number(), Alphabet::DIGITS, "4821000734")
            .expect("obfuscate");
        assert!(!ledger.is_empty());

        ledger.forget(0, 7);
        assert!(ledger.is_empty());
        assert!(ledger.field_at(0, 7).is_none());
    }

    #[test]
    fn the_pages_carrying_obfuscated_fields_are_reported_once_each() {
        let (mut ledger, dek) = ledger();
        for object in [7, 8, 9] {
            ledger
                .obfuscate(&dek, 3, object, part_number(), Alphabet::DIGITS, "4821000734")
                .expect("obfuscate");
        }
        ledger
            .obfuscate(&dek, 5, 1, part_number(), Alphabet::DIGITS, "4821000734")
            .expect("obfuscate");

        assert_eq!(ledger.obfuscated_pages(), vec![3, 5]);

        // And a record whose fields arrived out of order — which `parse` will
        // hand back exactly as the file had them — still reports each page
        // once.
        let mut shuffled = ledger.clone();
        shuffled.fields.reverse();
        assert_eq!(shuffled.obfuscated_pages(), vec![3, 5]);
    }

    // -- the format --------------------------------------------------------

    /// The record has to survive the file, and the key has to still work
    /// afterwards — a format that round-trips but whose envelope does not is a
    /// document nobody can recover.
    #[test]
    fn a_ledger_survives_being_written_and_read() {
        let (mut ledger, dek) = ledger();
        let written = ledger
            .obfuscate(&dek, 2, 11, part_number(), Alphabet::ALPHANUMERIC, "AB12CD34")
            .expect("obfuscate");

        let bytes = ledger.to_bytes().expect("write");
        let back = Ledger::parse(&bytes).expect("read");
        assert_eq!(back, ledger);

        let dek = back.unlock(b"open sesame").expect("unlock");
        let field = back.field_at(2, 11).expect("recorded");
        assert_eq!(back.reverse(&dek, field, &written).expect("reverse"), "AB12CD34");
    }

    /// Somebody else's attachment is left alone — a document carries them for
    /// all sorts of reasons.
    #[test]
    fn a_blob_that_is_not_ours_is_refused_by_name() {
        let (ledger, _) = ledger();
        let mut theirs = ledger.clone();
        theirs.magic = "acme.notes".into();
        let problem = Ledger::parse(&theirs.to_bytes().expect("write")).expect_err("refuse");
        assert!(format!("{problem}").contains("belongs to something else"), "{problem}");
    }

    /// And it must not be confused with a Lock vault, which is the attachment
    /// most likely to be sitting beside it in the same document.
    #[test]
    fn a_lock_vault_is_not_read_as_an_obfuscation_record() {
        let (vault, _) = super::super::vault::Vault::create(b"open sesame", cheap()).expect("vault");
        assert!(Ledger::parse(&vault.to_bytes().expect("write")).is_err());
    }

    #[test]
    fn a_future_version_refuses_rather_than_guessing() {
        let (mut ledger, _) = ledger();
        ledger.v = FORMAT_VERSION + 1;
        let problem = Ledger::parse(&ledger.to_bytes().expect("write")).expect_err("refuse");
        assert!(format!("{problem}").contains("newer version"), "{problem}");
    }

    // -- the binding — L-7, found by audit ----------------------------------

    /// **A right passcode over an edited field must not read as a normal
    /// unlock.** Editing `written` (or `alphabet`, or `kind`) needs no
    /// passcode at all — it is a plain JSON edit — and `reverse` only checks
    /// that it still matches the page, which says nothing about whether it is
    /// still the value that was actually encrypted.
    #[test]
    fn editing_a_fields_written_value_is_noticed_on_unlock() {
        let (mut ledger, dek) = ledger();
        ledger.obfuscate(&dek, 0, 7, part_number(), Alphabet::DIGITS, "4821000734").expect("obfuscate");
        ledger.seal_structure(&dek).expect("bind");
        assert!(ledger.unlock(b"open sesame").is_ok(), "a freshly bound ledger did not unlock");

        ledger.fields[0].written = "0000000000".into();
        assert!(
            ledger.unlock(b"open sesame").is_err(),
            "an edited `written` value went unnoticed"
        );
    }

    /// Deleting a field entirely is just as much an edit as changing one.
    #[test]
    fn deleting_a_field_is_noticed_on_unlock() {
        let (mut ledger, dek) = ledger();
        ledger.obfuscate(&dek, 0, 7, part_number(), Alphabet::DIGITS, "4821000734").expect("obfuscate");
        ledger.seal_structure(&dek).expect("bind");

        ledger.fields.clear();
        assert!(
            ledger.unlock(b"open sesame").is_err(),
            "a right passcode unlocked a ledger with its fields wiped"
        );
    }

    /// A ledger written before the binding existed still opens.
    #[test]
    fn a_ledger_with_no_binding_still_unlocks() {
        let (ledger, _) = ledger();
        assert!(ledger.bind.is_none(), "the fixture already has a binding");
        assert!(ledger.unlock(b"open sesame").is_ok(), "a legacy ledger was refused");
    }

    /// Sealing the structure again after a legitimate change still leaves the
    /// ledger openable, and every recorded value still reverses.
    #[test]
    fn resealing_after_a_real_change_still_unlocks() {
        let (mut ledger, dek) = ledger();
        ledger.obfuscate(&dek, 0, 7, part_number(), Alphabet::DIGITS, "4821000734").expect("first");
        ledger.seal_structure(&dek).expect("bind");

        let written = ledger
            .obfuscate(&dek, 0, 9, part_number(), Alphabet::DIGITS, "1234567890")
            .expect("second");
        ledger.seal_structure(&dek).expect("bind again");

        assert!(ledger.unlock(b"open sesame").is_ok(), "a legitimately grown ledger was refused");
        let field = ledger.field_at(0, 9).expect("recorded");
        assert_eq!(ledger.reverse(&dek, field, &written).expect("reverse"), "1234567890");
    }

    /// The binding survives being written to JSON and read back.
    #[test]
    fn the_binding_survives_a_round_trip_and_still_catches_tampering() {
        let (mut ledger, dek) = ledger();
        ledger.obfuscate(&dek, 0, 7, part_number(), Alphabet::DIGITS, "4821000734").expect("obfuscate");
        ledger.seal_structure(&dek).expect("bind");

        let mut back = Ledger::parse(&ledger.to_bytes().expect("write")).expect("read");
        assert!(back.unlock(b"open sesame").is_ok(), "a bound ledger did not survive a round trip");

        back.fields[0].alphabet = "letters".into();
        assert!(
            back.unlock(b"open sesame").is_err(),
            "an edited alphabet after a round trip went unnoticed"
        );
    }
}
