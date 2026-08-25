# Porting Pagify to iOS

Written for a coding agent picking this up cold. It covers what already ports,
what does not, and the things that have already cost a day each on Android and
will cost the same again if rediscovered.

Current state: Android only, `main` + branch `text-shaping-and-fonts`.
Repo `HSI-Lighting/Pagify`, bundle id on Android `com.hsilighting.pagify`.

---

## 1. What Pagify is

A PDF reader and annotator. You open a PDF, mark it up — highlighter, pen,
shapes, arrows, clouds, text captions, signatures — reorder and delete pages,
add blank pages, capture a region as an image, and save back to the file or to a
copy. Markup is written into the PDF as real page content, not as a sidecar.

---

## 2. The one fact that shapes the whole port

**The engine is already platform-neutral.** `rust/pdf_core` is ~12,000 lines of
Rust, and only one module is Android-specific:

```rust
// rust/pdf_core/src/lib.rs
#[cfg(target_os = "android")]
pub mod jni_bridge;
```

Everything else — document model, command/undo history, rendering, caching,
capture export, markup rasterisation, text shaping, font subsetting — compiles
and unit-tests on the host with no Android in sight. The host test suite is where
most of the coverage lives and it already runs on macOS/Linux/Windows.

So the port is:

1. a new FFI module beside `jni_bridge` (call it `ffi` or `swift_bridge`),
2. PDFium built for iOS,
3. a SwiftUI app reproducing ~22,000 lines of Kotlin UI.

**Do not fork the engine.** Add `#[cfg(target_os = "ios")] pub mod ffi;` and keep
one crate. Every bug fixed in the shared core is fixed for both platforms; a fork
means fixing the ToUnicode bug (§7.1) twice.

---

## 3. Repo map

```
rust/
  pdf_core/                     the engine, ~11,900 lines
    src/
      document/                 Document/DocumentMut traits, PDFium impl
        pdfium_doc.rs           the big one: open, render, annotate, save
        blank.rs                building a blank document (paper, ruling)
        metadata.rs
      command/                  Command enum + undo/redo history
      render/                   RenderTarget, cache, region/viewport capture,
                                markup rasterisation (tiny-skia), export,
                                stroke recognition
      text/mod.rs               shaping, ToUnicode, font registry, subsetting
      engine.rs                 session-level operations
      registry.rs               handle -> open document
      error.rs
      jni_bridge/               ANDROID ONLY — the thing you replace
        bridge.rs               33 #[no_mangle] entry points
        android_bitmap.rs       locks an Android Bitmap's pixels
    examples/                   probes; see §9
    tests/round_trip.rs
  vendor/pdfium-render/         patched copy, see §4.2

app/                            Android app, ~22,200 lines Kotlin
  src/main/java/com/hsilighting/pagify/
    core/                       platform-side model, layout, bridge wrapper
    data/                       repository, settings, recents
    ui/                         Compose: library, reader, components
  src/main/assets/fonts/        54 MB of bundled fonts
  src/main/jniLibs/<abi>/       libpdfium.so, checked in

tools/fetch_pdfium.ps1          downloads the pinned PDFium for Android
docs/EDITING_ROADMAP.md
```

---

## 4. PDFium on iOS — do this first

This is the piece most likely to stall the port. Settle it before writing any
Swift.

### 4.1 The pin is load-bearing

`pdfium-render` binds a *specific* PDFium C API surface per feature flag. The
crate is configured as:

```toml
pdfium-render = { version = "0.9.3", default-features = false, features = [
    "pdfium_latest",   # == pdfium_7881
    "thread_safe",
] }
```

`pdfium_latest` on 0.9.3 means **chromium/7881**, which is why Android ships
`libpdfium.so` from that exact tag (`tools/fetch_pdfium.ps1`). Pair those
bindings with a different PDFium and the dynamic loader fails to resolve a symbol
at the first render rather than at link time — a crash far from the cause.

