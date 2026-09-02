//! The C ABI, for callers that are not the JVM.
//!
//! Android reaches this crate through JNI (`jni_bridge`), which is a different
//! and much chattier boundary. iOS, and anything else that can call C, comes
//! through here instead.
//!
//! # The memory contract, which is the whole of the danger
//!
//! Every function returning a string returns a buffer **this crate allocated**,
//! and it must be given back to [`pagify_string_free`]. Calling `free()` on it,
//! or letting Swift's ARC take it, frees a pointer from a different allocator —
//! which does not fail, it corrupts the heap and crashes somewhere else entirely.
//!
//! In Swift the safe shape is always:
//!
//! ```swift
//! guard let raw = pagify_vcard(cardJson, exportedAt) else { return nil }
//! defer { pagify_string_free(raw) }
//! let vcard = String(cString: raw)
//! ```
//!
//! `defer` immediately after the guard, before anything that can throw or return.
//!
//! Every input is a NUL-terminated UTF-8 C string owned by the **caller**, read
//! and never retained past the call.
//!
//! # Errors
//!
//! A null return means "no result", and what that means is documented per
//! function. Nothing here panics across the boundary: a panic unwinding into
//! Swift is undefined behaviour, so every body is wrapped in `catch_unwind`.

use crate::contacts::BusinessCard;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Read a C string as `&str`, or `None` if it is null or not UTF-8.
///
/// Safety: `raw` must be null or a NUL-terminated string valid for this call.
unsafe fn borrow<'a>(raw: *const c_char) -> Option<&'a str> {
    if raw.is_null() {
        return None;
    }
    CStr::from_ptr(raw).to_str().ok()
}

