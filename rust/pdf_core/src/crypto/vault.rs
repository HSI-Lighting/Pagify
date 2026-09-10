//! What Lock puts in the file, and takes back out.
//!
//! Redaction destroys. Lock hides: the content comes off the page exactly as a
//! redaction takes it, and a sealed copy goes into the document so that a
//! passcode can put it back. That is the feature neither Acrobat nor Foxit
//! really offers, and it is one honest sentence away from being a lie — so the
//! two are named apart everywhere they are offered, and this module is the only
//! thing that makes recovery possible.
//!
//! # Why a whole page rather than the words
//!
//! The obvious design stores what was removed: the text, where it sat, what size
//! and colour it was. It is smaller and it cannot be made to work. A run's font
//! is a document resource, and a subset font carries only the glyphs the
//! document already used — so restoring a run means reattaching it to a font
//! that may have been rewritten, or embedding a new one that draws the letters
//! differently. A restore that comes back in the wrong typeface is a restore
//! nobody trusts.
//!
//! So a sealed page is **the page**, as a one-page PDF, taken before anything is
//! removed from it. It restores by replacing, which is the mechanism undo
//! already uses and the only one that can promise the page comes back as it was.
//! The cost is the page's own bytes, images and all, carried in the file.
//!
//! It gives nothing away. Everything in that blob was on the page a moment
//! before, and the parts that were not hidden are still in plain view — the seal
//! protects what was hidden, and there was never anything else to protect.
//!
//! # One format, defined here
//!
//! Read and written **only in Rust**, by every platform, through the core. That
//! is the `restore`-blob lesson applied before the third platform exists rather
//! than after: a blob parsed once in Swift and once in Kotlin is a document
//! locked on a phone that will not unlock on a Mac.

use serde::{Deserialize, Serialize};

use super::cipher::{self, Secret};
use super::envelope::Envelope;
use super::kdf::KdfParams;
use crate::error::{PdfError, Result};

/// Marks a blob as ours.
///
/// Documents carry attachments for all sorts of reasons, and one that is not
/// ours must be left alone rather than parsed and reported as damaged.
pub const MAGIC: &str = "pagify.lock";

/// The name the vault is attached under.
pub const ATTACHMENT: &str = "pagify-lock.json";

/// Bumped only for a change a previous build could not read. Adding a field with
/// a `serde` default is not such a change.
pub const FORMAT_VERSION: u32 = 1;

/// One page, as it was, sealed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SealedPage {
    /// Where the page sat when it was locked.
    pub page_index: usize,
    /// A one-page PDF, sealed under the vault's data key.
    #[serde(with = "base64_bytes")]
    pub sealed: Vec<u8>,
}

/// One object hidden on a page, and where it was.
///
/// # Why this holds no ciphertext
///
/// The first design sealed each image's own bytes here. It could not be made to
/// work: blanking or removing a page object only reaches the file through
/// `FPDFPage_GenerateContent`, which re-emits the **whole** content stream —
/// measured on a real catalogue page, that alone rewrote text nobody had
/// touched, turning `sky‑light` into `sky -\r\nlight` and changing how the
/// paragraph around it drew. Restoring the object could not undo that, because
/// the damage was to everything else.
///
/// So the way back is the page seal that [`Vault::seal_page`] already holds —
/// the page exactly as it was before any of this — and an item is only a record
/// of *what is hidden and where*. Unlocking one restores that page and hides
/// the others again.
///
/// The image is still encrypted and still recoverable only with the passcode;
/// it travels inside the sealed page rather than in a blob of its own, which
/// also means no re-encoding, no lost transparency and no colour profile to get
/// wrong.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SealedItem {
    /// Names this lock for the rest of its life — what a badge on the page is
    /// keyed by, and what an unlock asks for. Assigned once and never reused.
    pub id: String,
    pub page_index: usize,
    /// Which object on that page is hidden, so restoring the page can hide the
    /// others again.
    pub object: usize,
    /// Where it sat, in page points with a top-left origin, so the page can
    /// show a badge over the gap. **In the clear, deliberately**: the gap is
    /// visible anyway, and a badge nobody can place is a lock nobody can undo.
    pub rect: [f32; 4],
}