**Get an iOS PDFium of the same tag.** The usual source is the
`bblanchon/pdfium-binaries` releases, which publish iOS device and simulator
builds; confirm the tag matches chromium/7881 before using it. If only a
different tag is available, bump `pdfium-render` and its `pdfium_NNNN` feature to
match, and re-run the whole engine test suite — do not mix.

### 4.2 The vendored patch

`rust/vendor/pdfium-render` is 0.9.3 with **one** change: `PdfDocument::handle()`
is `pub` instead of `pub(crate)`. That is what makes incremental save, page
deletion, page reordering and font embedding reachable at all. It is pulled in
via `[patch.crates-io]`. Keep it. `rust/vendor/README.md` explains the reasoning
and how to re-apply on an upgrade.

### 4.3 Static vs dynamic

Android `dlopen`s `libpdfium.so` by soname:

```rust
// rust/pdf_core/src/document/pdfium_doc.rs
let bindings = match std::env::var("PAGIFY_PDFIUM_LIB") {
    Ok(path) => Pdfium::bind_to_library(&path)?,      // host tests
    Err(_)   => Pdfium::bind_to_system_library()?,    // Android
};
```

On iOS, prefer **static linking**. `pdfium-render` has a `static` feature and
`Pdfium::bind_to_statically_linked_library()`; there is also a `core_graphics`
feature which implies `static`. So:

- add a `#[cfg(target_os = "ios")]` arm calling
  `bind_to_statically_linked_library()`,
- enable the `static` feature for the iOS target only (a
  `[target.'cfg(target_os = "ios")'.dependencies]` block, mirroring how `jni` is
  already gated),
- link `libpdfium.a` plus `libc++` in the Xcode target.

Dynamic also works (embed a `.framework`), but static avoids a second binary to
sign and a `dlopen` path to get wrong.

**Verify early with a throwaway probe**: open a PDF, render page 0, check the
buffer is non-blank. Do it before any UI exists.

---

## 5. The FFI surface to replace

`jni_bridge/bridge.rs` has **33** `#[no_mangle]` entry points. That is the whole
contract between platform and engine; reproduce it as a C ABI and call it from
Swift.

Grouped by what they do:

**Lifecycle / documents**
`nativeInit`, `nativeVersion`, `openDocument(path, password)`,
`openDocumentFd(fd, password)`, `closeDocument(handle)`, `openDocumentCount`

**Reading**
`getPageCount`, `getPageSize`, `getMetadataJson`, `getPageText`,
`getTextSegmentsJson`, `getPageCharactersJson`, `getAnnotationsJson`,
`getTextMarksJson`, `getPageRotation`

**Rendering**
`renderPageInto(handle, page, zoom, rotation, bitmap) -> wasCacheHit`,
`prefetchPage`, `captureRegion`, `captureViewport`

**Editing**
`executeCommandJson(handle, json) -> json`, `undoEdit`, `redoEdit`,
`getEditStateJson`, `saveToFd(handle, fd, incremental)`,
`createBlankDocument(fd, pages, w, h, fill, ruling)`

**Text and fonts** (new; see §7)
`registerFont(name, bytes)`, `fontCovers(name, text) -> bool`,
`shapeTextJson(name, text) -> json`

**Memory**
`setCacheBudgetBytes`, `clearCache`, `getCacheStatsJson`, `onTrimMemory(level)`

**Other**
`recogniseStroke(pointsJson) -> json`

### 5.1 Why this is smaller than it looks

Most of the editing API is **one function**: `executeCommandJson`. The whole
command set (insert/delete/rotate/reorder pages, add/remove annotations, add
text) is a serde-tagged enum serialised as JSON. Adding an editing feature does
not change the FFI. Keep that.

The JSON shapes are `serde(rename_all = "camelCase")` on the Rust types in
`document/mod.rs` and `command/mod.rs`. Read those, not the Kotlin.

### 5.2 Mechanical translation notes

- **Panics must not cross into Swift.** Every JNI entry wraps its body in
  `catch_unwind` (`jni_bridge/mod.rs::guard`) and converts a panic to a Java
  exception. The profile deliberately keeps `panic = "unwind"` for this reason.
  Do the same for Swift: catch, and return an error code plus a message rather
  than letting it unwind.
