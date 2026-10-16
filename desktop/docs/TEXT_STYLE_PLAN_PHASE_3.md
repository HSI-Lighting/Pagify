# Text Style — phase 3 (character styles in the Edit Text box) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In the Edit Text box over a page's own words, every character style — font, size, colour, Bold, Italic, underline, strikethrough, superscript, subscript, letter spacing, character width, baseline shift — works on the selected characters (all of them when nothing is selected). Apply writes it into the page's own text, byte-safely and in one undo, and the box opens showing the looks the page already has.

**Architecture:** The engine's `TextLineEdit::Retype` gains `spans`. A piece whose words did not change is **restyled in place**: its own codes and spacing numbers are kept, cut at the span boundaries, with the text-state operators each span needs written between them and the piece's own state put back after. A retyped line is written segment by segment, each in its own font. Underlines and strike-throughs are filled rectangles, measured on a throwaway copy of the written page and drawn after the line's text object, tagged so that later edits and reads can find them. The app's `EditingRun` holds the phase-1 `RichText` model, seeded from the page. The panel acts on its selection and the preview draws each character's look. Apply sends spans only when looks changed, so every edit that works today takes the same path it takes today.

**Tech Stack:** Rust; egui/eframe 0.36.1 and `egui_kittest`; `pdf_core` over PDFium (content-stream parser and writer in `pdf_core::pdf::content`).

**Spec:** `desktop/docs/TEXT_STYLE_DESIGN.md` (§4a–4c, 4e, 4f). This plan refines §4e:
- Words that did not change are restyled in place rather than retyped.
- Rules are measured on a copy rather than computed from font widths, because the engine has no standard-14 widths (§2).
- Rules are tagged by their kind rather than by an edit id, since a line edit has no id; a piece's rules are found by where they sit.

Task 12 writes this back into the spec.

## Global Constraints

- Branch `pagify-windows-refactor`; check `git branch --show-current` and `git status --short` before any build or publish (Architecture §7).
- **Byte-safe:** only the operators of the pieces being restyled, retyped or removed change. Every other byte of the content stream is copied through.
- **Text state carries on:** whatever a span sets (`Tf`, `rg`, `Tc`, `Tz`, `Ts`) is put back straight after the piece, so everything after it is drawn exactly as before. The colour put back is the stream's own (`fill_before`).
- Superscript and subscript are drawn at **58%** of the character's size, raised **0.33** or lowered **0.14** of its size, with any baseline shift added. Letter spacing is in points after every character. Width is a percentage. All three are written as `Ts`, `Tc` and `Tz`, converted from page points through the operator's own magnification (`Placed::scale`).
- Rules (underlines and strike-throughs) are page content, never annotations. Position and thickness come from the font program where it can be read, otherwise −0.1 em / 0.05 em for an underline and 0.3 em / 0.05 em for a strike-through. A rule sits on the line's baseline: a raised character does not raise it. Rules other programs drew are not recognised.
- Anything the engine cannot follow is refused before anything is written, and the reason is said next to Apply, as today. One Apply is one undo.
- **Edits that work today keep today's path.** A box whose characters were never styled one by one (only whole-box size, colour, font, Bold or Italic), on a paragraph whose lines each have one look, sends exactly the commands it sends today.
- Text indices are characters, never bytes. Drawn lettering (outlined words) keeps one look; its character styles stay greyed: "Drawn lettering is replaced in one look."
- House rules: tests first; each new behaviour mutation-checked; suites with `PAGIFY_PDFIUM_LIB` and `PAGIFY_QPDF` set and `--no-fail-fast`, load-sensitive tests rerun alone (Architecture §6); no test opens a window or a terminal or touches the real clipboard.
- Ships as **0.1.91**: bump `desktop/Cargo.toml` line 27 and `pagify.iss` line 27, make sure Pagify.exe isn't running, then `powershell -NoProfile -ExecutionPolicy Bypass -File .\build.ps1 -Publish` from `desktop\packaging\windows`. Record it in Architecture §29. Commit and push only on the owner's word.

