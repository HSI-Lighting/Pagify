# Print window with live preview — design

Written 2026-10-12 against `pagify-windows-refactor` (clean tree, v0.1.63). Status: **spec awaiting review — no code written.**

## 1. Goal and success criteria

`Ctrl+P` opens one Pagify window: settings on the left, a live preview of the sheet on the right, **Print** sends the job. Done means:

1. What the preview shows is what the printer receives — one layout function feeds both, so they cannot disagree.
2. A landscape page on portrait paper is rotated to fill the sheet instead of shrinking onto half of it (today's behaviour).
3. The OS print dialog is no longer shown. Printer-specific options (duplex, tray, staple, paper size) stay reachable through the driver's own properties dialog.

## 2. Why this has to be our own window (findings)

- The only print path is `PrintDlgExW` (`print_windows.rs:172`, called from `main.rs:9159` via `print_current_document`, triggered by `Ctrl+P` at `main.rs:2960/3021`; there is no ribbon button or verb).
- On Windows 11 22H2+ the OS replaces that classic dialog with the modern one. Its preview pane is filled only by an app that supplies a WinRT print source (`IPrintManagerInterop` + an `IPrintDocumentSource`), so a Win32/egui app gets "This app doesn't support print preview". A WinRT source needs XAML plumbing an egui window cannot host. Rejected.
- Today's layout (`print_windows.rs:323`): scale-to-fit within the printable area, anchored top-left, never rotated; no actual-size mode.
- Drawn ink (highlights, lines, shapes) lives in the app's own `Markup` layer until a save (`main.rs:4870`, `commit_marks_on` at `9666`). Printing renders the *document* (`Session::render_page_rotated`), so **unsaved ink is not printed today.**

## 3. Decisions (agreed with the owner, 2026-10-12)

| Decision | Choice |
|---|---|
| Flow | Own print window (not "OS dialog then preview") |
| Placement | **Centred** in the printable area (was top-left) |
| Unsaved marks | Banner "N unsaved marks on M pages aren't included — save first"; no silent commit. Compositing ink is phase 2 |

## 4. Design

### 4a. Shared layout — `pagify_shell/src/print_layout.rs` (new, pure, no Windows types)

```rust
pub struct Paper {            // all in device pixels, from the printer DC
    pub physical: (u32, u32), // PHYSICALWIDTH/HEIGHT
    pub offset: (u32, u32),   // PHYSICALOFFSETX/Y — the unprintable margin
    pub printable: (u32, u32),// HORZRES/VERTRES
    pub dpi: (f32, f32),      // LOGPIXELSX/Y
}
pub enum Fit { Page, ActualSize }
pub struct Options { pub fit: Fit, pub rotate_to_fit: bool }
pub struct Placement {
    pub rotation: Rotation,   // None or Clockwise90 (pdf_core::document::Rotation)
    pub dest: PxRect,         // relative to the printable origin, which is GDI's (0,0)
    pub clipped: bool,        // ActualSize overflows the printable area
}
pub fn place(paper: &Paper, page_pt: (f32, f32), o: &Options) -> Placement
```

- **Fit::Page** = today's semantics (scale up *or* down to fill the printable area, aspect kept), now centred.
- **Fit::ActualSize**: 72 pt = 1 in at printer dpi. Per axis: centred if it fits, anchored to the printable origin if it overflows (`clipped = true`).
- **rotate_to_fit**: try `None` and `Clockwise90`; keep the one with the larger Fit scale (ActualSize: the one that fits, else the smaller overflow). Ties favour `None`. Off ⇒ always `None`.
- On-screen view rotation (`view_state.rotation`) is ignored exactly as today (`Rotation::None` base). Not changed here.
- Page selection reuses `organize::parse_range(spec, page_count)` (already used by Extract). `resolve_print_pages` and the `PD_*` flag logic are deleted along with `PrintDlgExW`.
- The render scale for a page becomes *placed size ÷ page size*, still clamped by the existing `render_scale()` (engine pixel ceiling, A3@600dpi lesson). The bitmap is then blitted ~1:1 instead of rendered at printer dpi and stretched.

### 4b. Windows layer — `pagify_app/src/print_windows.rs` (grows; `PrintDlgExW` removed)

| Function | Does |
|---|---|
| `list_printers()` | `EnumPrintersW` level 4 (names only — does not wake network devices); default from `GetDefaultPrinterW` |
| `PrinterSetup { name, devmode: Vec<u8> }` | The **whole** `DEVMODEW` buffer incl. `dmDriverExtra` (driver-private bytes must survive) |
| `default_setup(name)` | `OpenPrinterW` + `DocumentPropertiesW` (size query, then `DM_OUT_BUFFER`) |
| `apply(&mut setup, orientation, copies, collate)` | Set the field + its `dmFields` bit, then `DM_IN_BUFFER \| DM_OUT_BUFFER` so the driver validates/merges |
| `show_properties(owner, &mut setup)` | `DocumentPropertiesW` with `DM_IN_PROMPT` — the driver's own dialog; owner = `hub::active_window()` as `main.rs:9158` does |
| `paper(&setup) -> Paper` | Throwaway `CreateDCW` + `GetDeviceCaps`, then `DeleteDC`. Re-run only when the printer/orientation/properties change |
| `print(&setup, session, title, pages, &Options, output: Option<&Path>)` | The existing GDI job (`StartDocW`… `EndDoc`), driven by `place()`. `output` sets `DOCINFOW.lpszOutput` — used by the end-to-end test so "Microsoft Print to PDF" does not open a Save dialog |

New Cargo feature: `Win32_Graphics_Printing`. Everything here is `cfg(windows)`; other platforms keep "printing isn't available on this platform yet."

### 4c. UI — `pagify_app` (follows the Extract dialog pattern)

- `PrintAsk` in `PanelsState` (`print_ask: Option<PrintAsk>`, cfg-gated like the module), next to `extract_ask`. Holds: printer list, `PrinterSetup`, `Paper`, orientation, copies, collate, pages text, `Options`, current sheet index, preview texture + its key `(page, rotation, size)`, `problem`.
- `draw_print_dialog(ctx)` in `panels.rs`'s style: `egui::Modal`, drawn from the same place `draw_extract_dialog` is. `Ctrl+P` calls `open_print_window()` instead of `print_current_document`.
- **Left column:** printer combo · *Properties…* · Portrait/Landscape · Copies (+ Collate when >1) · Pages (field + *This page* / *All pages*, same as Extract) · Fit page / Actual size · *Rotate pages to fit* · unsaved-marks banner · problem line · **Print** / Cancel (Esc cancels).
- **Right column:** a paper rectangle (physical size, aspect-correct), a dashed printable-area rectangle, the page image at `place().dest`, a "Sheet 2 of 5" ◀ ▶ stepper over the resolved page list, and a red "Clipped — larger than the printable area" note when `clipped`.
- Preview image: `session.render_page_rotated(page, scale, placement.rotation)` into the modal's **own** texture, about 1.5× the pane's pixel size, re-rendered only when the key changes. It does **not** use `thumb_for`'s `doc.caches.thumbs` (would evict the rail's thumbnails). Synchronous: measured 15–30 ms normally, ~270 ms worst case (60k-shape drawing). `ponytail:` move to `RenderWorker` only if it hitches.
- Print: resolve pages → `print_windows::print`. Success: close, `say_info("sent N pages to <printer>")`. Failure: stay open, show it in `problem`, also `say_error`.
- Like today, a job blocks the UI thread while it renders and spools (unchanged; progress is out of scope).
- Unsaved ink: `PagifyApp::unsaved()` already returns `(marks, pages)`; the banner uses it. Nothing is committed or discarded.