- **File descriptors port unchanged.** `openDocumentFd` and `saveToFd` take a raw
  fd and adopt ownership. Swift gets one from `FileHandle.fileDescriptor`. Keep
  the ownership contract: the callee always closes.
- **Strings**: JNI `JString` → `char *` UTF-8 in, caller-frees `char *` out, or a
  length-prefixed buffer. Pick one and be consistent.
- **`onTrimMemory`** has no direct iOS twin; wire it to
  `didReceiveMemoryWarning` / `NSProcessInfo` pressure notifications.

### 5.3 The one genuinely Android-shaped entry

`renderPageInto` takes a Java `Bitmap` and locks its pixels
(`jni_bridge/android_bitmap.rs`). Everything under it already works on a plain
buffer:

```rust
RenderTarget::new(width, height, stride, PixelOrder::Rgba, slice)
```

So on iOS, pass a pointer + width/height/stride from a `CGBitmapContext` (or a
`CVPixelBuffer`) and drop `android_bitmap.rs` entirely.

**Pixel order — measured, not assumed.** PDFium asked for `FPDFBitmap_BGRA`
actually emits **red in byte 0** (verified on 151.0.7881/arm64 by rendering a
known orange). `PixelOrder::Rgba` is therefore the zero-copy path on Android.
Core Graphics normally wants BGRA premultiplied, so on iOS you will most likely
want `PixelOrder::Bgra` with
`CGImageAlphaInfo.premultipliedFirst | .byteOrder32Little`. **Verify with a
known colour before building on it** — the comment in `render/bitmap.rs` explains
how the Android value was established, and the same method applies.

---

## 6. Platform pieces with no direct twin

| Android | iOS equivalent | Notes |
|---|---|---|
| Storage Access Framework picker | `UIDocumentPickerViewController` | |
| `takePersistableUriPermission` | security-scoped bookmark data | **Do not skip.** See §7.4 |
| `Bitmap` + `lockPixels` | `CGBitmapContext` / `CVPixelBuffer` | §5.3 |
| `assets/` | app bundle resources, or On-Demand Resources | §7.3 |
| `Typeface.createFromFile` | `CGFont`/`CTFont` from a `CGDataProvider` | |
| `Canvas.drawGlyphs` (API 31+) | `CTFontDrawGlyphs` | available everywhere; **better** than Android here |
| Compose `pointerInput` passes | SwiftUI gestures / `UIGestureRecognizer` | §8.2 |
| `ViewModel` + `StateFlow` | `@Observable` / `ObservableObject` | |
| DataStore | `UserDefaults` or a small file | settings are tiny |

---

## 7. Things already learned the hard way

Each of these was a real bug. They are properties of PDFium and of the problem,
not of Android, so they will all recur.

### 7.1 Text: shaping is not optional, and ToUnicode fails silently

Text used to be written one character at a time at standard-14 metric widths.
Correct for English; wrong for most of the world. Arabic letters change shape
depending on what they join to, and **a joined form has no character of its
own** — so every letter came out isolated and the word read backwards.

The engine now shapes with `rustybuzz`, splits by direction with `unicode-bidi`,
and writes **glyph ids** against an embedded font. Two traps:

- **`FPDFText_LoadFont` builds an unusable ToUnicode.** It derives one by running
  the font's cmap backwards, and a joined form has nothing to run back to. The
  words drew perfectly and came out of the file as `اϨʹ۰ՍЪة` — unsearchable,
  uncopyable, and completely silent about it. Use
  **`FPDFText_LoadCidType2Font`** with a ToUnicode CMap built from the shaper's
  clusters (`text::to_unicode_from_glyphs`).
- **That call returns `null` with no explanation** unless you hand it an explicit
  identity CID-to-glyph table. Passing none is not "use Identity"; it is a
  failure with no diagnostics.

All of this is in the shared core and ports for free. **What does not** is the
preview: the on-screen renderer must draw by glyph id (`CTFontDrawGlyphs`), or it
shows the isolated letters the file no longer has.