## Review Focus

1. **Lines and pieces nobody restyled or retyped stay byte-for-byte as they were.** → Task 11, `untouched_lines_are_left_byte_for_byte`.
2. **A look-only change moves no glyph it does not have to**: underlining or colouring a word in a justified paragraph leaves every word where it was. → Task 2, `restyling_in_place_keeps_the_spacing_numbers`; Task 11, `underlining_a_word_of_a_justified_paragraph_moves_nothing`.
3. **Re-opening a styled paragraph and retyping another word of the same line keeps the styled word's look.** → Task 11, `a_styled_word_survives_retyping_its_line`.
4. **A selection across two pieces of one line** (the producer split the line) styles both, and what follows the line is drawn as before. → Task 2, `the_state_a_span_sets_is_put_back_after_the_piece`; Task 11, `a_selection_across_two_pieces_styles_both`.
5. **Bold on a face with no Bold installed** says so and changes nothing. → Task 10, `bold_without_a_bold_face_says_so_and_changes_nothing`.

---

## Engine (`pdf_core`)

### Task 1: Spans in a line edit

**Files:** Modify `rust/pdf_core/src/document/mod.rs` (`SpanStyle`, `SpanFont`, `TextLineEdit::Retype`). Modify every literal `Retype { … }` (in `command/mod.rs`, `pdfium_doc.rs`, `tests/justify.rs`, `tests/replace_lines.rs`, `pagify_shell/src/paragraph_lines.rs`, `pagify_shell/tests/*`, `pagify_app/src/exact_text_tests.rs`, `paragraph_apply_tests.rs`). Test: `rust/pdf_core/tests/span_styles.rs` (new).

**Interfaces — produces:**
- `pub struct SpanStyle { pub chars: Range<usize>, pub font: Option<SpanFont>, pub size: Option<f32>, pub color: Option<Color>, pub char_spacing: Option<f32>, pub width_pct: Option<f32>, pub rise: Option<f32>, pub underline: bool, pub strike: bool }`.
  - `chars` counts characters of the edit's `text`.
  - `size` is the size drawn at, in page points, with a superscript's 58% already applied.
  - `rise` is in points, up from the baseline.
  - `None` means the piece's own value.
  - The struct is serde camelCase.
- `pub enum SpanFont { Face(String), LikeObject(usize) }`. `Face` is a registered face, embedded when needed. `LikeObject` is the font that page object is drawn in.
- `TextLineEdit::Retype { …, #[serde(default, skip_serializing_if = "Vec::is_empty")] spans: Vec<SpanStyle> }`.

- [ ] **Step 1: Write the failing tests:**
  - `a_line_edit_saved_without_spans_still_reads`: JSON with no `spans` key gives an empty list.
  - `a_line_edit_with_spans_round_trips_through_json`.
  - `a_span_outside_its_words_refuses_the_batch`: `chars: 3..40` on a 10-character text gives `InvalidArgument` with the page unchanged (same stream bytes).
- [ ] **Step 2: Run** `cargo test --release --test span_styles`, **expect FAIL** (compile).
- [ ] **Step 3: Implement.** Add the types and the field, with `spans: Vec::new()` at every literal. `replace_text_lines` refuses spans that are empty, overlapping, or outside the text, before anything is written. Spans still change nothing else; Task 2 writes them.
- [ ] **Step 4: Run, expect PASS**, then the `pdf_core` suite.

### Task 2: A piece restyled in place

**Files:** Modify `rust/pdf_core/src/pdf/content.rs` (`split_show`) and `pdfium_doc.rs` (`set_runs_in_stream`). Test: `tests/span_styles.rs`.

**Interfaces — produces:** `content::split_show(operation: &Operation, width: usize, segments: &[(Range<usize>, Vec<u8>)]) -> Vec<u8>`.
- It writes the operator's own codes, and the spacing numbers between them, as one `[…] TJ` per segment.
- Each segment is a range of code indices; together they cover every code in order. Each `TJ` is preceded by that segment's bytes (its state operators).
- A spacing number at a boundary stays with the segment before it.
- A quote operator's line move comes first (`quote_effect`), as today.

