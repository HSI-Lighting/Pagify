package com.hsilighting.pagify.core

/**
 * Base type for everything the Rust engine throws.
 *
 * The two subclasses below are instantiated *by native code* via
 * `JNIEnv::throw_new`, which requires a `(String)` constructor and requires the
 * classes to survive shrinking — see `proguard-rules.pro`. Their fully-qualified
 * names are hard-coded in `rust/pdf_core/src/error.rs`.
 */
sealed class PdfException(message: String) : RuntimeException(message)

/**
 * The document is encrypted and the supplied password was absent or wrong.
 *
 * Distinguishing "needs a password" from "wrong password" is done on the message
 * because PDFium reports both through one channel; [isRetry] is the resulting
 * best-effort hint for the UI.
 */
class PdfPasswordException(message: String) : PdfException(message) {
    val isRetry: Boolean get() = message?.contains("incorrect", ignoreCase = true) == true
}

/** Anything else: a damaged file, a PDFium failure, or a panic in the core. */
class PdfNativeException(message: String) : PdfException(message)
