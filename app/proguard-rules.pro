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

# ML Kit finds its components by reflection at startup: an Android component
# discovery service instantiates each registrar by name, through a no-argument
# constructor R8 cannot see anybody calling. Stripped, every registrar throws
# NoSuchMethodException during discovery and the app dies before it draws —
# release builds only, which is exactly how it was found.
#
# Adding barcode scanning is what surfaced this. The text recogniser has the
# same shape and was equally unprotected; it survived only because nothing had
# forced discovery to run this early.
-keep class com.google.mlkit.** { *; }
-keep interface com.google.mlkit.** { *; }
-keep class com.google.android.gms.internal.mlkit_** { *; }
-dontwarn com.google.mlkit.**