/// Everything a passcode has to open.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vault {
    pub magic: String,
    pub v: u32,
    /// Where the data key lives, wrapped under the passcode.
    pub envelope: Envelope,
    pub pages: Vec<SealedPage>,
    /// Individually sealed objects.
    ///
    /// `#[serde(default)]` rather than a version bump: a vault written before
    /// this existed simply has none, and every build that could read that vault
    /// can still read this one. See [`FORMAT_VERSION`] — the version is for
    /// changes an older build could not survive, and an absent list is not one.
    #[serde(default)]
    pub items: Vec<SealedItem>,
}

impl Vault {
    /// Start a vault with a fresh data key.
    pub fn create(passcode: &[u8], params: KdfParams) -> Result<(Vault, Secret)> {
        let (envelope, dek) = Envelope::create(passcode, params)?;
        Ok((
            Vault {
                magic: MAGIC.to_string(),
                v: FORMAT_VERSION,
                envelope,
                pages: Vec::new(),
                items: Vec::new(),
            },
            dek,
        ))
    }

    /// Read a vault out of an attachment's bytes.
    ///
    /// **Refuses rather than guesses**, twice over: a blob that is not ours is
    /// somebody else's attachment, and a version this build does not know is a
    /// document written by a newer one. Both come back as errors that say which,
    /// because the alternative is a half-understood vault reporting that a page
    /// was never locked.
    pub fn parse(bytes: &[u8]) -> Result<Vault> {
        let vault: Vault = serde_json::from_slice(bytes)
            .map_err(|e| PdfError::InvalidArgument(format!("this is not a lock: {e}")))?;
        if vault.magic != MAGIC {
            return Err(PdfError::InvalidArgument(
                "that attachment belongs to something else".into(),
            ));
        }
        if vault.v > FORMAT_VERSION {
            return Err(PdfError::Unsupported(
                "this document was locked by a newer version of Pagify",
            ));
        }
        Ok(vault)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self)
            .map_err(|e| PdfError::Pdfium(format!("the lock could not be written: {e}")))
    }

    /// The data key, or an error if the passcode is wrong.
    pub fn unlock(&self, passcode: &[u8]) -> Result<Secret> {
        self.envelope.unwrap_dek(passcode)
    }

    /// Seal a page into the vault, if it is not already sealed.
    ///
    /// **The first seal wins, and that is the whole correctness of locking a
    /// page more than once.** A caller seals the page *as it currently stands*,
    /// which after an earlier lock is the page with that earlier area already
    /// removed. Replacing the seal therefore threw away the only copy holding
    /// the first area, and unlocking gave back a page where every area but the
    /// last was still missing — with nothing anywhere able to restore them.
    ///
    /// Reported from use: "if I lock multiple areas then I am able to only
    /// unlock the last one I locked."
    ///
    /// So a page that already has a way back keeps it. Locking a second area
    /// adds nothing to the vault, because the copy already there is the page
    /// before any of it — which is exactly what unlocking has to produce.
    ///
    /// Returns whether a new seal was written, for callers that want to say so.
    pub fn seal_page(&mut self, dek: &Secret, page_index: usize, page: &[u8]) -> Result<bool> {
        if self.pages.iter().any(|p| p.page_index == page_index) {
            return Ok(false);
        }
        let sealed = cipher::seal(dek, page, &binding(self.v, page_index))?;
        self.pages.push(SealedPage { page_index, sealed });
        self.pages.sort_by_key(|p| p.page_index);
        Ok(true)
    }

    /// Drop a page's way back.
    ///
    /// For a page that has been restored and is not locked any more — the seal
    /// has done its job, and keeping it would let a later lock of that page
    /// find a copy that predates whatever was done to it in between.
    pub fn forget_page(&mut self, page_index: usize) {
        self.pages.retain(|p| p.page_index != page_index);
    }

    // -- individually sealed objects ---------------------------------------

    /// Record that one object on a page is hidden, and name that lock.
    ///
    /// The id is random rather than derived from where the object sits: page
    /// and object indices both move when anything is inserted or reordered, and
    /// an id that moved with them would stop matching the badge a reader
    /// clicked.
    ///
    /// **The way back is the page seal**, which the caller must have written
    /// first — see [`SealedItem`] for why the bytes are not kept here.
    pub fn hide_item(
        &mut self,
        page_index: usize,
        object: usize,
        rect: [f32; 4],
    ) -> Result<String> {
        let id = hex(&cipher::random::<8>()?);
        self.items.push(SealedItem { id: id.clone(), page_index, object, rect });
        Ok(id)
    }

    pub fn item(&self, id: &str) -> Option<&SealedItem> {
        self.items.iter().find(|i| i.id == id)
    }

    /// Everything sealed on one page, for drawing its badges.
    pub fn items_on(&self, page_index: usize) -> Vec<&SealedItem> {
        self.items.iter().filter(|i| i.page_index == page_index).collect()
    }

    /// Forget one sealed object, once it is back on its page.
    pub fn forget_item(&mut self, id: &str) {
        self.items.retain(|i| i.id != id);
    }

    /// The original bytes of one locked page.
    ///
    /// **Verifies before it returns.** The tag is checked by `open`, so a blob
    /// somebody edited, truncated, or moved to another page index fails here
    /// rather than being put back onto a page as content.
    pub fn open_page(&self, dek: &Secret, page_index: usize) -> Result<Vec<u8>> {
        let sealed = self
            .pages
            .iter()
            .find(|p| p.page_index == page_index)
            .ok_or_else(|| {
                PdfError::InvalidArgument(format!("page {} is not locked", page_index + 1))
            })?;
        cipher::open(dek, &sealed.sealed, &binding(self.v, page_index))
    }

    pub fn locked_pages(&self) -> Vec<usize> {
        self.pages.iter().map(|p| p.page_index).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }
}