- [ ] **Step 1: Write the failing tests** on hermetic Helvetica pages (`page_with` from `replace_lines.rs`, moved into `tests/harness`). The page is `BT /F1 12 Tf 72 700 Td (Hello there) Tj 0 -14 Td (next line) Tj ET`. Each test is a `Retype` of object 0 with `text: "Hello there"` (its own words) and one span on 6..11:
  - `a_span_restyled_in_place_reads_back_in_its_size`: `size: Some(20.0)`. Runs read "Hello " at 12 and "there" at 20, and "there" starts where it did (origin x ± 0.05).
  - `the_state_a_span_sets_is_put_back_after_the_piece`: a span with size 20, colour red, `char_spacing: 2.0`, `width_pct: 50.0` and `rise: 3.0`. "next line" reads back at 12 pt, in black, at its old origin and width ± 0.05.
  - `restyling_in_place_keeps_the_spacing_numbers`: the same page drawn `[(Hel) -120 (lo the) -80 (re)] TJ`, and a colour span on 0..5. The stream's new operators still carry `-120` and `-80`, and every glyph's char box is where it was ± 0.01.
  - `letter_spacing_width_and_rise_reach_the_page_in_points`: under `2 0 0 2 0 0 cm` (magnification 2), a span with `char_spacing: 2.0` widens "there" by 2 pt per gap. `width_pct: 50` halves it, and `rise: 3.0` raises its box by 3 pt.
  - `nothing_outside_the_piece_changes`: the stream bytes before and after the operator's span are identical.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement** in `set_runs_in_stream`. An edit whose `text` equals what its operator's codes spell (each code's ToUnicode spelling joined, the same string `run_exact_texts` gives), and whose spans set no `font`, is written with `split_show`:
  - Map span characters to code ranges through each code's spelling length.
  - A styled segment is preceded by its `Tf` (size ÷ vertical scale, the piece's own font), `r g b rg`, `Tc` (points ÷ `Placed::scale`), `Tz` and `Ts` (points ÷ scale), for the fields it sets.
  - The first unstyled segment after a styled one, and the end of the piece, put back the piece's own `Tf`, `fill_before`, `Tc`, `Tz` and `Ts`, taken from `content::states` at the operator. Only what was changed is put back.
  - Any other edit with spans is left to Task 3; until then it is refused as `Unsupported("…")`.
- [ ] **Step 4: Run, expect PASS**; `pdf_core` suite. Mutation checks: drop the put-back (the next-line test fails); move a boundary spacing number to the next segment (the spacing test fails).

### Task 3: A retyped line written segment by segment, in several fonts

**Files:** Modify `pdfium_doc.rs` (`set_runs_in_stream`, `embed_typing_font`, `TOO_MANY_EMBEDS_IN_ONE_BATCH`). Test: `tests/span_styles.rs`.

**Interfaces — produces:**
- `EmbedBatch`, private to `pdfium_doc.rs`, holds a batch's embedded faces. The next free object number and the resource names it took run on from one embed to the next. Each face is embedded once per batch, whatever the number of edits that use it, and the page patch is built once from all the additions.
- `embed_typing_font(…, batch: &mut EmbedBatch)`.

- [ ] **Step 1: Write the failing tests:**
  - `a_retyped_line_with_a_span_writes_each_segment_in_its_look`: `text: "Hi there"`, span 3..8 at 20 pt. "there" reads back at 20 and the next line is unchanged.
  - `a_span_like_another_piece_is_written_in_that_pieces_font`: the line is `(Hello) Tj /F2 12 Tf ( there) Tj`, with F2 = Helvetica-Bold. Retyping object 0 to "Hello there!" with `LikeObject(1)` on 5..12, removing object 1, gives "there!" in F2's font (`run_font_names`) and "Hello" in F1.
  - `two_lines_with_a_bold_span_embed_the_face_once`: `Face(Montserrat-Bold)` (bundled bytes offered) on a word of each of two lines, in one batch. Both read in the face, and the saved file holds one embedded program for it (`check_both_saves` clean).
  - `bold_and_italic_spans_in_one_line_embed_both`: two `Face` spans with different faces in one edit; both read back.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement.** For an edit whose words changed, or with a `font` span:
  - Split `text` at the span boundaries and encode each segment in its own font. With no font, use the piece's own font and the three-tier rule per segment (its font, then `borrow_font_on_page`, then embed). `LikeObject` uses that object's font resource (via `operator_of`) and its map. `Face` uses `embed_typing_font(Some(face))`.
  - Write `[before codes] TJ`, then each segment's state and `TJ`, then the own state put back, then `[after codes] TJ`.
  - The tracking numbers (`content::tracked`) apply only to segments in the piece's own font.
  - With `EmbedBatch` every batch is written in one pass, so delete `TOO_MANY_EMBEDS_IN_ONE_BATCH` and `replace_text_lines_one_at_a_time`. Rename `replace_lines::lines_that_each_need_a_new_font_are_replaced_one_at_a_time_and_atomically` for what it now checks: both lines written in one pass, and a refusal leaves the page as it was.
- [ ] **Step 4: Run, expect PASS**; `pdf_core` suite (`replace_lines`, `justify`, `typing`). Mutation: embed per edit instead of per batch (the two-lines test fails, or its save is not clean).

### Task 4: A styled retyped line of a justified paragraph keeps its width

**Files:** Modify `pdfium_doc.rs` (`Stretch`, `Stretch::spread`). Test: `tests/span_styles.rs`.

- [ ] **Step 1: Write the failing test** `a_styled_line_stretched_to_its_width_spans_it`. Take a hermetic two-line paragraph whose first line is `[(alpha) -250 (beta) -250 (gamma)] TJ`. Retype it as "alpha beta gamma delta" with a 14 pt span on "beta" and `justify_to` set to its old width. Read back: from the first segment's left to the last segment's right is the target ± 0.05 pt.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement.** A segmented replacement is measured on the plain copy as one line: the first object's left to the last object's right, using renumbering as in Task 5.
  - The difference is shared among the spaces of every segment.
  - Each segment's spacing number is computed in its own units (`size × scale × Th / 1000`).
  - The existing refusals hold, applied to the shared gap.
- [ ] **Step 4: Run, expect PASS**; `justify` suite. Mutation: share the gap in the first segment's units only (the test fails).

### Task 5: Underlines and strike-throughs

**Files:** Modify `pdfium_doc.rs` (`set_runs_in_stream`, the measuring copy) and `rust/pdf_core/src/text/mod.rs` (`line_metrics_of`). Test: `tests/span_styles.rs`.

**Interfaces — produces:**
- `pdf_core::text::line_metrics_of(program: &[u8]) -> Option<LineMetrics>`, which `line_metrics(name)` becomes a wrapper of.
- The tag, `/PagifyRule <</Kind 1>> BDC … EMC` (1 = underline, 2 = strike-through). It is read through PDFium's page-object marks: name `PagifyRule`, int param `Kind`.

- [ ] **Step 1: Write the failing tests** (object 0 restyled in place, with `underline` on 6..11):
  - `an_underline_is_drawn_under_its_words`: one new shape object. Its x range is "there"'s origin to the next segment's origin ± 0.5 pt (to its own right edge when it is the line's last). It sits below the baseline, and its height is ≈ 0.05 × 12 ± 0.2.
  - `a_strike_through_crosses_its_words_at_the_strikeout_height`.
  - `a_rule_takes_its_words_colour`: the span is red and underlined, and the rule's fill (`FPDFPageObj_GetFillColor`) is red.
  - `a_rule_lands_under_words_drawn_through_a_transform`: the text object is under `1 0 0 1 50 20 cm`, and the rule is still under the words.
  - `one_undo_takes_the_rules_off_with_the_words`, plus `check_both_saves`.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement.**
  - Measure on a copy of the batch as it will be written, opened as the stretch's copy is. With no line to stretch, that is the stretch's own plain copy; with one, it is a second copy written after the stretch.
  - Find each ruled segment's object by renumbering. Removals before it lower the number. A replacement of N show operators raises everything after it by N − 1. Check the first segment's origin against the original piece (± 0.05), as the stretch does, or refuse with "could not be measured".
  - The rule runs from the segment's origin to the next segment's origin (or its own right edge), at the baseline minus position × size, with thickness × size. Metrics come from `line_metrics_of(run_font_data)`, otherwise `LineMetrics::FALLBACK`.
  - Insert `/PagifyRule <</Kind k>> BDC q <inverse of the CTM at the ET> cm r g b rg x y w h re f Q EMC` straight after the `ET` that closes the piece's text object. Points are mapped to PDF space by `RawPage::space`, then into the user space in force there.
- [ ] **Step 4: Run, expect PASS**; `pdf_core` suite. Mutation: take the inverse CTM out (the transform test fails); use the span's top instead of the baseline (the position test fails).

### Task 6: A rewritten or removed piece takes its old rules with it

**Files:** Modify `pdfium_doc.rs` (`set_runs_in_stream`). Test: `tests/span_styles.rs`.

- [ ] **Step 1: Write the failing tests:**
  - `restyling_an_underlined_word_without_the_underline_takes_the_rule_off`.
  - `removing_a_line_takes_its_rules_off`.
  - `a_piece_left_alone_keeps_its_rule`: two pieces are underlined and one is retyped; only the retyped one's rule changes.
  - `a_rule_other_programs_drew_is_left_alone`: an untagged rectangle under the words survives a retype.
  - `nothing_of_a_removed_rule_is_left_in_the_stream`: no empty `BDC … EMC` remains.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement.**
  - Tagged rules are found by a walk over the page's path objects for the `PagifyRule` mark; the census is read only when the page has at least one.
  - A rule belongs to a piece when its centre lies between 0.5 em above and 0.4 em below the piece's baseline and its x range overlaps the piece's box by at least half the piece's width.
  - The rules of every retyped, restyled or removed piece are cut in the same pass. Each is cut with `operator_span`, widened to the enclosing `PagifyRule` `BDC … EMC`.
- [ ] **Step 4: Run, expect PASS**; `pdf_core` suite. Mutation: match rules by line rather than by piece (the left-alone test fails).

### Task 7: Reading the page's looks back

**Files:** Modify `rust/pdf_core/src/document/mod.rs` (`RunTextLook`, `Document::run_text_looks`), `pdfium_doc.rs`, and `desktop/crates/pagify_shell/src/session/reading.rs` (the wrapper). Test: `tests/span_styles.rs`.

**Interfaces — produces:**
- `pub struct RunTextLook { pub char_spacing: f32, pub width_pct: f32, pub rise: f32, pub underline: bool, pub strike: bool }`, in points.
- `Document::run_text_looks(&self, page_index: usize, wanted: &HashSet<usize>) -> Result<HashMap<usize, RunTextLook>>`. The default is an empty map.
- `Session::run_text_looks(&self, page: usize, objects: &[usize]) -> Result<HashMap<usize, RunTextLook>>`.

- [ ] **Step 1: Write the failing tests:**
  - `a_pieces_spacing_width_and_rise_read_back_in_points`: `BT /F1 12 Tf 2 Tc 50 Tz 3 Ts …` under a magnification of 2 reads 4, 50 and 6.
  - `an_underline_written_here_reads_back_on_its_piece_only` (after Task 5's write).
  - `an_untagged_line_under_words_is_not_read_as_an_underline`.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement.** For each wanted object, `operator_of` gives its operator, and `content::states` at that operator gives `Tc`, `Tz` and `Ts`, multiplied by `Placed::scale`. Tagged rules are matched to pieces as in Task 6. Everything is read from one page open, the stream parsed once.
- [ ] **Step 4: Run, expect PASS**; `pdf_core` and `pagify_shell` suites.

## The model (`pagify_shell`, pure)

### Task 8: A page's font in the model, and looks into spans

**Files:** Modify `desktop/crates/pagify_shell/src/rich_text.rs`. Create `desktop/crates/pagify_shell/src/line_styles.rs` (`pub mod line_styles;` in `lib.rs`).

**Interfaces — produces:**
- `CharLook.page_font: Option<u32>`: the page's font id (`RunStyle::font`) this character is drawn in. It is `None` for typed-in faces, and `CharChange.face` clears it.
- `RichText::from_looks(text: &str, looks: &[CharLook]) -> RichText`, with one look per character, neighbours merged.
- `line_styles::piece_look(face: String, page_font: u32, size: f32, color: [u8; 4], look: &RunTextLook, line_size: f32) -> CharLook`.
  - A rise of 0.33 × `line_size` (± 0.05 ×) with a size of 0.58 × `line_size` (± 0.06 ×) reads as `Script::Super` at `line_size`, with the remainder as baseline shift. A rise of −0.14 × reads as `Sub`.
  - Anything else reads as baseline shift and size as found.
- `line_styles::align_to_box<P: Copy>(raw: &[(char, P)], fixed: &str) -> Vec<Option<P>>`. It pairs each character of `fixed` (one line of the box) with the raw character it came from. A dropped control code is skipped, a code turned into "-" keeps its source, and an appended "-" gets `None`.
- `line_styles::line_spans(rich: &RichText, chars: Range<usize>, base: &CharLook, font_objects: &HashMap<u32, usize>) -> Vec<SpanStyle>`.
  - A character needs a span when its look differs from `base` in any written field, or when it is underlined or struck.
  - The font is `None` when face and `page_font` equal the base's, `LikeObject(font_objects[page_font])` for another page font, and `Face(face)` otherwise.
  - Size and rise are worked out per the Global Constraints.
  - Equal neighbours are merged, and `chars` count from the start of `chars`.

- [ ] **Step 1: Write the failing tests:**
  - `from_looks_merges_equal_neighbours`.
  - `a_face_change_forgets_the_page_font`.
  - `super_and_sub_read_back_from_size_and_rise`.
  - `a_rise_that_fits_neither_is_a_baseline_shift`.
  - `align_to_box_skips_a_dropped_code_and_marks_an_added_hyphen`.
  - `line_spans_is_empty_for_a_line_in_its_base_look`.
  - `an_underlined_base_look_still_gets_its_span`.
  - `another_page_font_is_like_its_object`.
  - `superscript_becomes_58_percent_and_a_third_of_an_em_up`.
  - `neighbouring_characters_with_one_look_make_one_span`.
- [ ] **Step 2: Run** `cargo test --release -p pagify_shell line_styles rich_text`, **expect FAIL.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run, expect PASS**; `pagify_shell` suite. Mutation: leave out the underline-only case (its test fails).

## The Edit Text box (`pagify_app`)

### Task 9: The box opens with the page's looks

**Files:** Modify `desktop/crates/pagify_app/src/main.rs` (`EditingRun`) and `edit.rs` (`build_editor_from_lines`, the single-run pick at `edit.rs:200`). Test: `desktop/crates/pagify_app/src/edit_text_style_tests.rs` (new; `#[cfg(test)] mod edit_text_style_tests;`).

**Interfaces — produces** `EditingRun` gains:
- `rich: RichText` and `picked: RichText` (the looks as opened);
- `selection: Range<usize>`, kept while the box has the focus, as `NewTextBox` does;
- `styled: bool`;
- `line_base: Vec<CharLook>`: each line's first piece's look;
- `box_chars: Vec<Option<(usize, usize)>>`: for each character of `picked`, its piece and index in that piece's exact text, `None` for `\n` and added hyphens;
- `piece_texts: HashMap<usize, String>`: each piece's exact text;
- `font_objects: HashMap<u32, usize>`.

Drawn lettering gets a `RichText` in its one look, and `styled` stays false.

- [ ] **Step 1: Write the failing tests** on hermetic pages: `wrap_hyphen_tests::open_page(name, &[], content)`, whose page already defines F1 (Helvetica) and F2 (Helvetica-Bold):
  - `a_word_the_page_sets_larger_opens_larger`: a line drawn `(Hello ) Tj /F1 20 Tf (there) Tj` opens with `rich.look_of(6..11).size == One(20.0)`.
  - `a_piece_in_another_font_opens_in_it`.
  - `a_rise_and_letter_spacing_open_as_the_page_has_them`.
  - `a_paragraph_of_one_look_opens_unstyled`: `styled == false` and `rich == picked`.
  - `a_box_char_maps_to_its_piece_through_a_mended_hyphen`: the U+0002 case, as `pick_wiring_tests` builds it.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement.**
  - Read `run_text_looks` once for every piece of the paragraph. `run_styles` and `run_font_names` come from what the pick already holds.
  - Build each piece's `piece_look` with `line_size` set to the line's first piece's size.
  - Align each line's raw characters to the box's line with `align_to_box`.
  - Then `RichText::from_looks`.
- [ ] **Step 4: Run, expect PASS**; the Edit Text tests (`exact_text_tests`, `pick_wiring_tests`, `editor_blank_glyph_tests`) unchanged.

### Task 10: The panel and the preview act on the selection

**Files:** Modify `main.rs` (`draw_text_style`'s Edit Text branch, `draw_box_style`, `style_box_selection`, `choose_face`, `draw_run_editor`, `editor_layout_job`, the frame-top font installer). Test: `edit_text_style_tests.rs`.

**Interfaces — produces:**
- `StyleTarget { NewBox, EditText }`.
- `fn restyle_selection(&mut self, target: StyleTarget, range: Range<usize>, changes: &tsp::Changes)`, the body of `draw_box_style`'s apply part, shared by both targets.
- `editor_layout_job(…, looks: &EditorLooks)`. `EditorLooks { rich: RichText (edited to the buffer), base_size: f32, base_px: f32, doc: TextFormat, fallback: TextFormat, ready: BTreeSet<String> }`.
  - Per character: size `base_px × size ÷ base_size` (58% for super/sub), its colour, underline or strike strokes, `valign` TOP for super, `extra_letter_spacing`, and the width as in `rich_preview`.
  - A typed-in face is used once it is ready, otherwise the document's face, as today.

- [ ] **Step 1: Write the failing tests** (kittest, a picked hermetic paragraph):
  - `underline_on_a_selected_word_of_the_box_underlines_only_it`.
  - `the_edit_text_panel_shows_mixed_for_a_varied_selection`.
  - `with_nothing_selected_underline_takes_every_character`.
  - `a_whole_box_size_change_is_still_the_boxs_style`: no selection, size 16 gives `style.size == Some(16.0)` with `styled` still false.
  - `bold_without_a_bold_face_says_so_and_changes_nothing`: `system_fonts = Some(vec![])` and a face with no family.
  - `the_preview_draws_an_underlined_word_underlined` (the job's sections).
  - `the_character_styles_are_open_and_the_paragraph_styles_say_next`.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement.**
  - The Edit Text branch builds `Look` from `rich.look_of(selection)`, as the new box does.
  - Gates: every character style is open; justify, indent, lists and line/paragraph spacing say "Comes with paragraph styles, the next stage." Drawn lettering keeps everything but font, size and colour closed, saying "Drawn lettering is replaced in one look."
  - **A change with an empty selection** to size, colour, font, Bold or Italic sets the box's `style` (today's path) and the same on every character of `rich`. Any other change, or any change to a non-empty selection, sets `rich` only and `styled = true`.
  - The box keeps its own selection while it has the focus (id `("run-editor", page, object)`).
  - The installer's wanted faces include the box's typed-in faces.
  - The box grows by the largest character size over `was.size`.
- [ ] **Step 4: Run, expect PASS**; `text_style_tests` and the run-editor tests (`run_editor_*`, `editor_blank_glyph_tests`) unchanged. Mutation: set `styled` for an empty-selection size change (the whole-box test fails).

### Task 11: Apply writes the looks

**Files:** Modify `edit.rs` (`apply_one_edit`, `apply_paragraph_edit`), `main.rs` (`paragraph_edit_commands_with`), and `pagify_shell/src/paragraph_lines.rs` (`plan_paragraph_edit`, `line_edits`). Test: `edit_text_style_tests.rs`, and `desktop/crates/pagify_shell/src/paragraph_lines.rs` tests.

**Interfaces — produces:**
- `plan_paragraph_edit(original, frozen, typed, restyle: &dyn Fn(usize, usize) -> bool)`. The predicate takes the original line and the typed line, and says the line must be written though its words are the same. Today's callers pass `&|_, _| restyle`.
- `line_edits` takes `spans_for: &dyn Fn(usize) -> LineSpans`. `LineSpans` is `Whole(Vec<SpanStyle>)` for a retyped line, or `InPlace(Vec<(usize, String, Vec<SpanStyle>)>)` for a look-only line: object, its exact text, its spans.

- [ ] **Step 1: Write the failing tests** (end to end through the real Apply button, on hermetic pages unless named):
  - `underlining_a_word_writes_a_rule_under_exactly_it`.
  - `a_superscript_word_lands_smaller_and_raised`.
  - `colouring_one_word_colours_only_it`.
  - `letter_spacing_one_word_moves_only_the_words_after_it`.
  - `a_selection_across_two_pieces_styles_both`.
  - `untouched_lines_are_left_byte_for_byte`: the stream's operators for the other lines are identical.
  - `underlining_a_word_of_a_justified_paragraph_moves_nothing`: every char box ± 0.01 pt except the rule's.
  - `a_styled_word_survives_retyping_its_line`: underline, Apply, reopen, retype another word of that line, Apply. The word is still underlined and the rule is under its new place.
  - `one_undo_puts_the_paragraph_back`.
  - `a_single_run_takes_styles_too`.
  - `an_edit_of_one_look_sends_what_it_sent_before`: the commands built for a whole-box size change equal today's (`paragraph_apply_tests`' helpers).
  - On `VIVO mini.pdf` (skipped when absent): `underlining_a_word_of_vivo_changes_that_word_only`, with a render saved for a look.
- [ ] **Step 2: Run, expect FAIL.**
- [ ] **Step 3: Implement.**
  - **`styled == false` and `picked` has one look per line:** today's code, untouched.
  - **Otherwise, always through the paragraph path**, single runs too. A typed line paired with an original line is written when its words or its character looks differ.
  - A **look-only** line (its words unchanged) becomes one `Retype` per piece whose looks changed. Each takes the piece's exact text and its spans, mapped through `box_chars`, with nothing removed and no `justify_to`. A line with twins takes the retype path instead.
  - A **retyped** line becomes one `Retype` of its first piece with `line_spans` against `line_base`, its other pieces and twins removed, and `justify_to` as today.
  - **No separate recolour command:** colour travels in the spans.
  - Surplus typed lines are written below in the line's base look (Global Constraints).
- [ ] **Step 4: Run, expect PASS**; `pagify_app` suite. Mutation: write look-only lines as retypes (the justified test fails); drop the predicate (the survive test fails).

### Task 12: Ship 0.1.91

- [ ] **Step 1:** Check `TEXT_STYLE_DESIGN.md` against what was built. Write in §4e the refinements this plan makes: restyle in place, rules measured on a copy, rules as tagged filled rectangles after the text object, today's path for unstyled edits. Correct anything the build found otherwise.
- [ ] **Step 2:** Full suites: `pdf_core`, `pagify_shell`, `pagify_app`, per Global Constraints.
- [ ] **Step 3:** Bump to 0.1.91 and publish. Architecture §29 covers what phase 3 does, what a styled line costs in the file (measured), the tests and the limits; update the §2–§6 counts.
