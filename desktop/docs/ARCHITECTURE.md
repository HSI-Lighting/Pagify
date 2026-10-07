# Pagify Desktop — Architecture

Status as of 2026-10-07, build **0.1.46**. Branch `farzad-debug`, continuing the work on `pagify-windows-refactor` (which the design review's phases 0–3 built); public repo `HSI-Lighting/Pagify`.

Pagify is a PDF editor written in Rust. The window is built with egui/eframe 0.36; PDF reading and writing goes through PDFium.

## 1. The layers

```
 pagify_app   (desktop/crates/pagify_app)   the window: egui UI, tools, panels, tests of the UI
      │  calls
 pagify_shell (desktop/crates/pagify_shell) the app's logic with no UI: sessions, verbs, markup, guides, organize, reader
      │  calls
 pdf_core     (rust/pdf_core)               the engine: documents, commands + undo, text edits, render, OCR, crypto
      │  calls
 pdfium-render (rust/vendor/pdfium-render)   vendored Rust binding over PDFium (pdfium.dll)
```

`cad_kernel` (workspace dependency) is the 2D geometry for the markup layer (lines, circles, polylines, hatch). `pagify_issue` is the third workspace member (issue reporting).

| Layer | Size | What it owns |
|---|---|---|
| `pagify_app` | `main.rs` ≈ 15,500 lines, plus feature modules `canvas.rs`, `panels.rs`, `edit.rs`, `dispatch.rs`, `ribbon.rs`, `workspace.rs`, `view.rs`, `picking.rs`, `pending.rs`, `caches.rs`, `hub.rs`, `overlay.rs`, `home.rs`, `text_style_panel.rs`, `theme.rs`, `system_fonts.rs`, `focus.rs`, `instance.rs`, `print_windows.rs`, and ~30 test files beside them | Everything drawn, every pointer/keyboard gesture, per-tab state, the ribbon, the properties panel |
| `pagify_shell` | ~32 modules | `session.rs` (one open document), `verbs.rs` (the typed-command language), `markup.rs`, `guides.rs` (alignment lines), `organize.rs` (page reorder maths), `reader.rs` (view anchor, characters), `blocks.rs` + `block_input.rs` (paragraph detection), `paragraph_lines.rs` and `editor.rs` (editor/paragraph arithmetic, moved out of the app in Phase 4a), `spelling.rs`, `diagnose.rs` (why a file will not open), `session_log.rs` |
| `pdf_core` | `document/pdfium_doc.rs` ≈ 17,000 lines | The `Document` / `DocumentMut` traits, `Command` + undo, byte-safe content-stream edits, text shaping/embedding, rendering, redaction, signing |

The main file was large by accident, not design, and is being split by the design review (`desktop/docs/DESIGN_REVIEW.md`): it went from ≈ 46,000 lines (of which 22,900 were tests squeezed into three `#[cfg(test)]` modules) to ≈ 15,500 by Phase 1's module extraction, with every test moved beside the code it tests. Phase 1's function split followed: `ui()` (960 lines) is now a 13-line frame outline over seven named methods (`draw_frame_preamble`, `handle_input`, `open_dropped_files`, `draw_title_bar`, `draw_ribbon`, `draw_command_bar`, `draw_main_area`), and `act()` (820 lines) is a router over nine per-domain handlers in `dispatch.rs` (`act_document` … `act_measurement`). `verbs::parse` (406 lines) is now the same shape: a thin function that checks the planned-verb table and then tries seven per-domain parsers in turn (`parse_locking_and_redaction`, `parse_fill_and_sign`, `parse_signatures`, `parse_security_options`, `parse_document_edits`, `parse_file_and_view`, `parse_marks_and_misc`), each matching only its own commands and returning `None` otherwise. `picking::resolve` (233 lines) is now a thin dispatcher over six per-domain helpers (`resolve_point_pick`, `resolve_measure`, `resolve_form_mark`, `resolve_draw`, `resolve_modify`, `resolve_placement`), and the compile-time guarantee the parse split gave up — a command word registered in two domains — is tested again by `parse_domain_tests::no_command_is_claimed_by_two_domain_parsers`, which reads the parsers' own source. The remaining structural work is named there: the frame function's own `draw_main_area` (340 lines) is now four methods — `draw_left_rail`, `draw_layers_window`, `draw_page_canvas` and the deferred-command tail — and `act_security`'s Smart Redact pass moved out to `act_smart_redact`. `canvas::interact`'s right-click menu — the 231-line popup with its copy/layer/stacking items — moved out to `show_page_context_menu`. `draw_pages`'s tail — noting when the zoom last changed, collecting renders, prefetching neighbours, evicting rasters and recording where the strip ended up — moved out to `note_zoom_and_collect_renders` and `settle_strip_after_scroll`. `Tab::buttons` (241 lines of table) is now a fifteen-arm match over per-tab associated constants (`FILE_BUTTONS` … `AUTOMATE_BUTTONS`), and `draw_passcode_dialog`'s three per-variant wording matches moved out to the pure `passcode_wording`, and `draw_run_editor`'s last-line wrap loop, skin values, max-width maths and text-width measurement moved out to `wrap_typed_last_line`, `run_editor_skin`, `run_editor_max_width` and `run_editor_text_width`. Measured by clippy's non-comment-line count on 2026-10-07, the largest functions left are `interact` (257), `draw_pages` (220), `pick_text_run_traced` (205), `draw_signature_list` (184), `draw_command_bar` (181) and `draw_spell_check` (170) — down from `ui` (959) and `act` (818), with 49 functions still over 100 lines against the review's 150-line target. The tool transitions in `picking.rs` are still a dispatch match rather than event-returning `Tool::on_click`/`on_key` transitions, and Phase 4b (an `Effect`-returning command executor) has not started.

## 2. How an edit travels (the important path)

1. **Pick.** A click in Edit Text resolves against a cached `PageBlocks` reading of the page (`page_blocks`, stamped with page + render epoch + undo generation). The result is an `EditingRun`: the object numbers, the original text, per-line rectangles, `frozen` flags (lines the page draws as shapes), `twins` (faux-bold duplicates), the style as opened (`was`) and as edited (`style`), and `box_resize` (§5).
2. **Edit.** `draw_run_editor` draws an egui `TextEdit` over the words, in the document's own face where it can. Typing never touches the document.
3. **Apply.** `apply_editing_page` refuses a stale editor (the page changed under it), then `apply_one_edit` decides: empty buffer → delete; drawn (outlined) words → `replace_outlined_word`; several lines/objects or frozen lines → the paragraph path; otherwise `SetTextRun` for a single run.
4. **Paragraph path.** `plan_paragraph_edit` (pure, in `paragraph_lines.rs`) matches typed lines to original lines by content, not position, and gives each a fate: `Kept`, `Written`, `Removed`, `Frozen`. `line_edits` turns that into one `Command::ReplaceTextLines`.
5. **Engine.** Two routes, chosen by what the edit asks for:
   - **Byte-safe**: swap the characters inside the page's content stream (`transform_in_stream`, `object_wrap_site`, `set_run_in_stream`). Nothing else on the page is touched. This is the preferred route.
   - **PDFium object model**: used for colour, size and position changes, and for rotating text. It ends with `FPDFPage_GenerateContent`, which rewrites the whole page, so it runs behind a guard that compares the rest of the page before and after, and restores a snapshot if anything else moved.
6. **Undo.** Every `Command` has an `UndoRecord`. Either an inverse operation (a move is undone by the opposite move, a rotation by the opposite angle) or a page snapshot restore (used by anything that renumbers objects). A `Batch` undoes as one step.

Page objects are addressed by their **index in the page's object list**. Anything that adds or removes objects renumbers them, which is why open editors carry a `(render epoch, undo generation)` stamp and are closed when it moves.

## 3. State

- `PagifyApp` — app-wide: tabs, theme, shared clipboards (`object_clipboard`, `page_clipboard`, `paste_ghost`), session log, font registries, update checker.
- `DocTab` — per document: the `Doc` (session, thumbnails, textures, page strip), the selection mechanisms (`selected`, `group`, markup `Layer` selection, signature/placed-image selections), the open editors (`editing_run`, `new_text_box`), view state and `ViewSnapshot` (so the reader keeps its place).
- **One armed tool.** `DocTab.tool: Option<ArmedTool>` is the only waiting-for-clicks state: an `ArmedTool` is a `Tool` variant plus the page and the points/objects collected so far. All the kinds — draw, modify, measure, text, sign, lock, redact, whiteout, fill, calibrate — live in the one enum (`pending.rs`), with queries (`wants`, `prompt`, `repeats`, `command`, `wants_snapping`) on `Tool`; arming and resolving live in `picking.rs`. The old split between `PendingKind`/`Pending` and a second `ArmedTool` is gone (design review Phase 2).
- **One cache registry.** `Doc.caches: DocCaches` (`caches.rs`) holds every derived-from-the-document value — textures, thumbnails, detail tile, locked items, page blocks, sampling raster, page weight, rect page, characters, layers, foreign marks, internal links, drawn words — and `rendered_is_stale()` bumps the render epoch and empties them all through one method, so no edit path has to remember a list (design review Phase 3).
- The **selection mechanisms are mutually exclusive** by convention; most "delete / copy" code checks them in a fixed order.
- Documents are rendered off the UI thread by a `RenderWorker`; the page draws from the last picture it holds until the right one arrives.

## 4. Input and focus

- `Focus::capture` runs once per frame. Typing keys reach the document only when nothing has focus (`allows_document_keys`); copy/paste is allowed with the command box focused (`allows_clipboard_keys`).
- ⌘C / ⌘V arrive as `Event::Copy` / `Event::Paste(text)`, never as key presses. After a non-text copy the app writes a placeholder (`COPIED_IN_PAGIFY`) to the system clipboard so the next paste event fires.
- The command box (`pagify_shell::verbs`) is a second, complete way to drive everything; the ribbon buttons mostly prefill or run verbs.

## 5. Recent features and where they live

| Feature | Where |
|---|---|
| Reference lines while moving (grey/green, 6 px snap, Alt = no snap) | `pagify_shell::guides::align`; app `guide_targets`, `snap_the_move`, `draw_move_guides` |
| Rotate handle with angle label (Shift = 15° steps) | app `draw_rotate_icon`, `object_turn`, `rotate_thing`; engine `Command::RotateObject`, `Document::rotate_object` |
| Typed-away text deletes it | `apply_one_edit` empty branch; `edit_has_changes` |
| Paste ghost (50 % opacity, click to place) | `start_paste_ghost`, `draw_paste_ghost`, `place_paste_ghost`; `ObjectClipboard::{Shapes, Image, Text}` |
| Text-box resize handles | `RunBox` on `EditingRun.box_resize`; grips in `draw_run_editor`; eight handles in `draw_new_text_box` |
| Text Style panel | `text_style_panel.rs` (UI only: `Look`, `Gates`, `Changes`, `show`) wired by `draw_text_style` |
| Xref repair on open | `pdfium_doc.rs`: `mended_if_damaged`, `repair_misplaced_xref_type` |
| One-row tab strip, multi-window, single instance | `tabs_that_fit`, `hub.rs`, `instance.rs` |

**Resize handles.** A run's box is a *width to wrap to*. A resize joins the box's lines into one and the existing wrap loop re-wraps it; Apply writes the first line over the run and the rest as new lines below. The left handle also moves the run: `left_shift_pt` is folded into `style.at` from `was.at` when applying.

**Paste.** Words paste as new editable text (all lines in one `Batch`, so one undo). A picture or shape picked from the *page itself* is copied as a raster of its box, because the engine has no "duplicate this object" command. Shapes drawn in Pagify's markup layer and pictures placed by Pagify paste as themselves.

**Text Style panel.** Font, size, colour, Bold/Italic (the same family's real Bold/Italic face, found by name over installed and bundled fonts), and left/centre/right for a new box work. The rest is drawn greyed with a reason on hover.

## 6. Build, test, ship

- Build: `cargo build --release --locked` from `D:\pagify desktop\desktop`. The exe loads `pdfium.dll` and `ocr\` models from beside it (`target\Pagify\`).
- Tests: `cargo test -p pagify_app --release --bin pagify_app` (≈ 950 tests, ~40 s once built; needs `PAGIFY_PDFIUM_LIB` pointing at `third_party\pdfium\pdfium-win-x64\bin\pdfium.dll`, otherwise engine tests silently skip). UI tests use `egui_kittest` (`harness`, `click`, `drag`, `run_steps`).
- Ship: `desktop\packaging\windows\build.ps1 -Publish` — gates (verify third-party checksums, `cargo audit`, no-sockets check), builds, copies to `target\Pagify`, then publishes loose files + `Pagify.zip` + `version.txt` to `D:\Dropbox\YASEEN\pagify desktop`. The version lives in `desktop/Cargo.toml`, `pagify.iss` and `Cargo.lock`.
- Run `build.ps1` through a child `powershell -File`, never with PowerShell `*>` redirection (the audit step prints to stderr and PowerShell turns that into a fatal error).
- If the app is running from `target\Pagify`, move its `Pagify.exe` and `pdfium.dll` to `target\Pagify-<ver>-running\` first (a rename is allowed on a running exe). Never kill the user's window.
- Always confirm the Dropbox `Pagify.exe` hash equals `target\Pagify\Pagify.exe` and that `version.txt` shows the new number.

## 7. Hazards worth knowing

- `FPDFPage_GenerateContent` rewrites an entire page. Used carelessly it corrupts files from Illustrator/InDesign (fonts merged, spacing and colour operators lost). See *Pagify-Remaining-Work.md*.
- Object numbers change when objects are added or removed. Never hold one across an edit.
- Parallel tests must each use their own fixture file name; a shared temp path made tests race.
- Never write source files with PowerShell (encoding damage); use the editor tools or `sed`/`perl`.
- Tests must not open consoles or windows on the owner's screen (`CREATE_NO_WINDOW`, stand-in exes, isolated `APPDATA`).