### 7.2 PDFium embeds fonts whole — subset first

A four-character Chinese caption was putting a **16 MB** font into the document.
Every write now subsets to the glyphs actually used first: **16 MB → 2 kB**, and
the text still round-trips. `text::subset` (crate `subsetter`). Subsetting
renumbers glyphs, so the ids written into the page, the ToUnicode and the
CID-to-glyph table must all be the *new* ones — getting that wrong draws the
wrong letters rather than failing.

### 7.3 The fonts are 54 MB

19 files in `app/src/main/assets/fonts/`. The four CJK faces are 45 MB of that
(NotoSansSC 17 MB, TC 12 MB, KR 10 MB, JP 9 MB — variable fonts). The Android APK
went from 56 MB to 93 MB.

On iOS, **strongly consider On-Demand Resources for the CJK four** and bundle the
rest. Nothing in the engine cares where the bytes came from: `registerFont(name,
bytes)` takes them from anywhere. The full list is the `PdfFont` enum in
`core/PdfFonts.kt`, with the asset filename on each entry.

Each face is labelled in its own script (نسخ, বাংলা, 简体中文, 한국어…) and drawn in
its own file in the picker, so a reader finds their script by recognising it. Do
the same on iOS — it is the single thing that made that list usable.

### 7.4 A created file is dead next launch without a persisted grant

Documents created or copied through the picker were unreadable the next time the
app started: the grant expires with the process. Android needed
`takePersistableUriPermission`; iOS needs **security-scoped bookmark data**
stored with the recent-documents entry, and `startAccessingSecurityScopedResource`
around every access. Same bug, same shape, and it only shows up after a restart —
so test by force-quitting, not by navigating back.

### 7.5 Save must not read and write the same file

PDFium reads objects lazily for a document's whole life, so a save reads from the
source while writing. Aiming both ends at one file truncates the input mid-save.
The Android path writes to a scratch file and copies over
(`PdfRepository.writeTo` → `PdfDocument.saveVia`). Keep that shape.

### 7.6 Marks must survive a save to still be erasable

Text and its frame are written as page content tagged with PDFium marked content
(`FPDFPageObj_AddMark`, name `PagifyText`, an int id and a restore blob). Without
the tag, saved text stops being a mark: the eraser could take the ring off a
clouded caption and leave the words. `examples/text_mark_probe.rs` proves the tag
survives a save and can be removed by id.

### 7.7 `debug_assert!` does not run in release

A mutation inside one silently does not happen. This shipped once already.

---

## 8. The UI to reproduce

~22,200 lines of Kotlin. Read `ui/reader/PdfReaderState.kt` first — it is the
whole app state in one file and the fastest way to understand what exists.

### 8.1 Screens

- **Library** (`ui/library/LibraryScreen.kt`) — recents, search, a `+` that asks
  *blank pages* or *open a file*.
- **Reader** (`ui/reader/PdfReaderScreen.kt`) — the app. Continuous page list,
  zoom, thumbnails strip, tool ribbons, page organiser, text selection.
- **Settings** (`ui/settings/SettingsScreen.kt`) — theme, thumbnails, viewfinder.
- **Capture editor** (`ui/components/CaptureEditor.kt`) — a screenshot of a page
  region, marked up with the same tools.

### 8.2 Gestures are where the Android time went

Compose has two pointer passes: Initial travels parent→child, Main child→parent.
Three separate bugs came from that ordering — pinch-to-zoom fighting the drawing
canvas, double-tap zoom dying whenever a tool was armed, and a caption scaling the
page along with itself. On iOS the equivalent battleground is
`UIGestureRecognizer` delegates and `simultaneousRecognition`. Budget real time
for it, and note that **the Android pinch behaviour was never observed working**:
`sendevent` on the test device is permission-denied, so all two-finger behaviour
there is reasoned, not verified. iOS simulator multi-touch will actually let you
test it — do, and report back what the truth is.

### 8.3 Things that look small and are not

