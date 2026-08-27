# Building Pagify for iOS

Companion to [`IOS_PORT.md`](IOS_PORT.md), which is the design brief. This is the
operational half: how to build what exists today, and what the port's open
questions turned out to be once they were actually measured.

Current state: milestones 1–4 and most of 6–7 of `IOS_PORT.md` §10 are done, and
verified both in the simulator and on a physical iPhone (12 Pro Max, iOS 26.5).

**Working:** the reader (scroll, pinch zoom), the whole editing model
(`executeCommandJson` + undo/redo + `saveToFd`), the markup tools — highlighter,
pen, line, arrow, rectangle, ellipse, revision cloud, eraser — with colour, nib
width and the five line types, and the page organiser (reorder, rotate, insert
blank, delete).

**Not built yet:** the text and caption tools (§7.1–7.3 — shaping, fonts,
`CTFontDrawGlyphs`), signatures, notes, the capture editor, the library screen
with recents, and persisted security-scoped bookmarks. `ios/Scripts/wire-probe`
covers what does exist. The reader opens a document, renders pages, and scrolls them. None of
the markup, editing, capture or text UI exists yet — though the whole engine
behind all of it is already reachable through the bridge.

---

## 1. What you need

| | |
|---|---|
| Xcode | 26.x with an iOS SDK (built against 26.5) |
| Rust | via **rustup** — Homebrew's `rustc` carries no iOS `std` |
| iOS targets | `rustup target add aarch64-apple-ios aarch64-apple-ios-sim` |
| xcodegen | `brew install xcodegen` — the `.xcodeproj` is generated, not checked in |
| PDFium | fetched by script, see below |

If `xcodebuild` reports *"requires Xcode, but active developer directory is a
command line tools instance"*, point it at Xcode once:

```bash
sudo xcode-select -s /Applications/Xcode.app/Contents/Developer
```

## 2. Build and run

```bash
cd ios && ./Scripts/fetch-pdfium.sh && xcodegen generate && open Pagify.xcodeproj
```

From a terminal instead of Xcode:

```bash
xcodebuild -project ios/Pagify.xcodeproj -scheme Pagify -configuration Debug -destination 'platform=iOS Simulator,name=iPhone 17 Pro' build
```

Cargo runs from a build phase, so there is no separate Rust step to remember and
no checked-in binary to go stale. A device build additionally needs
`DEVELOPMENT_TEAM` set; the simulator override that disables signing does not
apply to it.

## 3. Run the engine suite and the wire probe

The wire format is the thing most likely to break silently — a mistyped JSON key
is not a compile error, and the engine answers a malformed command with a string
nobody reads. `ios/Scripts/run-wire-probe.sh` compiles the app's **own** editing
sources for the host and drives them against the same engine the app calls,
asserting on pixels and state rather than on the absence of an error:

```bash
./ios/Scripts/run-wire-probe.sh
```

It commits every tool, undoes and checks the page came back *exactly*, runs the
page commands, reads the marks back and saves and reopens the file.

### The engine suite

Per `IOS_PORT.md` §9, and it needs a desktop PDFium or **the PDFium-backed tests
skip silently** — a green run that proved nothing:

```bash
PAGIFY_PDFIUM_LIB=$PWD/third_party/pdfium/pdfium-mac-arm64/lib/libpdfium.dylib cargo test --manifest-path rust/pdf_core/Cargo.toml
```

Expected, and current: **199 / 18 / 22 / 6**.

---

## 4. The open questions from §11, answered

**PDFium at chromium/7881 for iOS — does a prebuilt exist?** Yes.
`bblanchon/pdfium-binaries` publishes `pdfium-ios-device-arm64`,
`pdfium-ios-simulator-arm64` and `pdfium-ios-simulator-x64` at that exact tag,
all reporting `MAJOR=151 MINOR=0 BUILD=7881`. The pin holds and `pdfium-render`
did not need bumping.

**Static or dynamic?** Dynamic, against the brief's preference — there is no
static archive at this tag, only `libpdfium.dylib`. It is copied into
`Frameworks/` by a build phase and `dlopen`'d by path;
`pagify_set_pdfium_library_path` hands the engine the bundle path it only learns
at runtime. The device path signs the dylib with the app's identity, which the
simulator does not require and which is therefore easy to leave out and only
discover on hardware.

**Pixel order and premultiplication on Core Graphics.** Measured, not inferred —
`PixelOrderProbe` builds a document painted `A=FF R=FF G=80 B=00`, renders it
both ways and reads the bytes back. Asking the engine for `PixelOrder::Rgba`
yields `FF 80 00 FF`: R in byte 0, exactly as asked. So **RGBA pairs with
`premultipliedLast | byteOrder32Big` and is the zero-copy path on iOS, the same
as on Android**. Asking for BGRA correctly yields `00 80 FF FF`. The colour is
asymmetric on purpose: a grey probe passes whichever way round the channels come
out. `quadrants.pdf` in the bundled fixture list is the visual confirmation —
red, green, blue and yellow have to land in the right corners.

**Minimum iOS version.** 17.0, chosen by SwiftUI, not by the text path.

