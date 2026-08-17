//! Error type for the whole engine, plus the mapping onto the Java exception
//! classes that `NativeBridge` declares it throws.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, PdfError>;

#[derive(Debug, Error)]
pub enum PdfError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    /// Anything the PDFium backend reported. Stringified at the boundary because
    /// `PdfiumError` is not available when the crate is built for a host test run.
    #[error("pdfium error: {0}")]
    Pdfium(String),

    /// `Pdfium::bind_to_system_library()` failed — almost always a packaging fault
    /// (libpdfium.so missing from the APK for this ABI) rather than a bad document.
    #[error("could not load libpdfium.so: {0}")]
    LibraryUnavailable(String),

    #[error("no open document with handle {0}")]
    InvalidHandle(i64),

    #[error("page {index} out of range (document has {count} pages)")]
    PageOutOfRange { index: usize, count: usize },

    #[error("document is password protected")]
    PasswordRequired,

    #[error("incorrect password")]
    IncorrectPassword,

    #[error("file is not a PDF or is damaged beyond recovery")]
    MalformedDocument,

    /// The caller passed a target bitmap that does not match the requested render.
    #[error("invalid bitmap: {0}")]
    InvalidBitmap(String),

    #[error("requested render is too large: {width}x{height} px")]
    RenderTooLarge { width: u32, height: u32 },

    #[error("{0}")]
    InvalidArgument(String),

    /// Raised by `catch_unwind` at the JNI boundary.
    #[error("internal error: {0}")]
    Panic(String),

    #[error("{0} is not implemented yet")]
    Unsupported(&'static str),
}

impl PdfError {
    /// Fully-qualified Java class this error should surface as. `NativeBridge`
    /// catches these and re-wraps them into the Kotlin `PdfException` hierarchy.
    pub fn java_exception_class(&self) -> &'static str {
        match self {
            PdfError::Io(_) => "java/io/IOException",
            PdfError::PasswordRequired | PdfError::IncorrectPassword => {
                "com/hsilighting/pagify/core/PdfPasswordException"
            }
            PdfError::PageOutOfRange { .. } => "java/lang/IndexOutOfBoundsException",
            PdfError::InvalidHandle(_)
            | PdfError::InvalidArgument(_)
            | PdfError::InvalidBitmap(_)
            | PdfError::RenderTooLarge { .. } => "java/lang/IllegalArgumentException",
            PdfError::Unsupported(_) => "java/lang/UnsupportedOperationException",
            _ => "com/hsilighting/pagify/core/PdfNativeException",
        }
    }
}

/// PDFium reports "wrong password" and "damaged file" through the same error
/// channel, so the textual form is the only thing available to tell them apart.
/// Isolated here so the guesswork has exactly one home.
pub(crate) fn classify_pdfium_load_error(message: &str) -> PdfError {
    let lowered = message.to_ascii_lowercase();
    if lowered.contains("password") {
        PdfError::IncorrectPassword
    } else if lowered.contains("format") || lowered.contains("corrupt") {
        PdfError::MalformedDocument
    } else {
        PdfError::Pdfium(message.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_errors_map_to_the_password_exception() {
        let class = PdfError::PasswordRequired.java_exception_class();
        assert_eq!(class, "com/hsilighting/pagify/core/PdfPasswordException");
        assert_eq!(PdfError::IncorrectPassword.java_exception_class(), class);
    }

    #[test]
    fn page_range_errors_map_to_index_out_of_bounds() {
        let err = PdfError::PageOutOfRange {
            index: 9,
            count: 3,
        };
        assert_eq!(err.java_exception_class(), "java/lang/IndexOutOfBoundsException");
        assert_eq!(
            err.to_string(),
            "page 9 out of range (document has 3 pages)"
        );
    }

    #[test]
    fn unknown_failures_fall_back_to_the_generic_native_exception() {
        let err = PdfError::Pdfium("something went sideways".into());
        assert_eq!(
            err.java_exception_class(),
            "com/hsilighting/pagify/core/PdfNativeException"
        );
    }

    #[test]
    fn load_error_classification_separates_password_from_damage() {
        assert!(matches!(
            classify_pdfium_load_error("Incorrect Password"),
            PdfError::IncorrectPassword
        ));
        assert!(matches!(
            classify_pdfium_load_error("File not in PDF format"),
            PdfError::MalformedDocument
        ));
        assert!(matches!(
            classify_pdfium_load_error("unrecognised"),
            PdfError::Pdfium(_)
        ));
    }
}