- Text on a curved baseline, per-glyph, with the frame (cloud/box/ellipse) fitted
  around a multi-line block.
- The eraser hit-testing marks including embedded text.
- Page reorder/delete with annotation index remapping (`core/AnnotationRemap.kt`).
- Zoomed-page paging: at the bottom of a zoomed page, one more swipe turns it.

---

## 9. Tests and probes

**Run the engine suite first, on the Mac, before writing any Swift.** It needs a
desktop PDFium:

```bash
export PAGIFY_PDFIUM_LIB=/path/to/libpdfium.dylib
cd rust/pdf_core && cargo test
```

Without that variable the PDFium-backed tests **skip silently** — a green run
that proved nothing. Current expected: 199 / 18 / 22 / 6 passing.

`examples/` holds standalone probes, each answering one question that had to be
settled before code was built on top. Read them; they document PDFium behaviour
that is not in any header:

| probe | question |
|---|---|
| `shaping_probe.rs` | does shaped Arabic embed, draw, and stay searchable? |
| `cjk_probe.rs` | do the CJK fonts embed, and how big is the file? |
| `blank_document_probe.rs` | does a document built from nothing come back intact? |
| `text_mark_probe.rs` | does a marked-content tag survive a save? |
| `text_lifecycle_probe.rs` | does a caption's whole life leave the page intact? |
| `text_mark_dump.rs` | what marks does this file actually carry? |

**The house rule for this project: measure, do not infer.** When PDFium's
behaviour matters, write a probe that reads the file back off disk with none of
our own code in the way, and assert on numbers. Every §7 entry was found that
way; several looked fine right up to the assertion.

**Mutation-test every new guard.** Break the thing the test protects and confirm
the test fails. A test that has never failed has proved nothing, and this project
has shipped one that could not fail.

---

## 10. Suggested order

1. **Prove PDFium.** Static-link for arm64 device + simulator, render a page into
   a buffer, dump a PNG, look at it. Nothing else until this works.
2. **Confirm the pixel order** with a known colour (§5.3).
3. **Stand up the FFI** — start with `openDocumentFd`, `getPageCount`,
   `getPageSize`, `renderPageInto`, `closeDocument`. That is a working reader.
4. **A scrolling page view** with zoom and the render cache wired to memory
   warnings.
5. **The document picker** with security-scoped bookmarks (§7.4) and recents.
6. **`executeCommandJson` + undo/redo + `saveToFd`** — the whole editing model
   arrives in one piece.
7. **Markup tools**, starting with pen and highlighter; the ribbon last.
8. **Text and fonts** — `registerFont` / `shapeTextJson` / `CTFontDrawGlyphs`.
   Test with Persian and Chinese from day one, not at the end.
9. **Capture editor.**

Milestones 1–4 are the risky half. After that it is mostly Swift against an API
that already works.

---

## 11. Open questions to settle early

- **PDFium chromium/7881 for iOS** — does a prebuilt of that exact tag exist? If
  not, which tag, and what does bumping `pdfium-render` cost? (§4.1)
- **Pixel order and premultiplication** on Core Graphics. (§5.3)
- **CJK fonts**: bundled (+45 MB) or On-Demand Resources? (§7.3)
- **Minimum iOS version.** `CTFontDrawGlyphs` is ancient, so the text path does
  not constrain it; pick from SwiftUI needs.
- **Does the Rust `subsetter` crate's MSRV bite?** It resolved to 0.2.6 needing
  rustc 1.85 against a declared `rust-version = "1.75"`; it builds on 1.96. If
  the iOS toolchain is older, pin `subsetter = "=0.2.2"`.

---

## 12. Do not

- Fork `pdf_core`. Add a `cfg`-gated FFI module.
- Reimplement shaping, subsetting, ToUnicode, markup rasterisation or capture
  export in Swift. All of it is in the core and tested on the host.
- Add a second PDF writer. PDFium is the single writer on purpose: two object
  models writing one file is a corruption class that is very hard to diagnose.
- Trust a green test run without `PAGIFY_PDFIUM_LIB` set.
