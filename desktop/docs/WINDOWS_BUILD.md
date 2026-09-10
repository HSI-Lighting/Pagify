# Building Pagify for Windows

Written after an attempt from macOS that got as far as the linker. Nothing here
is speculative about the *code* — the Windows paths already exist and are
already exercised. What is missing is a toolchain and a place to run the result.

---

## 1. What already works

| | |
|---|---|
| `x86_64-pc-windows-msvc` target | **installed** (`rustup target list --installed`) |
| PDFium for Windows | **vendored** — `third_party/pdfium/pdfium-win-x64/bin/pdfium.dll` |
| The code's Windows branches | **written** — `pdfium.rs` maps the slice at compile time |
| Windows-specific eframe features | **declared** — the non-Linux `eframe` entry omits `wayland`/`x11` |

**PDFium is loaded at runtime, not linked.** `pdfium-render` is taken with
`default-features = false` and only `pdfium_latest` + `thread_safe`, so the
`static` feature is off and the library is opened with `dlopen`/`LoadLibrary`.

That matters more than it sounds: it means a Windows build needs **no import
library for PDFium**, and the MSVC-versus-GNU ABI question only has to be
answered for Rust's own code, not for the C++ library.

---

## 2. What is missing

Exactly one thing on this machine: **a linker that emits PE binaries.**

```
error: linker `link.exe` not found
note: the msvc targets depend on the msvc linker but `link.exe` was not found
```

Everything compiled. `pdfium-render` and `rten` both reached the link step and
stopped there.

---

## 3. Three routes

### A. mingw-w64, cross-compiled from this Mac

```bash
brew install mingw-w64
rustup target add x86_64-pc-windows-gnu
cargo build --release -p pagify_app --target x86_64-pc-windows-gnu
```

Nothing to accept, ~200 MB, reversible. Produces a GNU-ABI binary.

**The caveat, stated plainly:** `pdfium.dll` is an MSVC build. Calls into it go
through the C ABI, which is the same either way, so this *should* work — and
"should" is doing real work in that sentence, because it cannot be checked
without running it on Windows.

### B. cargo-xwin — a true MSVC build

```bash
brew install llvm            # for lld-link
cargo install cargo-xwin
cargo xwin build --release -p pagify_app --target x86_64-pc-windows-msvc
```

Produces the same ABI `pdfium.dll` was built against, removing the caveat above.

**It downloads Microsoft's CRT and Windows SDK headers and libraries**, which is
a licence to accept — not a technical decision, and not one to be made on
somebody's behalf.

### C. Build natively on Windows

Install Rust and **Visual Studio Build Tools with the "Desktop development with
C++" workload** (VS Code is not sufficient — it is a different product), then:

```bash
cargo build --release -p pagify_app
```

Slower to set up and the only route where the binary is built and run on the
same platform.

---

## 4. What the built binary needs beside it

Two directories, and the code already looks for both.

```
Pagify\
  pagify_app.exe
  pdfium.dll          <- third_party\pdfium\pdfium-win-x64\bin\pdfium.dll
  ocr\
    text-detection.rten
    text-recognition.rten
```

- **`pdfium.dll` beside the executable.** `pagify_shell::pdfium` maps the slice
  at compile time — `("pdfium-win-x64", "bin/pdfium.dll")` — and falls back to
  `PAGIFY_PDFIUM_LIB` and then the system loader. Windows searches the
  executable's own directory first, so beside the `.exe` is enough.
- **`ocr\` beside the executable.** `models::candidates` already tries
  `exe_dir/ocr`; the `../Resources/ocr` entry above it is the Mac bundle layout
  and simply will not exist. The models are **SHA-256 verified before loading**,
  so a partial copy fails by name rather than by crashing inside the runtime.

Both are plain file copies. There is no installer and no registry work.

---

## 5. What cannot be claimed without running it

A binary produced here would be **built and never executed**. Handing that over
as "the Windows build" would be overclaiming, so these are the things a first run
has to establish rather than assume:

- **PDFium loads and renders.** The dynamic load is the one place the ABI
  question becomes real.
- **`eframe`/`glow` gets a GL context.** Windows GL drivers vary in a way macOS
  does not; this is the likeliest place a cross-built binary falls over first.
- **Path handling.** Every path in the app goes through `PathBuf`, and
  `resolve_path` handles `~` and escaped spaces — neither of which is how Windows
  spells anything. Opening a file from `C:\Users\…` is worth trying early.
- **The file dialog.** `rfd` is used for Open; it has a Windows backend, and it
  has never been run.
- **OCR.** `rten` is pure Rust and should be indifferent to the platform. The
  482 MB peak measured on macOS is the number to re-measure, because the tiling
  budget is sized from it.

## 6. The honest summary

The code is ready and the assets are vendored. What is missing is a linker and a
Windows machine — one of which is a download, and the other of which is not
something a Mac can supply.

Route **C** is the one that produces something anybody should ship. Routes A and
B produce something worth *testing*, which is a different and still useful thing.
