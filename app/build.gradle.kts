import org.gradle.internal.os.OperatingSystem
// Imported rather than written as `java.util.Properties`: inside a Gradle Kotlin
// DSL script `java` resolves to the JavaPluginExtension, not the package root.
import java.io.File
import java.util.Properties

plugins {
    alias(libs.plugins.android.application)
    // No `kotlin.android` here: AGP 9 provides Kotlin compilation itself.
    alias(libs.plugins.kotlin.compose)
}

// --------------------------------------------------------------------------
// Rust core
//
// Built with `cargo-ndk` rather than the `rust-android-gradle` plugin: that
// plugin has not tracked AGP 9, and cargo-ndk is a single well-maintained binary
// that already knows how to point the NDK's clang at each Rust target. The cost
// is this ~40 lines of wiring; the benefit is a build that does not break on the
// next AGP bump.
// --------------------------------------------------------------------------

val rustCrateDir = rootProject.layout.projectDirectory.dir("rust/pdf_core")
val rustProfile = (findProperty("pagify.rustProfile") as String?) ?: "release"
val targetAbis = ((findProperty("pagify.abis") as String?) ?: "arm64-v8a")
    .split(',')
    .map(String::trim)
    .filter(String::isNotEmpty)

// cargo-ndk writes `<out>/<abi>/libpdf_core.so`, which is exactly the layout
// AGP expects from a jniLibs source directory.
val rustJniLibsDir = layout.buildDirectory.dir("rustJniLibs")

/** Kept in sync with `android.ndkVersion` below; referenced before that block runs. */
val ndkVersionForRust = "29.0.14206865"

val buildRustCore = tasks.register<Exec>("buildRustCore") {
    group = "build"
    description = "Cross-compiles the pdf_core Rust crate for ${targetAbis.joinToString()}"

    workingDir = rustCrateDir.asFile

    // Only re-runs when the crate or the requested output actually changes.
    inputs.dir(rustCrateDir.dir("src"))
    inputs.file(rustCrateDir.file("Cargo.toml"))
    inputs.property("profile", rustProfile)
    inputs.property("abis", targetAbis)
    outputs.dir(rustJniLibsDir)

    val cargo = if (OperatingSystem.current().isWindows) "cargo.exe" else "cargo"
    val args = mutableListOf(cargo, "ndk")
    targetAbis.forEach { abi -> args += listOf("-t", abi) }
    args += listOf(
        // 24 is the app's minSdk; cargo-ndk uses it to pick the right sysroot stubs.
        "--platform", "24",
        "-o", rustJniLibsDir.get().asFile.absolutePath,
        "build",
    )
    if (rustProfile == "release") args += "--release"

    commandLine(args)

    doFirst {
        // cargo-ndk resolves the NDK from these environment variables. AGP 9
        // removed `android.ndkDirectory`, so the path is derived from the SDK
        // location instead — which is the normal case on a fresh clone, where
        // neither variable is set.
        if (System.getenv("ANDROID_NDK_HOME") == null && System.getenv("ANDROID_NDK_ROOT") == null) {
            val ndkDir = File(File(resolveSdkDir(), "ndk"), ndkVersionForRust)
            require(ndkDir.exists()) {
                "NDK $ndkVersionForRust not found at $ndkDir. Install it via the SDK Manager " +
                    "(\"NDK (Side by side)\"), or set ANDROID_NDK_HOME."
            }
            environment("ANDROID_NDK_HOME", ndkDir.absolutePath)
        }
    }

    doLast {
        // cargo-ndk copies every .so cargo produced, and pdfium-render declares a
        // `cdylib` crate-type of its own — so a `libpdfium_render-<hash>.so` lands
        // here that nothing ever dlopen()s. Left alone it is ~0.3 MB of dead
        // weight per ABI in the shipped APK.
        rustJniLibsDir.get().asFile.walkTopDown()
            .filter { it.isFile && it.name.endsWith(".so") && it.name != "libpdf_core.so" }
            .forEach { stray ->
                logger.info("pruning unused native artifact ${stray.name}")
                stray.delete()
            }
    }
}

