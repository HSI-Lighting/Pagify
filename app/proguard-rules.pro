# The JNI symbol names in libpdf_core.so encode this class's package, name and
# method names. Renaming or removing any of them turns every native call into an
# UnsatisfiedLinkError in release builds only — the classic "works in debug"
# failure.
-keepclasseswithmembernames,includedescriptorclasses class com.hsilighting.pagify.core.NativeBridge {
    native <methods>;
}
-keep class com.hsilighting.pagify.core.NativeBridge { *; }

# Native code constructs these by fully-qualified name via JNIEnv::throw_new
# (see rust/pdf_core/src/error.rs), so R8 cannot see the reference.
-keep class com.hsilighting.pagify.core.PdfException { *; }
-keep class com.hsilighting.pagify.core.PdfPasswordException { *; }
-keep class com.hsilighting.pagify.core.PdfNativeException { *; }