**Does `subsetter`'s MSRV bite?** No. It builds on the rustup stable used here
(1.98); no pin needed.

**CJK fonts, bundled or On-Demand?** Still open — no fonts are bundled yet.

---

## 5. Things that cost time here, so they need not cost it twice

### 5.1 LTO silently deletes the whole bridge

`[profile.release]` sets `lto = true`, which is right for the Android `.so`
because that gets *linked*. iOS links the `staticlib`, and when the output is an
archive rather than a linked binary, LTO drops the `#[no_mangle]` exports on the
way through: nothing inside the crate references them, and there is no link step
yet to say something outside will.

Measured on this crate: `lto = true` and `lto = "thin"` each produce an archive
defining **0** `_pagify_*` symbols. `lto = false` defines all **37**. Hence
`[profile.release-ios]`, which is the release profile with LTO off — and hence
the check in `Scripts/build-rust.sh`, because the failure surfaces as an
unreadable Xcode link error a long way from the cause:

```bash
nm -g --defined-only libpdf_core.a | grep pagify
```

### 5.2 The simulator will happily run a build the phone cannot load

`crate-type` includes both `cdylib` (for the JVM) and `staticlib`, so Cargo
writes **`libpdf_core.a` and `libpdf_core.dylib` into the same directory**. Given
`-lpdf_core` and a search path, the linker takes the dylib, and the app records
its absolute build path on the build machine as a load command.

That app then runs perfectly in the simulator — the simulator shares the host
filesystem, so `/Users/…/target/aarch64-apple-ios/release-ios/deps/libpdf_core.dylib`
resolves. On a device it does not exist and dyld kills the process before a line
of Swift runs:

```
dyld[918]: Library not loaded: /Users/…/release-ios/deps/libpdf_core.dylib
  Referenced from: …/Pagify.app/Pagify.debug.dylib
```

`devicectl device process launch --console` is what shows this; the app simply
"terminated with exit code 0" otherwise. The fix is in `project.yml`: name the
`.a` by full path instead of searching for `-lpdf_core`, which leaves the linker
no choice to get wrong. Check with:

```bash
otool -L <App>.app/<binary> | grep /Users/
```

Anything matching means the build is host-bound and will not run on hardware.

### 5.3 The app icon is generated, not exported

`ios/Scripts/make-appicon.swift` composites the Android adaptive icon's two
layers into the single flat 1024px square iOS wants. Two things it gets right
that are easy to get wrong by hand: only the central 72dp of the 108dp adaptive
canvas is ever visible, so using the whole canvas renders the mark two-thirds
size; and `CGColor(red:green:blue:alpha:)` makes an **sRGB** colour that shifts
when drawn into a DeviceRGB context — `#3B00E6` came out as `#4D2FEB` until the
colour was built in the context's own space. Verify by reading a background pixel
back off the written PNG.

### 5.4 Fixtures that are blank on purpose

`pages-ladder.pdf` is five pages with **no content streams at all** — a
page-geometry fixture. A blank render of it is the correct render, and half an
hour can go into debugging that. `quadrants.pdf` is the one to look at.

### 5.5 `examples/incremental_probe.rs` did not compile on macOS

`FPDF_SaveWithVersion` takes an `FPDF_DWORD` the bindings widen to `u64`; the
probe passed `u32`. Examples are not built for Android, so it had never been
compiled. It aborted `cargo test` before a single test ran — the exact "green run
that proved nothing" failure mode §9 warns about, arriving by a different door.
Fixed.

---

## 6. Layout

```
ios/
  project.yml                 xcodegen input; the .xcodeproj is generated
  Sources/
    PagifyApp.swift           entry point
    PagifyEngine.swift        start-up, PDFium binding, error plumbing
    PagifyDocument.swift      one open document; render into a CGBitmapContext
    PixelOrderProbe.swift     §5.3 of the brief, measured at launch
    ReaderModel.swift         launch checks, document lifetime, memory trim
    ReaderView.swift          the scrolling page list
    DocumentPicker.swift      UIDocumentPickerViewController + security scope
  CPagifyCore/include/pagify_core.h    the C ABI contract
  Support/                    Info.plist, bridging header
  Resources/                  bundled fixtures
  Scripts/                    fetch-pdfium, build-rust, embed-pdfium
rust/pdf_core/src/ffi/mod.rs  the 37 C entry points — the JNI bridge's twin
third_party/pdfium/           fetched, not checked in
```

## 7. Next, in the brief's order

Milestone 5 onwards. The nearest edges:

- **Security-scoped bookmarks** (§7.4). Access is currently held for as long as a
  document is open, which is correct, but nothing is *persisted* — so a picked
  file is unreachable next launch. Test by force-quitting, not by navigating back.
- **Recents**, which is the same piece of work.
- **`executeCommandJson` + undo/redo + `saveToFd`** (§10.6). All three are already
  exported and unused; the editing model arrives in one piece behind them.
- **Zoom and gestures** (§8.2). Note the brief's standing question: Android's
  pinch behaviour was never observed working, and the simulator can actually test
  two-finger input. Worth settling.