/** ANDROID_HOME / ANDROID_SDK_ROOT / local.properties, in the order AGP itself uses. */
fun resolveSdkDir(): File {
    System.getenv("ANDROID_HOME")?.let { return File(it) }
    System.getenv("ANDROID_SDK_ROOT")?.let { return File(it) }

    val localProperties = rootProject.file("local.properties")
    require(localProperties.exists()) {
        "No Android SDK found. Set ANDROID_HOME, or create local.properties with " +
            "sdk.dir=<path> (use forward slashes)."
    }
    val properties = Properties().apply {
        localProperties.inputStream().use { stream -> load(stream) }
    }
    val sdkDir = properties.getProperty("sdk.dir")
    require(!sdkDir.isNullOrBlank()) { "local.properties does not define sdk.dir" }
    return File(sdkDir)
}

// `preBuild` is the one task name that has been stable across AGP versions;
// hooking the variant-specific JNI merge tasks by name breaks on every bump.
tasks.named("preBuild") { dependsOn(buildRustCore) }

android {
    namespace = "com.hsilighting.pagify"
    compileSdk = 37
    ndkVersion = ndkVersionForRust

    defaultConfig {
        applicationId = "com.hsilighting.pagify"
        minSdk = 24
        targetSdk = 37
        versionCode = 7
        versionName = "0.1.6"

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"

        ndk {
            abiFilters += targetAbis
        }
    }

    sourceSets {
        getByName("main") {
            // libpdfium.so ships from the default src/main/jniLibs (checked in,
            // pinned to chromium/7881); libpdf_core.so is generated into build/.
            // AGP 9 replaced the deprecated `srcDirs(...)` with this mutable set.
            jniLibs.directories.add(rustJniLibsDir.get().asFile.absolutePath)
        }
    }

    packaging {
        jniLibs {
            // Keeps the .so files mmap'd inside the APK instead of being unpacked
            // on install: smaller install footprint, and required for the 16 KB
            // page alignment Android 15+ expects to be honoured.
            useLegacyPackaging = false
        }
    }

    // Signing for a release build, when the key is on this machine.
    //
    // Read from a properties file outside version control rather than written
    // here: the password is a real secret, and a build file is the first place
    // anyone looks. Without that file the release build is simply unsigned, which
    // is the right failure — an APK signed with some fallback key would install
    // once and then refuse every update, with nothing to say why.
    val signingProperties = rootProject.file("keystore/keystore.properties")
    val releaseSigning: Properties? = if (signingProperties.exists()) {
        val loaded = Properties()
        signingProperties.inputStream().use { stream -> loaded.load(stream) }
        loaded
    } else {
        null
    }

    signingConfigs {
        if (releaseSigning != null) {
            create("release") {
                storeFile = rootProject.file(releaseSigning.getProperty("storeFile"))
                storePassword = releaseSigning.getProperty("storePassword")
                keyAlias = releaseSigning.getProperty("keyAlias")
                keyPassword = releaseSigning.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        debug {
            isMinifyEnabled = false
        }
        release {
            signingConfig = signingConfigs.findByName("release")
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlin {
        compilerOptions {
            jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
        }
    }

    buildFeatures {
        compose = true
    }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.activity.compose)
    implementation(libs.kotlinx.coroutines.android)

    // On-device OCR. The bundled model rather than the Play-Services-downloaded
    // one: this is an offline reader, and a text layer that only appears once the
    // device has fetched a model is worse than none at all.
    implementation(libs.mlkit.text.recognition)

    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.graphics)
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.material.icons.extended)

    debugImplementation(libs.androidx.compose.ui.tooling)
    debugImplementation(libs.androidx.compose.ui.test.manifest)

    testImplementation(libs.junit)
    // Shadows the stubbed org.json in the unit-test android.jar, whose methods all
    // throw "not mocked". Without this, PdfMetadata/TextSegment cannot be tested
    // off-device at all.
    testImplementation(libs.org.json)

    androidTestImplementation(platform(libs.androidx.compose.bom))
    androidTestImplementation(libs.androidx.junit)
    androidTestImplementation(libs.androidx.espresso.core)
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
}
