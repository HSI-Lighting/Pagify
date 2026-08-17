# Pagify

A PDF reader with a thin Kotlin/Compose UI and a Rust core. Android first;
the core is deliberately platform-agnostic so iOS and desktop can reuse it.

**Status: phase 1.** Open, view, navigate, zoom, rotate, cache. Editing and
digital signatures are designed for but not implemented — see [Roadmap](#roadmap).

---

## Architecture

```
┌─────────────────────────────────────────────┐
│  Kotlin / Compose                            │
│  Screens → ViewModel → Repository            │
└───────────────────┬─────────────────────────┘
                    │  JNI (16 exported symbols)
┌───────────────────▼─────────────────────────┐
│  Rust core (pdf_core)                        │
│    jni_bridge   exceptions, panic barrier    │
│    engine       cache-aware orchestration    │
│    registry     handle ownership             │
│    document     Document/Page + PDFium impl  │
│    render       pixel formats, LRU cache     │
└───────────────────┬─────────────────────────┘
                    │  dlopen
┌───────────────────▼─────────────────────────┐
│  libpdfium.so (chromium/7881, prebuilt)      │
└─────────────────────────────────────────────┘
```

Only `jni_bridge` is Android-specific. Everything beneath it compiles and tests
on the host, which is where most of the test suite runs.

### Three decisions worth knowing

**Rendering is zero-copy.** Kotlin allocates an `ARGB_8888` bitmap; Rust locks
its pixels via `libjnigraphics` and PDFium rasterises directly into the Java heap
object. A page never exists twice in memory.

**The bitmap's dimensions decide the render size**, not the `zoom` argument —
`zoom` only identifies the cache entry. Kotlin's rounding is therefore the single
source of truth, and the two sides cannot disagree about how big a page is.

**Both sides have misleading names, and they happen to agree.** Android's
`ARGB_8888` is R,G,B,A in memory; PDFium's `FPDFBitmap_BGRA` *also* emits R,G,B,A
on little-endian targets. So the handover needs no channel conversion — but that
is a measured fact, not an assumption, and it is pinned in one constant
([`PDFIUM_OUTPUT_ORDER`](rust/pdf_core/src/render/bitmap.rs)) with an instrumented
test using an asymmetric orange fixture as the tripwire. Assuming the documented
BGRA order instead renders every document with red and blue transposed; that bug
was in the first build and the test is what caught it.

---

## Building

### Prerequisites

| Requirement | Version used | Notes |
|---|---|---|
| Android SDK | platform 37, build-tools 36 | |
| Android NDK | `29.0.14206865` | r28+ needed for 16 KB page alignment |
| JDK | 17+ | Android Studio's bundled JBR works |
| Rust | 1.75+ | |
| `cargo-ndk` | 4.x | `cargo install cargo-ndk` |
| Rust targets | | `rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android` |

Create `local.properties`:

```properties
# Forward slashes! Java properties treats \U, \h, \A as escapes, so a
# backslash path silently mangles into "java.io.IOException: Invalid file path".
sdk.dir=C:/Users/you/AppData/Local/Android/Sdk
```

### Build

```bash
./gradlew assembleDebug
```

Gradle drives the Rust build itself — no separate step. Useful flags:

```bash
# Just your device's ABI, for faster iteration
./gradlew assembleDebug -Ppagify.abis=arm64-v8a

# Debug-profile Rust: ~4x faster to build, several times slower to render
./gradlew assembleDebug -Ppagify.rustProfile=debug
```

### Test

```bash
cargo test --manifest-path rust/pdf_core/Cargo.toml   # 64 host tests, no device
./gradlew connectedDebugAndroidTest                   # 18 tests, needs a device
```

> **MIUI/HyperOS devices restrict installing *new* packages.** Updates to a package
> that already exists always work; creating one is what gets refused, with
> `INSTALL_FAILED_USER_RESTRICTED: Install canceled by user`. That hits
> `connectedDebugAndroidTest` on a clean device, and it is **intermittent** — plain
> `adb install` of a new package succeeded once here and was refused later on an
> awake, unlocked device, so it is not simply the screen being locked. When it
> refuses, enable *Developer options → Install via USB* (needs a Mi account and
> network) or install from Android Studio.
>
> When it does let you in, seed both packages once and Gradle stays happy after:
>
> ```bash
> ./gradlew assembleDebug assembleDebugAndroidTest
> adb install -r -t app/build/outputs/apk/debug/app-debug.apk
> adb install -r -t app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk
> ```
>
> After that `./gradlew connectedDebugAndroidTest` works normally. One more trap:
> MIUI's `adb uninstall` reports `DELETE_FAILED_INTERNAL_ERROR` while actually
> removing the package, so an uninstall "failure" in the log is usually not one —
> but it does leave you needing to re-seed, which may then be refused.

The host tests cover pixel-format conversion, cache eviction, handle lifetimes and
the cache-hit/miss logic against a fake document. The instrumented tests cover what
they cannot: the JNI boundary, real PDFium rendering, and session leaks across 200
open/close cycles.

---

## PDFium

`app/src/main/jniLibs/*/libpdfium.so` is checked in, pinned to **chromium/7881**.

The pin is not arbitrary: `pdfium-render 0.9.3`'s `pdfium_latest` feature binds the
7881 API surface, and because binding is dynamic (`dlopen`), a mismatch would
surface as a missing symbol at first render rather than as a link error. If you
bump `pdfium-render`, bump the tag to match its newest `pdfium_NNNN` feature and
re-run:

```powershell
pwsh tools/fetch_pdfium.ps1
```

Both 64-bit ABIs ship 16 KB-aligned (`0x4000`), which Google Play requires for
Android 15+. Verify after any update:

```bash
llvm-readelf -l app/src/main/jniLibs/arm64-v8a/libpdfium.so | grep LOAD
```

---

## Project layout

```
Pagify/
├── app/
│   ├── build.gradle.kts              # also drives the Rust build
│   ├── proguard-rules.pro            # keeps JNI + native-thrown exception classes
│   └── src/
│       ├── main/
│       │   ├── java/com/hsilighting/pagify/
│       │   │   ├── MainActivity.kt
│       │   │   ├── core/             # NativeBridge, PdfDocument, exceptions
│       │   │   ├── data/             # PdfRepository (keeps native calls off the UI thread)
│       │   │   └── ui/               # Compose screens, ViewModel, theme
│       │   └── jniLibs/<abi>/libpdfium.so
│       └── androidTest/              # JNI integration tests + generated fixtures
├── rust/pdf_core/
│   └── src/
│       ├── document/   # Document/Page traits, PDFium impl, metadata
│       ├── render/     # bitmap formats, LRU page cache
│       ├── engine.rs   # cache-aware render orchestration
│       ├── registry.rs # handle ownership
│       ├── jni_bridge/ # JNI exports, Android bitmap locking
│       ├── command/    # undo/redo (phase 3, implemented + tested)
│       └── plugins/    # plugin trait (phase 5)
└── tools/fetch_pdfium.ps1
```

---

## Roadmap

| Phase | Features | Status |
|---|---|---|
| 1 | Viewing, navigation, zoom, rotation, caching | **Done** |
| 2 | Text search, thumbnails, recent documents (Room) | Text extraction API exists; UI pending |
| 3 | Annotation editing | `EditableDocument` trait + command stack in place, `lopdf` behind the `editing` feature |
| 4 | Digital signatures | Types declared |
| 5 | Form filling, plugins | `Plugin` trait in place |

`lopdf` stays behind a feature flag until it is exercised — it is young compared
to PDFium, and untrusted-input parsing is where that difference matters.

---

## Known gaps

- **Verified on one device only** — Xiaomi Pad 5 (arm64-v8a, Android 13 / API 33).
  The armeabi-v7a and x86_64 builds compile and package but are untested at runtime.
- **Two-finger pinch is not hardware-verified.** Double-tap zoom, one-finger
  panning and the navigator were all confirmed on device via screenshots, but a
  real pinch could not be: `adb` has no multitouch primitive, and SELinux blocks
  writing synthetic events to `/dev/input` (being in the `input` group is not
  enough without root). `PinchToZoomTest` covers the handler with Compose's
  multi-pointer injection — including that a one-finger drag must be ignored so
  scrolling still works — but it has not run yet, because MIUI is currently
  refusing to install the test APK. **Please try a pinch by hand.**
- **No tiled rendering.** A page is one bitmap, so cost grows with the square of
  zoom. `RenderScale.MAX_PIXELS` caps it at 16 MP (64 MB) and the bitmap is
  upscaled beyond that, which keeps the app alive but goes soft at high zoom.
  Rendering only the visible tile is the correct fix.
- **Heavy documents (multi-GB, tens of MB per page) still render slowly per
  page** — a thumbnail or full page load on such a file can take 1-3 seconds even
  after the fixes below, because PDFium serialises internally and a page's own
  resource/content-stream parsing dominates. The thumbnail cache, background
  warmer, and interactive-render priority (the warmer yields while you are
  actively waiting on a render) all reduce *repeated* cost and contention, but
  none of them can make a single first render of a heavy page fast — see the
  render-pipeline plan for the tiled-rendering approach that would.
- **Recent documents** are not persisted (phase 2). Persistable URI permissions
  are already taken, so the plumbing is there.
- **No text search or selection UI**, though `getPageText` is exposed.
- **Rotation clears the whole cache** rather than re-keying it. Correct, but does
  more work than necessary.
- **Scroll-anchor drift on mixed-size documents** — understood, and handled where
  it bit. Every page starts at a guessed A4 aspect and resizes once its real
  dimensions arrive; with many items above the viewport those corrections
  accumulate and drag `LazyColumn`'s anchor. This is what produced the earlier
  one-off "Page 17 of 149" on open, and it was reproducible in the thumbnail rail,
  which landed a dozen pages away from the page being read. The rail now
  re-asserts its position whenever the current page drifts out of view (while
  idle, so it never fights a manual scroll). The main reader is not yet given the
  same treatment; the proper fix for both is to measure every page up front —
  `Document::page_size` is now cheap enough to make that practical.
