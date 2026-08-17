//! Direct access to an `android.graphics.Bitmap`'s pixels via `libjnigraphics`.
//!
//! This is what makes rendering zero-copy: PDFium writes into the very buffer the
//! Compose layer will draw from, so a page never exists twice in memory and the
//! Java heap sees no per-frame allocation.

use std::ffi::c_void;
use std::os::raw::c_int;

use jni::objects::JObject;
use jni::sys::jobject;
use jni::JNIEnv;

use crate::error::{PdfError, Result};

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct AndroidBitmapInfo {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: i32,
    pub flags: u32,
}

pub const ANDROID_BITMAP_FORMAT_RGBA_8888: i32 = 1;
const ANDROID_BITMAP_RESULT_SUCCESS: c_int = 0;

// Part of the NDK's public API surface; present on every Android device.
#[link(name = "jnigraphics")]
extern "C" {
    fn AndroidBitmap_getInfo(
        env: *mut jni::sys::JNIEnv,
        bitmap: jobject,
        info: *mut AndroidBitmapInfo,
    ) -> c_int;

    fn AndroidBitmap_lockPixels(
        env: *mut jni::sys::JNIEnv,
        bitmap: jobject,
        addr_ptr: *mut *mut c_void,
    ) -> c_int;

    fn AndroidBitmap_unlockPixels(env: *mut jni::sys::JNIEnv, bitmap: jobject) -> c_int;
}

pub fn bitmap_info(env: &mut JNIEnv, bitmap: &JObject) -> Result<AndroidBitmapInfo> {
    let mut info = AndroidBitmapInfo::default();
    let status =
        unsafe { AndroidBitmap_getInfo(env.get_raw(), bitmap.as_raw(), &mut info as *mut _) };
    if status != ANDROID_BITMAP_RESULT_SUCCESS {
        return Err(PdfError::InvalidBitmap(format!(
            "AndroidBitmap_getInfo failed with status {status}"
        )));
    }
    Ok(info)
}

/// Pixels locked for the lifetime of this guard.
///
/// Locking pins the bitmap against GC movement, so it *must* be released — the
/// `Drop` impl guarantees that even if a render fails or panics partway through.
pub struct LockedPixels<'a> {
    env_ptr: *mut jni::sys::JNIEnv,
    bitmap: jobject,
    pub info: AndroidBitmapInfo,
    pixels: &'a mut [u8],
}

impl<'a> LockedPixels<'a> {
    /// # Safety
    /// `bitmap` must be a live, non-null `android.graphics.Bitmap` that is not
    /// already locked, and must outlive the returned guard.
    pub unsafe fn lock(env: &mut JNIEnv, bitmap: &JObject) -> Result<LockedPixels<'a>> {
        if bitmap.is_null() {
            return Err(PdfError::InvalidBitmap("target bitmap is null".into()));
        }

        let info = bitmap_info(env, bitmap)?;

        if info.format != ANDROID_BITMAP_FORMAT_RGBA_8888 {
            return Err(PdfError::InvalidBitmap(format!(
                "target bitmap must be ARGB_8888 (format {ANDROID_BITMAP_FORMAT_RGBA_8888}), got format {}",
                info.format
            )));
        }
        if info.width == 0 || info.height == 0 {
            return Err(PdfError::InvalidBitmap(format!(
                "target bitmap is {}x{}",
                info.width, info.height
            )));
        }

        let env_ptr = env.get_raw();
        let raw_bitmap = bitmap.as_raw();

        let mut addr: *mut c_void = std::ptr::null_mut();
        let status = unsafe { AndroidBitmap_lockPixels(env_ptr, raw_bitmap, &mut addr) };
        if status != ANDROID_BITMAP_RESULT_SUCCESS || addr.is_null() {
            return Err(PdfError::InvalidBitmap(format!(
                "AndroidBitmap_lockPixels failed with status {status}"
            )));
        }

        let len = info.stride as usize * info.height as usize;
        let pixels = unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, len) };

        Ok(LockedPixels {
            env_ptr,
            bitmap: raw_bitmap,
            info,
            pixels,
        })
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.pixels
    }
}

impl<'a> Drop for LockedPixels<'a> {
    fn drop(&mut self) {
        let status = unsafe { AndroidBitmap_unlockPixels(self.env_ptr, self.bitmap) };
        if status != ANDROID_BITMAP_RESULT_SUCCESS {
            // Nothing useful to do at this point, but a silent failure here would
            // show up much later as a bitmap that never redraws.
            log::error!("AndroidBitmap_unlockPixels failed with status {status}");
        }
    }
}
