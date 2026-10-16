# Text Style — phases 0 and 1 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Phase 0 — the Italic letter is visible and the font field always names a real font. Phase 1 — a new text box takes every character style (font, size, colour, B, I, U, S, superscript, subscript, letter spacing, character width, baseline shift) on the selected characters, previewed and written from one model and one layout.

**Architecture:** A pure rich-text model (`pagify_shell::rich_text`) holds the box's text and its style runs; a pure layout (`pagify_shell::text_layout`) places each run of each row in points from the real font's advances. The box's `TextEdit` draws a `LayoutJob` built from the model, and Add to Page writes the same rows as **one** `Annotation::Text` — its first run in the existing fields, the rest in a new `more` list, underlines and strike-throughs in a new `rules` list — so one undo takes the whole box.

**Tech Stack:** Rust; egui/eframe 0.36.1 and `egui_kittest`; `pdf_core` over PDFium; rustybuzz / ttf-parser for shaping and font tables.

**Spec:** `desktop/docs/TEXT_STYLE_DESIGN.md` (agreed 2026-10-15; §4c, §4d and §3 refined with this plan — see Task 10).

## Global Constraints

- Branch `pagify-windows-refactor`. Before any build or publish: `git branch --show-current` and `git status --short` (the folder is shared with another branch — Architecture §7).
- Superscript and subscript: **58%** of the run's size, raised **0.33 em** / lowered **0.14 em** of the run's own size; exclusive; line spacing unchanged.
- Letter spacing: points added after every glyph but a row's last; menu −1, −0.5, 0, 0.5, 1, 2, 5.
- Character width: percent; menu 75, 90, 100, 110, 125, 150.
- Baseline shift: points, + up, − down, added to any super/sub rise; menu −3, −2, −1, 0, 1, 2, 3.
- Underline and strikethrough: page content (tagged lines), never annotations; position and thickness from the font's `post` / `OS/2` tables, falling back to −0.1 em / 0.05 em (underline) and 0.3 em / 0.05 em (strike).
- A character style changes the selection, or every character when nothing is selected; typed text takes the look of the character before the cursor (the first character's, at the very start).
- A look that varies inside the selection is *mixed*: a toggle neither lit nor dark, a number field blank; pressing a mixed toggle turns it on for all.
- **No "(automatic)" anywhere** — the font field and the font picker always name a font (owner, 2026-10-15). A new box starts in **Arial** when it is installed, else the bundled **Montserrat-Regular**.
- Text indices are **characters** (egui's `CCursor`), never bytes.
- House rules: tests first; each new behaviour mutation-checked (take it out, watch its test fail) before it counts as done; suites run with `PAGIFY_PDFIUM_LIB` and `PAGIFY_QPDF` set and `--no-fail-fast`, load-sensitive tests rerun alone (Architecture §6); tests never touch the real clipboard and never open a window or a terminal.
- Each phase ships: bump `[workspace.package].version` in `desktop/Cargo.toml` and `MyAppVersion` in `desktop/packaging/windows/pagify.iss` past `version.txt` in `D:\Dropbox\YASEEN\pagify desktop`, Pagify.exe not running, `powershell -NoProfile -ExecutionPolicy Bypass -File .\build.ps1 -Publish` from `desktop\packaging\windows`, and a dated § in `C:\Users\hsili\Desktop\APP ARCHITECTURE\Pagify-Architecture.md`. Commit and push only on the owner's word.

## Review Focus

1. **A face drawn on the frame it is installed panics egui** ("a family nothing is bound to"). Expected: a newly chosen face draws in the default face for one frame, then in itself; and installing a new box's faces never unbinds the Edit Text box's face. → Task 8, `a_face_is_used_from_the_frame_after_it_is_installed` and `a_new_boxs_faces_do_not_unbind_the_edit_text_face`.
2. **Clicking the panel takes the focus from the box.** Expected: the panel still acts on the selection the box had. → Task 8, `the_panel_acts_on_the_selection_the_box_had_before_the_click`.
3. **Typing at a run's edge** (right after an underlined word, or before the first character). Expected: Word's rule — the look before the cursor, the first character's at the start. → Task 4, `typing_after_a_word_takes_its_look`, `typing_at_the_start_takes_the_first_characters_look`.
4. **A span whose face cannot draw one of its letters** (a subset, a face without CJK). Expected: written in a face that can, and said — never empty boxes. → Task 9, `a_letter_the_face_cannot_draw_is_written_in_one_that_can_and_said`.
5. **Undo after Add to Page.** Expected: one undo takes every word and line of the box off, redo puts it all back. → Task 9, `one_undo_takes_the_whole_box_off_and_redo_puts_it_back`.

---

## Phase 0

### Task 1: The Italic letter is drawn

**Files:**
- Modify: `desktop/crates/pagify_app/src/text_style_panel.rs` (`draw_letter`; delete `shape_to_sheared`)
- Test: the same file's `#[cfg(test)] mod tests`

**Interfaces:** none new.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn the_italic_letter_is_drawn_from_the_font_atlas() {
    use egui_kittest::kittest::Queryable;
    let look = look();
    let mut h = egui_kittest::Harness::new_ui(|ui| {
        show(ui, egui::Id::new("style"), &look, &Gates::default());
    });
    h.run();
    let button = h.get_by_label("Italic").rect();
    let primitives = h.ctx.tessellate(h.output().shapes.clone(), h.ctx.pixels_per_point());
    let uvs: Vec<egui::Pos2> = primitives
        .iter()
        .filter_map(|p| match &p.primitive { egui::epaint::Primitive::Mesh(m) => Some(m), _ => None })
        .flat_map(|m| m.vertices.iter())
        .filter(|v| button.contains(v.pos))
        .map(|v| v.uv)
        .collect();
    assert!(!uvs.is_empty(), "nothing is drawn on the Italic button");
    assert!(
        uvs.iter().all(|uv| (0.0..=1.0).contains(&uv.x) && (0.0..=1.0).contains(&uv.y)),
        "the letter samples outside the font atlas: {:?}",
        &uvs[..uvs.len().min(4)]
    );
}
```

- [ ] **Step 2: Run it — expect FAIL** ("samples outside the font atlas"): `cargo test --release -p pagify_app the_italic_letter_is_drawn`
- [ ] **Step 3: In `draw_letter`, lay the italic letter out as a `LayoutJob` with `TextFormat { font_id, color, italics: true, ..Default::default() }` and draw it with `painter.galley`; delete `shape_to_sheared`; reword the comment ("egui leans glyphs itself: `TextFormat::italics`").**
- [ ] **Step 4: Run it — expect PASS.** Mutation check: put the old mesh path back, the test fails.

### Task 2: The font field always names a font

**Files:**
- Modify: `desktop/crates/pagify_app/src/main.rs` — `begin_text_box`, `choose_face` (extract `register_system_face`), `draw_text_style` (label), `draw_font_picker` (first entry), the drawn-word pick that sets `current_face: None` (`main.rs` ~12651)
- Modify: `desktop/crates/pagify_app/src/ui_tests.rs` — `right_aligned_text_lands_near_the_boxs_right_edge` (see Step 5)
- Create: `desktop/crates/pagify_app/src/text_style_tests.rs` (`#[cfg(test)] mod text_style_tests;` in `main.rs` beside `object_hover_tests`)

**Interfaces:**
- Produces: `fn default_box_face(&mut self) -> String`; `fn register_system_face(&mut self, name: &str) -> bool` (the reading and registering `choose_face` does today, for both `pdf_core::text` and the session's typing fonts).

- [ ] **Step 1: Write the failing tests** in `text_style_tests.rs`:
  - `a_new_box_starts_in_a_named_font` — after `begin_text_box` on `two-column.pdf`, `new_text_box.face == Some(expected)` where `expected` is `"Arial"` when `system_fonts::list()` has a font named exactly `Arial`, else `"Montserrat-Regular"`; and `h.get_by_label(&format!("Font {expected}"))` exists.
  - `a_new_box_without_arial_starts_in_the_bundled_face` — set `faces_state.system_fonts = Some(Vec::new())` first; the face is `Some("Montserrat-Regular")`.
  - `nothing_in_the_panel_or_the_picker_says_automatic` — with a new box open and the picker opened by clicking the font button, then with an Edit Text editor open on `text-lines.pdf` and its picker opened: no widget's label contains `"(automatic)"`; the Edit Text picker's first entry reads `"<the run's font> (the page's own font)"`.
  - `a_drawn_word_names_the_face_its_replacement_is_written_in` — a pick on a drawn word of `outlined-montserrat.pdf` gives a font button labelled `Font Montserrat-…`, the first of `writing_faces()` that covers the word.
- [ ] **Step 2: Run them — expect FAIL** (`face` is `None`; labels say `(automatic)`).
- [ ] **Step 3: Implement.**
  - `default_box_face`: load `system_fonts` if `None`; `Arial` via `register_system_face` when present, else the first of `writing_faces()` (`Montserrat-Regular`). `begin_text_box` sets `face: Some(self.default_box_face())`.
  - Label in `draw_text_style`: a new box shows its face; Edit Text shows `style.face`, else `current_face`, else `"The page's own font"` — only reached on a page too heavy to read (`page_is_heavy`), where reading the name is a further open of the page that a click there is built to avoid; its hover says so.
  - The drawn-word pick sets `current_face` to the first `writing_faces()` entry that `pdf_core::text::covers` the recognised word, else `"Helvetica"` (what `replace_outlined_word` writes in).
  - The picker: no entry for a new box; for Edit Text, `"{current_face} (the page's own font)"` (or `"The page's own font"`), which still clears `style.face` as `(automatic)` did.
- [ ] **Step 4: Run them — expect PASS.** Mutation check each: `face: None` back in `begin_text_box`; the old label; the old picker entry.
- [ ] **Step 5: Fix what the named face changes.** A named face is written one text object per glyph, so `right_aligned_text_lands_near_the_boxs_right_edge` can no longer find a run reading `"Hi"`: assert instead that the box's first character on the page (`session.characters(0)`, the `H`) starts past the box's middle. Run `cargo test --release -p pagify_app` and repair any other test that assumed a new box writes base-14 Helvetica, keeping what it was checking.

### Task 3: Ship phase 0 (0.1.88)

- [ ] **Step 1:** Full suites per Global Constraints (`pagify_app`, `pagify_shell`; `pdf_core` is untouched). Expected: all pass, the load-sensitive ones alone.
- [ ] **Step 2:** Version 0.1.88 in both files; publish; check `version.txt` reads `0.1.88`.
- [ ] **Step 3:** Architecture §27 (Italic letter: cause and fix; no "(automatic)"; a new box writes in Arial / Montserrat-Regular, embedded, one object per glyph) and the §5 lines it settles.

---

## Phase 1

### Task 4: The rich-text model

**Files:**
- Create: `desktop/crates/pagify_shell/src/rich_text.rs` (`pub mod rich_text;` in `lib.rs`), tests in its own `#[cfg(test)] mod tests`

**Interfaces:**
- Produces:
  - `pub enum Script { #[default] Normal, Super, Sub }` (Debug, Clone, Copy, PartialEq, Eq, Default)
  - `pub struct CharLook { pub face: String, pub size: f32, pub color: [u8; 4], pub underline: bool, pub strike: bool, pub script: Script, pub letter_spacing_pt: f32, pub width_pct: f32, pub baseline_pt: f32 }` (Debug, Clone, PartialEq) and `CharLook::plain(face: impl Into<String>, size: f32, color: [u8; 4]) -> CharLook` (no decoration, `Normal`, 0, 100, 0)
  - `pub struct CharChange { pub face: Option<String>, pub size: Option<f32>, pub color: Option<[u8; 4]>, pub underline: Option<bool>, pub strike: Option<bool>, pub script: Option<Script>, pub letter_spacing_pt: Option<f32>, pub width_pct: Option<f32>, pub baseline_pt: Option<f32> }` (Default)
  - `pub enum Mixed<T> { One(T), Mixed }` and `pub struct LookOf { pub face: Mixed<String>, pub size: Mixed<f32>, pub color: Mixed<[u8; 4]>, pub underline: Mixed<bool>, pub strike: Mixed<bool>, pub script: Mixed<Script>, pub letter_spacing_pt: Mixed<f32>, pub width_pct: Mixed<f32>, pub baseline_pt: Mixed<f32> }`
  - `pub struct RichText` (Debug, Clone, PartialEq) with `new(look: CharLook) -> Self`, `text(&self) -> &str`, `char_count(&self) -> usize`, `runs(&self) -> &[(usize, CharLook)]` (each run's end, in characters, ascending, the last equal to `char_count`; one `(0, look)` for an empty text — the typing look), `spans(&self, range: Range<usize>) -> Vec<(Range<usize>, &CharLook)>`, `look_at(&self, index: usize) -> &CharLook` (what typing at `index` takes), `edit(&mut self, new_text: &str)`, `set_char(&mut self, range: Range<usize>, change: &CharChange)` (an empty range means every character, and the typing look), `look_of(&self, range: Range<usize>) -> LookOf` (an empty range means every character).

- [ ] **Step 1: Write the failing tests** — the look under test is told apart by `underline` and `size`:
  - `typing_after_a_word_takes_its_look` — `"ab"`, `set_char(1..2, underline)`, `edit("abc")` → characters 1..3 underlined, 0..1 not.
  - `typing_at_the_start_takes_the_first_characters_look` — `"ab"` underlined whole, `edit("xab")` → all three underlined.
  - `deleting_across_runs_keeps_the_rest` — `"aaBBcc"` with `BB` at size 20, `edit("aBcc")` → runs `[(1, 14), (2, 20), (4, 14)]` by size.
  - `a_paste_over_a_range_takes_the_look_before_it` — `"aaBBcc"` (`BB` 20), replace `"BB"` by `"xyz"` → `xyz` at size 14 (the `a` before it).
  - `setting_a_look_and_taking_it_off_again_leaves_one_run` — `set_char(2..4, underline on)` then `set_char(2..4, underline off)` → `runs().len() == 1`.
  - `an_empty_range_changes_every_character_and_the_typing_look` — on `""`, `set_char(0..0, size 20)`, `edit("x")` → `x` at 20.
  - `look_of_a_range_says_mixed_where_it_varies` — `"ab"` with `b` underlined: `look_of(0..2).underline == Mixed::Mixed`, `look_of(0..2).size == Mixed::One(14.0)`.
  - `indices_are_characters_not_bytes` — `"é日b"`, `set_char(1..2, size 20)` → `spans(0..3)` has the 20 on `日` alone.
- [ ] **Step 2: Run** `cargo test --release -p pagify_shell rich_text` — expect FAIL (no module).
- [ ] **Step 3: Implement.** `edit`: the change is the stretch between the common prefix and the common suffix (in characters; the suffix may not overlap the prefix); the removed characters leave their runs, the inserted ones take `look_at(prefix)`; ends after the change shift by the difference; neighbours that look the same merge; zero-length runs go, except the one an empty text keeps.
- [ ] **Step 4: Run — expect PASS.** Mutation checks: inserted text takes the look *after* the cursor; no merging.

### Task 5: The layout, and the font's line metrics

**Files:**
- Modify: `rust/pdf_core/src/text/mod.rs` — `LineMetrics`, `line_metrics`, a unit test
- Create: `desktop/crates/pagify_shell/src/text_layout.rs` (`pub mod text_layout;`), tests inside

**Interfaces:**
- Consumes: Task 4's `RichText`, `CharLook`, `Script`.
- Produces:
  - `pdf_core::text::LineMetrics { pub ascent: f32, pub descent: f32, pub underline_position: f32, pub underline_thickness: f32, pub strikeout_position: f32, pub strikeout_thickness: f32 }` in em, positions above the baseline positive (an underline's is negative); `pub fn line_metrics(name: &str) -> Result<LineMetrics>` from the registered program's `head`/`hhea`/`post`/`OS/2` through ttf-parser, with the Global Constraints' fallbacks where a table has nothing.
  - `pub trait Measure { fn advances_em(&self, face: &str, text: &str) -> Vec<f32>; fn lines(&self, face: &str) -> pdf_core::text::LineMetrics; }` — one advance per glyph, as shaped.
  - `pub struct FaceMeasure;` implementing `Measure` with `pdf_core::text::shape` and `line_metrics` (fallbacks on error).
  - `pub struct RowIn { pub chars: Range<usize>, pub baseline_pt: f32 }`; `pub enum RowAlign { Left, Center, Right }`.
  - `pub struct PlacedSpan { pub chars: Range<usize>, pub text: String, pub look: CharLook, pub x_pt: f32, pub baseline_pt: f32, pub size_pt: f32, pub advances_pt: Vec<f32>, pub scale_x: f32 }` (`baseline_pt` includes the rise, y downwards; `size_pt` includes the script scale; one advance per glyph, letter spacing included).
  - `pub struct LineRule { pub x0_pt: f32, pub x1_pt: f32, pub y_pt: f32, pub thickness_pt: f32, pub color: [u8; 4] }`; `pub struct PlacedRow { pub spans: Vec<PlacedSpan>, pub rules: Vec<LineRule> }`.
  - `pub fn place_rows(rich: &RichText, rows: &[RowIn], left_pt: f32, width_pt: f32, align: RowAlign, measure: &dyn Measure) -> Vec<PlacedRow>`; `pub const SCRIPT_SCALE: f32 = 0.58; SUPER_RISE_EM: f32 = 0.33; SUB_DROP_EM: f32 = 0.14;`

- [ ] **Step 1: Write the failing tests.** In `pdf_core::text` tests: `montserrat_has_an_underline_below_and_a_strikeout_above` (register `desktop/third_party/fonts/Montserrat-Regular.ttf` via `CARGO_MANIFEST_DIR`, skip if absent) — `underline_position < 0.0`, `strikeout_position > 0.0`, both thicknesses in `(0.0, 0.2)`. In `text_layout` tests, against a `FixedMeasure` (every glyph 0.5 em; ascent 0.8, descent 0.2, underline −0.1 / 0.05, strike 0.3 / 0.05), one row `0..len` at baseline 100, box left 10, width 200:
  - `spans_follow_one_another_along_the_row` — `"ab"` at 10 pt then `"cd"` at 20 pt: second span's `x_pt == 10 + 2 × 0.5 × 10`.
  - `centre_and_right_alignment_place_the_row_in_the_box` — `"abcd"` at 10 pt (20 pt wide): `Center` → `x_pt == 10 + 90`, `Right` → `10 + 180`.
  - `superscript_is_smaller_and_raised_and_subscript_lowered` — at 12 pt: `size_pt == 12 × 0.58`; `baseline_pt == 100 − 0.33 × 12` (super), `100 + 0.14 × 12` (sub).
  - `letter_spacing_goes_after_every_glyph_but_the_rows_last` — `"abc"` 10 pt, 2 pt spacing: advances `[7, 7, 5]`.
  - `character_width_scales_the_advances` — 50%: advances `2.5` each, `scale_x == 0.5`.
  - `baseline_shift_moves_the_span_up` — +2 pt → `baseline_pt == 98`.
  - `an_underline_is_one_rule_per_stretch_at_the_fonts_position` — `"ab"` 10 pt + `"cd"` 20 pt, both underlined, one colour: one rule, `x0 == 10`, `x1 == 10 + 5 + 10`, `y == 100 + 0.1 × 20`, `thickness == 0.05 × 20`; with two colours, two rules.
  - `a_strikethrough_sits_at_the_fonts_strikeout_height` — `y == 100 − 0.3 × size`.
  - `trailing_spaces_are_not_measured_or_underlined` — `"ab  "` right-aligned: placed as if `"ab"`; the rule ends at `b`.
- [ ] **Step 2: Run** `cargo test --release -p pdf_core line_metrics` and `-p pagify_shell text_layout` — expect FAIL.
- [ ] **Step 3: Implement `line_metrics`, `FaceMeasure` and `place_rows`.** One rule per maximal stretch of adjacent underlined (or struck) spans of one colour, at the metrics of the stretch's largest `size_pt` span and the row's baseline (a raised span does not raise the rule).
- [ ] **Step 4: Run — expect PASS.** Mutation checks: the rise sign; spacing after the last glyph; one rule per span.

### Task 6: One text mark of several parts, with lines and a horizontal scale

**Files:**
- Modify: `rust/pdf_core/src/document/mod.rs` — `TextPart`, `TextRule`, three fields on `Annotation::Text`, `Annotation::bounds`
- Modify: `rust/pdf_core/src/document/pdfium_doc.rs` — `write_text`
- Modify every `Annotation::Text { … }` literal — `pagify_app`: `main.rs` (two), `edit.rs`, `tests.rs`, `lock_wiring_tests.rs` (two); `pagify_shell`: `commit.rs`; `pdf_core`: `pdfium_doc.rs` (two), `command/history.rs`, `examples/stack_probe.rs`, `examples/text_lifecycle_probe.rs`, `tests/move_many.rs`, `tests/round_trip.rs` (three) — adding `scale_x: 1.0, more: Vec::new(), rules: Vec::new()`
- Create: `rust/pdf_core/tests/text_parts.rs`

**Interfaces:**
- Produces: `pub struct TextPart { pub font: String, pub font_asset: Option<String>, pub size: f32, pub color: Color, pub glyphs: Vec<Glyph>, pub scale_x: f32 }` and `pub struct TextRule { pub from: Point, pub to: Point, pub width: f32, pub color: Color }` (both Debug, Clone, PartialEq, Serialize, Deserialize, camelCase); on `Annotation::Text`: `#[serde(default = "unit_scale")] scale_x: f32`, `#[serde(default)] more: Vec<TextPart>`, `#[serde(default)] rules: Vec<TextRule>`.

- [ ] **Step 1: Write the failing tests** in `text_parts.rs` (`harness::skip_without_pdfium`, `harness::serial`, `open_fixture(&pdfium, "single-page.pdf")`, `CommandHistory`):
  - `a_mark_of_several_parts_writes_each_part_in_its_own_look` — one `Annotation::Text`: `"Big"` Helvetica 24 red, `more: [TextPart "small" Helvetica 10 blue]`, `rules: [TextRule]`; after `execute`, `text_runs(0)` has a run reading `Big` at size ≈ 24 in red and one reading `small` at ≈ 10 in blue; `harness::check_both_saves` passes.
  - `a_rule_is_drawn_as_a_line_where_it_was_put` — `drawn_objects(0)` gains one shape whose rect spans the rule's `from.x..to.x` (± 1 pt) at its `y`.
  - `one_undo_takes_every_part_and_rule_off` — `undo` → `text_runs(0)` and `drawn_objects(0)` as before the add; `redo` → both back.
  - `a_part_drawn_at_half_width_is_half_as_wide` — two marks `"WWWW"`, `scale_x` 1.0 and 0.5 → their runs' widths in the ratio 0.5 ± 0.02.
  - `a_text_mark_saved_before_parts_reads_as_one_unscaled_part` — JSON of an `Annotation::Text` without the three keys deserialises with `scale_x == 1.0` and both lists empty (no PDFium).
  - `a_marks_bounds_cover_its_parts_and_rules` — `bounds()` holds every part's glyph origins and both ends of every rule (no PDFium).
- [ ] **Step 2: Run** `cargo test --release -p pdf_core --test text_parts` with `PAGIFY_PDFIUM_LIB` set — expect FAIL (no such fields).
- [ ] **Step 3: Implement.** `write_text` writes the existing fields as the first part, then each of `more`, through one helper; each glyph's matrix is `(s·cos, s·sin, −sin, cos, x, y)` with `s` the part's `scale_x`; each rule is a stroked path (butt caps, the rule's width and colour, no fill) tagged with the mark's id exactly as `frame` is; the `restore` string stays on the very first object only. Add the three fields at every literal.
- [ ] **Step 4: Run — expect PASS**, then the whole `pdf_core` suite (only the two known `image_signature` failures). Mutation checks: rules untagged (undo leaves the line behind); `scale_x` ignored.

### Task 7: The panel shows mixed looks and the three character-spacing inputs

**Files:**
- Modify: `desktop/crates/pagify_app/src/text_style_panel.rs`; `main.rs` `draw_text_style` (build `Look` with `Mixed::One` for the Edit Text box, whose behaviour is unchanged)

**Interfaces:**
- Consumes: `pagify_shell::rich_text::{Mixed, Script}` — the panel's own `Script` enum is deleted and re-exported from there.
- Produces: `Look { font: Mixed<String>, size: Mixed<f32>, color: Mixed<[u8; 3]>, bold: Mixed<bool>, italic: Mixed<bool>, underline: Mixed<bool>, strike: Mixed<bool>, script: Mixed<Script>, letter_spacing: Mixed<f32>, width: Mixed<f32>, baseline: Mixed<f32>, align: Option<Align> }`; `Changes` gains `letter_spacing: Option<f32>, width: Option<f32>, baseline: Option<f32>`; `Gates` replaces `spacing` with `letter_spacing, width, baseline, line_spacing, paragraph_spacing`.

- [ ] **Step 1: Write the failing tests** (in the panel's test module, `Harness::new_ui` as in Task 1):
  - `a_mixed_toggle_is_neither_lit_nor_dark_and_turns_it_on` — `underline: Mixed::Mixed`: the Underline widget reports not selected and draws its outline; clicking it gives `changes.underline == Some(true)`.
  - `the_spacing_inputs_hand_back_what_was_picked` — with their gates open, choosing `125` in the width menu gives `changes.width == Some(125.0)`, `2` in the letter-spacing menu `Some(2.0)`, `-1` in the baseline menu `Some(-1.0)`.
  - `a_mixed_number_field_shows_no_number` — `letter_spacing: Mixed::Mixed` draws no digits in the field; its menu still offers the presets.
  - `a_gated_spacing_input_is_inert_with_its_reason` — a closed gate gives no change on click and hovers its reason.
- [ ] **Step 2: Run — expect FAIL.**
- [ ] **Step 3: Implement.** A mixed toggle: no fill, a 1 px `violet` outline, hover `"<name> — mixed"`. A number field: a `DragValue` (letter spacing −20..20 pt, width 10..400 %, baseline −50..50 pt) beside the menu of the Global Constraints' values; a mixed one is a blank field with the same menu. The line- and paragraph-spacing fields stay drawn and gated until phase 2.
- [ ] **Step 4: Run — expect PASS**, and the existing panel tests.

### Task 8: The new box holds rich text

**Files:**
- Modify: `desktop/crates/pagify_app/src/main.rs` — `NewTextBox`, `begin_text_box`, `draw_new_text_box`, `draw_text_style` (new-box branch), `choose_face` / `pick_style_face` (new box: the selection), `install_fonts`, the frame-top font installer, `FacesState`
- Modify: `desktop/crates/pagify_app/src/ui_tests.rs`, `g4_edit_error_tests.rs` — tests that read a box's `size` / `face` read `size()` / `rich`
- Create: `desktop/crates/pagify_app/src/rich_preview.rs` (`mod rich_preview;`) — the `LayoutJob` builder
- Test: `desktop/crates/pagify_app/src/text_style_tests.rs`

**Interfaces:**
- Consumes: Task 4 (`RichText`…), Task 7 (`Look`, `Changes`, `Gates`).
- Produces:
  - `NewTextBox { page, rect, buffer: String, rich: RichText, align: TextAlign, focused: bool }` and `NewTextBox::size(&self) -> f32` (`rich.look_at(0).size`, for the tests that read a box's size).
  - `rich_preview::face_family(face: &str) -> String` (`"pagify-face:{face}"`) and `rich_preview::box_job(rich: &RichText, text: &str, px_per_pt: f32, ready: &BTreeSet<String>, wrap_width: f32) -> egui::text::LayoutJob` — `text` is what the `TextEdit` holds this frame (the model is brought up to it on a copy first).
  - `install_fonts(ctx: &egui::Context, run_face: Option<Vec<u8>>, faces: &[(String, std::sync::Arc<Vec<u8>>)])`; `FacesState` gains `installed_run_face: Option<Vec<u8>>`, `box_faces: BTreeSet<String>` (installed) and `box_faces_ready: BTreeSet<String>` (usable this frame).

- [ ] **Step 1: Write the failing tests** (`ui_tests::harness("two-column.pdf")`, a box from `begin_text_box`, words put in `buffer`, the selection put into `egui::TextEditState` under the box's id `("new-text-box", page)`):
  - `underline_on_a_selection_underlines_only_those_characters` — `"Hello there"`, selection 6..11, click `Underline` → `rich.spans(0..11)` underlined exactly on 6..11.
  - `the_panel_acts_on_the_selection_the_box_had_before_the_click` — after the Underline click, clicking `Strikethrough` strikes 6..11 too, and 0..6 stays plain.
  - `with_nothing_selected_a_style_changes_every_character` — no selection, click `Underline` → all 11 underlined.
  - `typing_after_a_styled_word_continues_its_look` — `buffer.push('!')` and a frame → `!` underlined.
  - `a_selection_that_varies_shows_mixed_and_a_click_sets_it_for_all` — select 0..11 after underlining 6..11: Underline not selected; a click underlines all 11.
  - `the_box_draws_each_run_in_its_face_size_and_colour` — unit test of `box_job`: a run at 20 pt red in a ready face gives a section with `FontFamily::Name(face_family(face))`, `20 × px_per_pt` px, red, an underline stroke when underlined, `valign == Align::TOP` for `Super`.
  - `a_face_is_used_from_the_frame_after_it_is_installed` — choose `Montserrat-Bold` for a selection: that frame's `box_job` gives its section `FontFamily::Proportional`; two frames later `Name(face_family("Montserrat-Bold"))`; no panic over five frames.
  - `a_new_boxs_faces_do_not_unbind_the_edit_text_face` — open Edit Text on `text-lines.pdf` (its face installed and ready), Escape, open a new box with `Montserrat-Bold`, run frames, Escape, open Edit Text again: frames run without a panic and the run's face is installed again before it is drawn.
- [ ] **Step 2: Run — expect FAIL.**
- [ ] **Step 3: Implement.**
  - `draw_new_text_box`: after the `TextEdit`, `if buffer != rich.text() { rich.edit(&buffer) }`; its `layouter` is `box_job` with the view's `px_per_pt` and `box_faces_ready`. `apply_new_text_box` brings the model up to the buffer first too.
  - `box_job`: one section per span — family as above when ready, else `Proportional`; `size × px_per_pt`, × 0.58 for super/sub, with `valign` `TOP` for super; colour; underline / strikethrough strokes of `0.05 em`; `extra_letter_spacing = letter_spacing_pt × px_per_pt`; a width other than 100% split per character with `extra_letter_spacing` adding `(width/100 − 1)` × that character's advance (the letters keep their shape — the approximation the spec names).
  - Fonts: each frame the faces the box uses (`rich.runs()`) are wanted; at the top of the frame `box_faces_ready = box_faces`, and when a wanted face is not installed, `install_fonts` is called with **both** the Edit Text face (`installed_run_face`) **and** every wanted box face (bytes from `pdf_core::text::font_data`), `box_faces` becoming that set. The run face's own install passes the box faces likewise, so neither drops the other.
  - `draw_text_style` (new box): the selection from `TextEditState::load(ctx, box id).cursor.char_range()` (sorted; none or empty → every character); `Look` from `rich.look_of(range)` (bold/italic per run by `style_of(face)`, mixed when the runs differ); each `Changes` field through `rich.set_char`; Bold/Italic per span of the range through `family_variant(span's face, …)`, saying which family has no such face; a font picked in the picker (`choose_face`) through `set_char(face)` on the same selection. egui keeps a galley per `LayoutJob`, so a box that has not changed is not laid out again. Gates for a new box: everything of phase 1 open; line and paragraph spacing, indent and lists closed with "comes with paragraph styles"; alignment as today (the whole box).
- [ ] **Step 4: Run — expect PASS**, then `cargo test --release -p pagify_app`. Mutation checks: install a face and use it the same frame (the face test fails or panics); drop `installed_run_face` from a box install; take the selection from the focused widget only.

### Task 9: Add to Page writes the styled box as one mark

**Files:**
- Modify: `desktop/crates/pagify_app/src/main.rs` — `apply_new_text_box` (and the glyph placing it shares with `styled_line_command`)
- Test: `desktop/crates/pagify_app/src/text_style_tests.rs`

**Interfaces:**
- Consumes: Task 5 (`place_rows`, `FaceMeasure`, `RowIn`, `RowAlign`), Task 6 (`TextPart`, `TextRule`, the three fields), Task 8 (`box_job`, `NewTextBox::rich`).

- [ ] **Step 1: Write the failing tests:**
  - `add_to_page_writes_each_look_where_the_box_showed_it` — `"Hello there"` with `there` at 20 pt and underlined: the page's characters `Hello` at ≈ 14 pt and `there` at ≈ 20 pt; `t` where `place_rows` put it (± 0.5 pt) **and within 1 pt of where the box's own galley drew it**; one drawn line under `there` (its x range ± 1 pt, below the baseline).
  - `superscript_lands_smaller_and_raised` — `"x2"` with `2` super: `2` at ≈ 0.58 × 14 pt, its baseline ≈ 0.33 × 14 pt above `x`'s.
  - `letter_spacing_width_and_baseline_shift_reach_the_page` — each, read back from the characters' boxes.
  - `one_undo_takes_the_whole_box_off_and_redo_puts_it_back` — after Add to Page, `undo` once → no character or line of the box left; `redo` → all back; the message no longer says "`undo` N times".
  - `a_letter_the_face_cannot_draw_is_written_in_one_that_can_and_said` — a span in Montserrat-Regular holding `日`: no glyph 0 on the page for it, and the command bar names the face it was written in.
- [ ] **Step 2: Run — expect FAIL.**
- [ ] **Step 3: Implement.** Rows from the galley `box_job` gives with the same arguments as the frame's preview: each row's characters (its glyphs, plus the newline it ends with) and its baseline, from the row's largest normal-script glyph (`row.pos.y + glyph.pos.y`, over `px_per_pt`, from the box's top). `place_rows(…, RowAlign::from(box.align), &FaceMeasure)` (`impl From<TextAlign> for RowAlign` in `main.rs`, beside `TextAlign`). Each placed span is shaped with `pdf_core::text::shape` and its glyphs placed from `x_pt` by its `advances_pt` at `baseline_pt − offset_y × size_pt`; a span whose face shapes a non-space letter to glyph 0 is written in `face_that_draws` (or the bundled Montserrat-Regular), which is said. The first span is the mark's own fields, the rest `more`, every row's lines `rules`; one `Command::AddAnnotation`; "text added to page N."
- [ ] **Step 4: Run — expect PASS**, then the app suite. Mutation checks: one command per span; rules left out; glyph-0 check removed.

### Task 10: Ship phase 1 (0.1.89)

- [ ] **Step 1:** Check `TEXT_STYLE_DESIGN.md` still says what was built (it already carries this plan's refinements: one mark per box, named faces, one font installer, the third approximation); correct it where the build found otherwise.
- [ ] **Step 2:** Full suites — `pdf_core` (the engine changed), `pagify_shell`, `pagify_app` — per Global Constraints.
- [ ] **Step 3:** Version 0.1.89; publish; Architecture §28 (what phase 1 does, the measured file-size cost of a styled box, tests, limits) and §2–§6 counts.
