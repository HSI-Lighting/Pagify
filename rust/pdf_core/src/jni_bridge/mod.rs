//! JNI surface. Compiled only for Android.
//!
//! Two rules hold for every exported function in [`bridge`]:
//!
//! 1. The body runs inside [`guard`], so a Rust panic becomes a Java exception
//!    instead of unwinding into the JVM (which is undefined behaviour).
//! 2. Errors are thrown as typed Java exceptions, never encoded into sentinel
//!    return values — the Kotlin layer relies on that to build its sealed
//!    `PdfException` hierarchy.

pub mod android_bitmap;
pub mod bridge;

use jni::objects::JString;
use jni::JNIEnv;

use crate::error::{PdfError, Result};

/// Run a fallible body, converting both errors and panics into Java exceptions.
///
/// `fallback` is returned after an exception is queued. The JVM ignores the
/// returned value once an exception is pending, so it only has to be *a* value —
/// but it must not be one Kotlin could mistake for success, hence the callers all
/// pass an obviously-invalid handle or zero.
pub fn guard<'local, T>(
    env: &mut JNIEnv<'local>,
    fallback: T,
    body: impl FnOnce(&mut JNIEnv<'local>) -> Result<T>,
) -> T {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(env)));

    match outcome {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            throw(env, &error);
            fallback
        }
        Err(payload) => {
            let message = panic_message(&payload);
            log::error!("panic crossing the JNI boundary: {message}");
            throw(env, &PdfError::Panic(message));
            fallback
        }
    }
}

fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

fn throw(env: &mut JNIEnv, error: &PdfError) {
    // Throwing while an exception is already pending aborts the VM, and any JNI
    // call made in that state misbehaves. This can happen when a JNI helper failed
    // inside the body and its error was propagated rather than cleared.
    if env.exception_check().unwrap_or(false) {
        log::warn!("suppressing {error} because a Java exception is already pending");
        return;
    }

    let class = error.java_exception_class();
    let message = error.to_string();

    if env.throw_new(class, &message).is_err() {
        // The custom exception classes live in the app's dex; if the class cannot
        // be found (obfuscation, a stripped build) fall back to something that is
        // guaranteed to exist rather than returning normally with no exception.
        let _ = env.exception_clear();
        let _ = env.throw_new("java/lang/RuntimeException", format!("{class}: {message}"));
    }
}

/// Read an optional Java string. `null` maps to `None`, which is how an absent
/// password is distinguished from an empty one.
pub fn optional_string(env: &mut JNIEnv, value: &JString) -> Result<Option<String>> {
    if value.is_null() {
        return Ok(None);
    }
    let text: String = env
        .get_string(value)
        .map_err(|e| PdfError::InvalidArgument(format!("could not read Java string: {e}")))?
        .into();
    Ok(Some(text))
}

pub fn required_string(env: &mut JNIEnv, value: &JString, name: &str) -> Result<String> {
    optional_string(env, value)?
        .ok_or_else(|| PdfError::InvalidArgument(format!("{name} must not be null")))
}