/// What the seal is bound to, beyond the key.
///
/// Built by hand rather than by serialising anything, for the same reason the
/// envelope's is: the bytes an AEAD authenticates must not change because a
/// field was renamed or reordered, or every document written before the change
/// stops opening.
///
/// **The page index is in here**, which is what stops a sealed page being lifted
/// out and dropped onto a different one. Without it, moving a blob from page 40
/// to page 1 would decrypt cleanly and restore the wrong page's content over
/// somebody's front cover.
fn binding(v: u32, page_index: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(MAGIC.len() + 13);
    out.extend_from_slice(MAGIC.as_bytes());
    out.push(0);
    out.extend_from_slice(&v.to_le_bytes());
    out.extend_from_slice(&(page_index as u64).to_le_bytes());
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Bytes as base64 in the JSON.
///
/// Not hex, which the envelope uses: an envelope is measured in bytes and worth
/// reading by eye, and a sealed page is measured in megabytes, where the third
/// the encoding saves is the difference between a file somebody can email and
/// one they cannot.
mod base64_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
            let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(ALPHABET[(n >> (18 - i * 6)) as usize & 0x3F] as char);
                } else {
                    out.push('=');
                }
            }
        }
        s.serialize_str(&out)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        let mut out = Vec::with_capacity(text.len() / 4 * 3);
        let mut acc = 0u32;
        let mut bits = 0u32;
        for c in text.bytes() {
            if c == b'=' || c == b'\n' || c == b'\r' {
                continue;
            }
            let Some(value) = ALPHABET.iter().position(|a| *a == c) else {
                return Err(serde::de::Error::custom("not base64"));
            };
            acc = acc << 6 | value as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cheap on purpose. The cost of the derivation is tested where the
    /// derivation is; here it would only make every test slow.
    fn cheap() -> KdfParams {
        KdfParams { memory_kib: 8, time: 1, lanes: 1 }
    }

    fn vault() -> (Vault, Secret) {
        Vault::create(b"open sesame", cheap()).expect("create")
    }

    #[test]
    fn a_sealed_page_comes_back_exactly() {
        let (mut vault, dek) = vault();
        let page = b"%PDF-1.7 ... a whole page ...".to_vec();
        vault.seal_page(&dek, 3, &page).expect("seal");

        assert_eq!(vault.open_page(&dek, 3).expect("open"), page);
        assert_eq!(vault.locked_pages(), vec![3]);
    }

    /// **The whole point of the passcode.**
    #[test]
    fn the_wrong_passcode_opens_nothing() {
        let (mut vault, dek) = vault();
        vault.seal_page(&dek, 0, b"secret").expect("seal");

        assert!(vault.unlock(b"open sesame").is_ok());
        assert!(vault.unlock(b"open sesam").is_err(), "a near miss opened it");
        assert!(vault.unlock(b"").is_err());
    }

    /// **What the page index in the binding is for.** A sealed page lifted from
    /// one index and dropped onto another must not decrypt — otherwise somebody
    /// with an editor restores page 40's content over the front cover.
    #[test]
    fn a_sealed_page_cannot_be_moved_to_another_index() {
        let (mut vault, dek) = vault();
        vault.seal_page(&dek, 40, b"the confidential page").expect("seal");

        // Exactly the edit an attacker would make: the same ciphertext, filed
        // under a different page.
        vault.pages[0].page_index = 1;
        assert!(
            vault.open_page(&dek, 1).is_err(),
            "a page was restored onto an index it was never sealed for"
        );
    }

    /// The tag is checked before anything is handed back, so an edited blob
    /// fails rather than becoming page content.
    #[test]
    fn a_tampered_seal_fails_rather_than_returning_something() {
        let (mut vault, dek) = vault();
        vault.seal_page(&dek, 0, b"the original page").expect("seal");

        let last = vault.pages[0].sealed.len() - 1;
        vault.pages[0].sealed[last] ^= 0x01;
        assert!(vault.open_page(&dek, 0).is_err());
    }

    #[test]
    fn a_truncated_seal_fails_too() {
        let (mut vault, dek) = vault();
        vault.seal_page(&dek, 0, b"the original page").expect("seal");
        vault.pages[0].sealed.truncate(4);
        assert!(vault.open_page(&dek, 0).is_err());
    }

    /// **Locking twice leaves one way back**, and it is to the page as it was
    /// before any of it — not to the page as it stood after the first lock.
    ///
    /// This test asserted the opposite of its own name until it was reported
    /// from use, and the bug it was pinning in place is the one below.
    #[test]
    fn locking_a_page_twice_keeps_the_first_original() {
        let (mut vault, dek) = vault();
        assert!(vault.seal_page(&dek, 2, b"the untouched original").expect("first"));
        assert!(
            !vault
                .seal_page(&dek, 2, b"already had one area hidden")
                .expect("second"),
            "the second seal reported itself as new"
        );

        assert_eq!(vault.pages.len(), 1, "two ways back is one too many");
        assert_eq!(vault.open_page(&dek, 2).expect("open"), b"the untouched original");
    }

    /// **The bug as it was reported.** "If I lock multiple areas then I am able
    /// to only unlock the last one I locked."
    ///
    /// Written the way `lock_area` really behaves: each lock seals the page *as
    /// it stands at that moment*, so the second one is handed a page that is
    /// already missing the first area. Replacing the seal discarded the only
    /// copy that still had it, and no unlock could ever bring it back.
    #[test]
    fn locking_three_areas_still_unlocks_all_three() {
        let (mut vault, dek) = vault();
        let original = b"NAME: A. Person   SALARY: 90000   PHONE: 555-0101";

        vault.seal_page(&dek, 0, original).expect("lock the name");
        // What the page looks like to the next lock, and the next.
        vault
            .seal_page(&dek, 0, b"NAME: [locked]   SALARY: 90000   PHONE: 555-0101")
            .expect("lock the salary");
        vault
            .seal_page(&dek, 0, b"NAME: [locked]   SALARY: [locked]   PHONE: 555-0101")
            .expect("lock the phone");

        assert_eq!(
            vault.open_page(&dek, 0).expect("unlock"),
            original,
            "unlocking gave back a page that was still missing the earlier areas"
        );
    }

    /// A page that has been restored gives its seal up, so a later lock of the
    /// same page cannot find a copy that predates whatever happened in between.
    #[test]
    fn a_page_that_has_been_given_back_can_be_locked_afresh() {
        let (mut vault, dek) = vault();
        vault.seal_page(&dek, 1, b"the original").expect("lock");
        vault.forget_page(1);
        assert!(vault.locked_pages().is_empty());

        assert!(vault.seal_page(&dek, 1, b"edited since, then locked again").expect("lock again"));
        assert_eq!(
            vault.open_page(&dek, 1).expect("unlock"),
            b"edited since, then locked again",
            "a stale seal outlived the page it described"
        );
    }

    #[test]
    fn a_page_that_was_never_locked_says_so() {
        let (vault, dek) = vault();
        let problem = vault.open_page(&dek, 7).expect_err("should not open");
        assert!(format!("{problem}").contains("page 8"), "it counted from zero: {problem}");
    }

    // -- hidden objects ----------------------------------------------------

    const RECT: [f32; 4] = [10.0, 20.0, 110.0, 90.0];

    /// An item records where something is hidden; the page seal beside it is
    /// what brings the page back.
    #[test]
    fn a_hidden_item_records_its_place_and_the_page_holds_the_way_back() {
        let (mut vault, dek) = vault();
        vault.seal_page(&dek, 3, b"the page with the photo on it").expect("seal the page");
        let id = vault.hide_item(3, 7, RECT).expect("hide");

        let item = vault.item(&id).expect("recorded");
        assert_eq!(item.page_index, 3);
        assert_eq!(item.object, 7, "the restore would hide the wrong object again");
        assert_eq!(item.rect, RECT, "the badge would be drawn in the wrong place");

        assert_eq!(
            vault.open_page(&dek, 3).expect("open"),
            b"the page with the photo on it",
            "the page seal is the way back and it did not survive"
        );
    }

    /// Several on one page, each named on its own — which is what clicking one
    /// badge has to address without disturbing the others.
    #[test]
    fn each_hidden_item_is_named_separately() {
        let (mut vault, _dek) = vault();
        let first = vault.hide_item(0, 1, RECT).expect("a");
        let second = vault.hide_item(0, 2, RECT).expect("b");
        vault.hide_item(5, 1, RECT).expect("c");

        assert_ne!(first, second, "two locks share an id");
        assert_eq!(vault.items_on(0).len(), 2);
        assert_eq!(vault.items_on(5).len(), 1);
        assert!(vault.items_on(9).is_empty());

        vault.forget_item(&first);
        assert!(vault.item(&first).is_none());
        assert!(vault.item(&second).is_some(), "forgetting one released the other");
    }

    // -- the format --------------------------------------------------------

    /// A vault written before items existed still loads — the field is
    /// defaulted rather than versioned, so an older document does not have to
    /// be refused.
    #[test]
    fn a_vault_from_before_items_existed_still_opens() {
        let (mut vault, dek) = vault();
        vault.seal_page(&dek, 0, b"a page").expect("seal");

        let mut json = serde_json::to_value(&vault).expect("to value");
        json.as_object_mut().expect("object").remove("items");
        let bytes = serde_json::to_vec(&json).expect("write");

        let back = Vault::parse(&bytes).expect("an older vault must still open");
        assert!(back.items.is_empty());
        assert_eq!(back.open_page(&dek, 0).expect("open"), b"a page");
    }

    #[test]
    fn a_vault_survives_being_written_and_read() {
        let (mut vault, dek) = vault();
        let page = vec![0xABu8; 5000];
        vault.seal_page(&dek, 1, &page).expect("seal");

        let bytes = vault.to_bytes().expect("write");
        let back = Vault::parse(&bytes).expect("read");
        assert_eq!(back, vault);

        // And the key still works through the round trip, which is the part
        // that matters: a format that survives but whose envelope does not is
        // a document nobody can open.
        let dek = back.unlock(b"open sesame").expect("unlock");
        assert_eq!(back.open_page(&dek, 1).expect("open"), page);
    }

    /// **Somebody else's attachment is left alone.** Documents carry attachments
    /// for all sorts of reasons and parsing one as a lock, failing, and calling
    /// the document damaged is worse than not recognising it.
    #[test]
    fn a_blob_that_is_not_ours_is_refused_by_name() {
        let foreign = br#"{"magic":"acme.notes","v":1,"envelope":null,"pages":[]}"#;
        assert!(Vault::parse(foreign).is_err());

        // The shape is right and only the magic is wrong, which is the case a
        // structural check would wave through.
        let (vault, _) = vault();
        let mut theirs = vault.clone();
        theirs.magic = "acme.notes".into();
        let problem = Vault::parse(&theirs.to_bytes().expect("write")).expect_err("refuse");
        assert!(format!("{problem}").contains("belongs to something else"));
    }

    /// A document written by a newer build says so rather than being half read.
    #[test]
    fn a_future_version_refuses_rather_than_guessing() {
        let (mut vault, _) = vault();
        vault.v = FORMAT_VERSION + 1;
        let problem = Vault::parse(&vault.to_bytes().expect("write")).expect_err("refuse");
        assert!(format!("{problem}").contains("newer version"));
    }

    // -- base64 ------------------------------------------------------------

    #[test]
    fn base64_round_trips_every_length_of_tail() {
        // The three cases the padding exists for, plus empty.
        for len in [0usize, 1, 2, 3, 4, 5, 6, 255, 1024] {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 7 % 256) as u8).collect();
            let (mut vault, dek) = vault();
            vault.seal_page(&dek, 0, &bytes).expect("seal");
            let back = Vault::parse(&vault.to_bytes().expect("write")).expect("read");
            assert_eq!(
                back.open_page(&dek, 0).expect("open"),
                bytes,
                "a {len}-byte page did not survive"
            );
        }
    }
}