/// Hand a string to the caller, who must return it to [`pagify_string_free`].
///
/// Null when the string holds an interior NUL, which cannot be a C string. That
/// is not reachable from anything this crate produces, but returning null beats
/// truncating somebody's data at the first zero byte.
fn hand_over(value: String) -> *mut c_char {
    match CString::new(value) {
        Ok(owned) => owned.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Give back a string this library returned.
///
/// Null is accepted and does nothing, so a caller need not check before freeing.
///
/// Safety: `raw` must be null, or a pointer this library returned and which has
/// not already been freed. Freeing twice is undefined behaviour.
#[no_mangle]
pub unsafe extern "C" fn pagify_string_free(raw: *mut c_char) {
    if raw.is_null() {
        return;
    }
    drop(CString::from_raw(raw));
}

/// Write a contact as a vCard 3.0, stamped `exported_at` as its `REV`.
///
/// `card_json` is a `BusinessCard` as JSON; `exported_at` is an RFC 3339
/// timestamp in UTC. The clock belongs to the platform — it knows the time zone
/// and this crate does not.
///
/// Returns null when the JSON cannot be read, or either argument is null.
///
/// Safety: both arguments must be null or valid NUL-terminated UTF-8. The result
/// must be freed with [`pagify_string_free`].
#[no_mangle]
pub unsafe extern "C" fn pagify_vcard(
    card_json: *const c_char,
    exported_at: *const c_char,
) -> *mut c_char {
    catch_unwind(AssertUnwindSafe(|| {
        let (Some(json), Some(stamp)) = (borrow(card_json), borrow(exported_at)) else {
            return std::ptr::null_mut();
        };
        let Ok(card) = serde_json::from_str::<BusinessCard>(json) else {
            return std::ptr::null_mut();
        };
        hand_over(crate::contacts::to_vcard(&card, stamp))
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// The same for several contacts, as one file.
///
/// `cards_json` is a JSON array of `BusinessCard`. Every card is stamped with the
/// same `exported_at`, which is what a group export needs: everyone sent together
/// left together.
///
/// Safety: as [`pagify_vcard`].
#[no_mangle]
pub unsafe extern "C" fn pagify_vcards(
    cards_json: *const c_char,
    exported_at: *const c_char,
) -> *mut c_char {
    catch_unwind(AssertUnwindSafe(|| {
        let (Some(json), Some(stamp)) = (borrow(cards_json), borrow(exported_at)) else {
            return std::ptr::null_mut();
        };
        let Ok(cards) = serde_json::from_str::<Vec<BusinessCard>>(json) else {
            return std::ptr::null_mut();
        };
        hand_over(crate::contacts::to_vcards(&cards, stamp))
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Read a vCard into a `BusinessCard`, as JSON.
///
/// This is the QR path: a card carrying a vCard QR needs no detection, no
/// rectification and no recognition, so every field comes back at full
/// confidence.
///
/// **Null is an ordinary answer, not a failure.** Most QR codes on business cards
/// hold a web address rather than a contact, and the caller has to tell the two
/// apart so it can fall through to reading the card by eye instead of saving a
/// blank contact.
///
/// Safety: `text` must be null or valid NUL-terminated UTF-8. The result must be
/// freed with [`pagify_string_free`].
#[no_mangle]
pub unsafe extern "C" fn pagify_vcard_parse(text: *const c_char) -> *mut c_char {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(text) = borrow(text) else {
            return std::ptr::null_mut();
        };
        let Some(card) = crate::contacts::from_vcard(text) else {
            return std::ptr::null_mut();
        };
        match serde_json::to_string(&card) {
            Ok(json) => hand_over(json),
            Err(_) => std::ptr::null_mut(),
        }
    }))
    .unwrap_or(std::ptr::null_mut())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Call across the boundary the way a caller would, and free the result.
    fn round_trip(call: impl FnOnce() -> *mut c_char) -> Option<String> {
        let raw = call();
        if raw.is_null() {
            return None;
        }
        // Safety: the pointer came from this library and is freed once, below.
        let value = unsafe { CStr::from_ptr(raw) }.to_str().unwrap().to_string();
        unsafe { pagify_string_free(raw) };
        Some(value)
    }

    fn a_card_json() -> CString {
        CString::new(
            r#"{"name":{"value":"Jane Okafor","confidence":1.0},
                "company":{"value":"Meridian Systems","confidence":1.0},
                "emails":[{"value":"jane@meridian.example","confidence":1.0}],
                "rawText":"Jane Okafor"}"#,
        )
        .unwrap()
    }

    #[test]
    fn a_card_crosses_the_boundary_and_comes_back_as_a_vcard() {
        let card = a_card_json();
        let stamp = CString::new("2026-08-27T10:22:31Z").unwrap();
        let vcard = round_trip(|| unsafe { pagify_vcard(card.as_ptr(), stamp.as_ptr()) })
            .expect("a valid card should produce a vCard");

        assert!(vcard.contains("FN:Jane Okafor\r\n"));
        assert!(vcard.contains("ORG:Meridian Systems\r\n"));
        assert!(vcard.contains("REV:2026-08-27T10:22:31Z\r\n"));
    }

    #[test]
    fn a_vcard_crosses_back_as_a_card() {
        let text = CString::new(
            "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Jane Okafor\r\n\
             EMAIL:jane@meridian.example\r\nEND:VCARD\r\n",
        )
        .unwrap();
        let json = round_trip(|| unsafe { pagify_vcard_parse(text.as_ptr()) })
            .expect("a vCard should parse");
        assert!(json.contains("Jane Okafor"));
        assert!(json.contains("jane@meridian.example"));
    }

    /// The answer the QR path depends on being able to tell apart.
    #[test]
    fn a_payload_that_is_not_a_vcard_returns_null() {
        let url = CString::new("https://www.hsilighting.com").unwrap();
        assert!(round_trip(|| unsafe { pagify_vcard_parse(url.as_ptr()) }).is_none());
    }

    /// A null in is a null out, never a crash. Swift will pass one eventually.
    #[test]
    fn null_arguments_are_survivable() {
        let stamp = CString::new("2026-08-27T10:22:31Z").unwrap();
        assert!(round_trip(|| unsafe { pagify_vcard(std::ptr::null(), stamp.as_ptr()) }).is_none());
        assert!(round_trip(|| unsafe { pagify_vcard(a_card_json().as_ptr(), std::ptr::null()) })
            .is_none());
        assert!(round_trip(|| unsafe { pagify_vcard_parse(std::ptr::null()) }).is_none());
    }

    /// Malformed JSON must not panic across the boundary — a panic unwinding
    /// into Swift is undefined behaviour, not an exception.
    #[test]
    fn rubbish_json_returns_null_rather_than_unwinding() {
        let rubbish = CString::new("{ not json at all").unwrap();
        let stamp = CString::new("2026-08-27T10:22:31Z").unwrap();
        assert!(round_trip(|| unsafe { pagify_vcard(rubbish.as_ptr(), stamp.as_ptr()) }).is_none());
    }

    #[test]
    fn freeing_null_is_allowed() {
        // So a caller need not check before freeing.
        unsafe { pagify_string_free(std::ptr::null_mut()) };
    }

    #[test]
    fn several_cards_become_one_file_with_one_timestamp() {
        let cards = CString::new(format!("[{0},{0}]", a_card_json().to_str().unwrap())).unwrap();
        let stamp = CString::new("2026-08-27T10:22:31Z").unwrap();
        let vcard = round_trip(|| unsafe { pagify_vcards(cards.as_ptr(), stamp.as_ptr()) })
            .expect("two cards should produce a file");

        assert_eq!(vcard.matches("BEGIN:VCARD").count(), 2);
        // Everyone sent together left together, which is what a group export
        // means and what its `lastExportedAt` has to agree with.
        assert_eq!(vcard.matches("REV:2026-08-27T10:22:31Z").count(), 2);
    }
}
