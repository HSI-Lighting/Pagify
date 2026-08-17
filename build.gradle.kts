// AGP 9 has built-in Kotlin support and registers the `kotlin` extension itself,
// so `org.jetbrains.kotlin.android` must NOT be applied — doing so fails with
// "Cannot add extension with name 'kotlin'". Only the Compose compiler plugin is
// applied on top.
plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.kotlin.compose) apply false
}