## 5. Out of scope (deliberate)

Paper-size dropdown (driver Properties covers it) · N-up / booklet · colour mode · print-to-file · composited unsaved ink (phase 2) · progress bar / async print · macOS / Linux · per-page DEVMODE (mixed shapes are handled by rotating the raster, not the sheet) · honouring on-screen page rotation.

## 6. Risks

1. `DocumentPropertiesW` needs an open printer handle and a real owner `HWND`; both come from the same `hub::active_window()` path the old dialog used. Close the handle on every exit.
2. Network/virtual printers (the office Canon iR-ADV C5535) can be slow to answer `CreateDCW`; hence names-only enumeration and re-querying paper only on change.
3. A wrong `dmFields` bit makes the driver ignore a setting silently — the e2e test checks orientation and copies actually took.
4. Centring changes where pages land on paper for anyone used to top-left. Intentional, agreed above.

## 7. Testing

- **Unit (`pagify_shell`, runs everywhere):** landscape page on portrait paper → `Clockwise90` and a larger scale; portrait page on landscape paper likewise; Fit centres on both axes; ActualSize centred when it fits, top-left + `clipped` when it overflows; `rotate_to_fit` off never rotates; square page tie → `None`; zero/degenerate paper does not panic; A3/A0 at 300/600/1200 dpi stay inside the engine ceiling (the existing `a_big_sheet_…` test moves with `render_scale`). **Each test is run once against a deliberately broken `place()` to prove it can fail.**
- **kittest (`pagify_app`):** build the modal with a hand-set `Paper` (no printer needed), assert the preview image rect equals `place().dest` mapped into the paper rect; stepper moves through the resolved pages; banner appears iff `unsaved()`; Esc closes without printing.
- **End to end, no paper:** print 3 mixed-orientation pages to "Microsoft Print to PDF" via the `output` path; reopen the result with `pdf_core` and check page count, each sheet's orientation, and that the ink bounding box matches `place().dest` — an independent measurement, not the layout function checking itself. Run headless with an isolated `APPDATA` so nothing appears on the owner's screen; never invoke the real iR-ADV.
- **Regression:** full `pagify_shell` + `pagify_app` suites once before publish (known env-sensitive `hub_tests` and wall-clock tests excluded per ARCHITECTURE §6).

## 8. Rollout

Bump `[workspace.package].version` and `MyAppVersion` in `packaging\windows\pagify.iss` together to **0.1.64** (Dropbox `version.txt` is 0.1.63). Confirm `git branch --show-current` and `git status --short`, make sure `Pagify.exe` is not running, then `powershell -NoProfile -ExecutionPolicy Bypass -File .\build.ps1 -Publish` from `packaging\windows\`. Update the `print_windows.rs` line in `Pagify-Architecture.md` §2a afterwards.
