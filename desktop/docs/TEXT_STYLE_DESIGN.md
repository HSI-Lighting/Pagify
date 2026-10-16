# Text Style panel, every control working — design

Written 2026-10-15 against `pagify-windows-refactor` (clean tree at `74f0db3`, v0.1.87). Status: agreed with the owner section by section on 2026-10-15, and refined the same day by the phase 0–1 plan (`TEXT_STYLE_PLAN_PHASES_0_1.md`): one mark per box, a named font always, one font installer, a third preview approximation.

## 1. Goal and success criteria

Every control of the Properties panel's Text Style block works, in a new text box and in the Edit Text box over a page's own words. Done means:

1. Bold, italic, underline, strikethrough, superscript, subscript, letter spacing, character width and baseline shift change **the selected characters**, or all the words when nothing is selected.
2. Alignment (left, centre, right, justify), indent and outdent, bulleted and numbered lists, line spacing and paragraph spacing change **the paragraph the cursor or selection is in**.
3. What the box shows is what lands on the page. One model and one layout feed the preview and the writer, so they cannot disagree; the approximations the preview has to make are named in §4c.
4. The Italic button's letter is visible, and the font field always names a font — never "(automatic)".

## 2. Findings (the tree at 0.1.87)

- **The panel already draws every control** (`pagify_app/src/text_style_panel.rs`): it is given what to show (`Look`), what works (`Gates`), and hands back what changed (`Changes`). `draw_text_style` (`main.rs`) un-gates only the font, size, colour, Bold and Italic, and a new box's left/centre/right. The spacing fields are drawn with fixed strings (`"0.00"`, `"100%"`) and do nothing.
- **Bold and Italic are faces, not effects**: `pick_style_face` → `family_variant` finds the family's own Bold/Italic face among the installed and bundled fonts, and `choose_face` makes it the style's `face`.
- **The Italic letter is invisible.** `draw_letter` leans it with `shape_to_sheared`, which copies the galley's row meshes into a `Shape::mesh`. A galley's UVs are in texels until epaint's `tessellate_text` normalises them (`uv_normalizer`, `epaint-0.36.1/src/tessellator.rs:2030`), so the raw mesh samples outside the font atlas. egui has faux italics of its own: `TextFormat::italics` (`text_layout.rs:1173`).
- **The engine's text style is four fields**: `TextStyle { size, color, at, face }` (`pdf_core/src/document/mod.rs`). An existing line is rewritten with `TextLineEdit::Retype { first, text, style, remove, justify_to }` through `set_runs_in_stream`, byte-safe: only the retyped operators change.
- **The Edit Text box has one look for all its words** (`EditingRun::style`). Its buffer has one `\n`-separated entry per page line, and its preview (`editor_layout_job`) lays out those explicit lines, justified by hand.
- **A new box has one look and one alignment** (`NewTextBox { rect, buffer, size, color, face, align }`). `apply_new_text_box` wraps the buffer with **egui's default proportional font at the box's size — not the chosen face** — and writes each row with `styled_line_command`, which shapes it with `pdf_core::text::shape` and places every glyph itself.
- **`write_text`** (`pdfium_doc.rs`) writes one text object per glyph with a full matrix (`FPDFPageObj_Transform`), tags every object with the mark's id (`TEXT_MARK_NAME`), and writes a caption's `frame` ring as a path tagged with the same id.
- **An added text mark is undone by its id** (`UndoRecord::RemoveText`), and `remove_text` refuses an id nothing carries any more. `Annotation::bounds` reads only a text mark's glyph origins. **The engine has no standard-14 widths**: a box in base-14 Helvetica (today's "(automatic)") can be written but not measured.
- **egui 0.36.1's `TextFormat`** has `font_id`, `extra_letter_spacing`, `line_height`, `color`, `background`, `italics`, `underline`, `strikethrough` and `valign`. It has no horizontal scale and no arbitrary rise.

## 3. Decisions (agreed with the owner, 2026-10-15)

| Decision | Choice |
|---|---|
| What a control changes | Character styles: the selection, or every word when nothing is selected. Paragraph styles: the paragraph the cursor or selection is in |
| Paragraphs | In a new box, every Enter starts one. The Edit Text box opens one of the page's paragraphs, so there the box is that paragraph |
| Typing | Continues the look at the cursor |
| The font's name | Never "(automatic)": a new box starts in Arial, or the bundled Montserrat-Regular where Arial is not installed; the Edit Text box names the page's font; the picker's first entry names the page's own font |
| A look that varies inside the selection | Shown *mixed* (toggle neither lit nor dark, field blank); pressing a toggle sets it for all of it |
| Approach | One rich-text model and one layout in `pagify_shell`, shared by both previews and both writers |
| Order | New text boxes first (phases 1–2), the Edit Text box after (phases 3–4); the Italic icon first of all (phase 0) |
| Underline and strikethrough | Page content (tagged filled lines), **not** annotations: annotations do not print everywhere and previews such as WhatsApp's skip them (Architecture §21b) |
| Superscript / subscript | 58% of the size, raised 0.33 em / lowered 0.14 em (`Ts`); line spacing unchanged; exclusive |
| Letter spacing | Points after every character (`Tc`); menu −1, −0.5, 0, 0.5, 1, 2, 5 |
| Character width | Horizontal scale in % (`Tz` on existing text, the glyph matrix on new text); menu 75, 90, 100, 110, 125, 150 |
| Baseline shift | Points up (+) or down (−) (`Ts`), added to super/sub; menu −3 … +3 |
| Justify | Every line but the paragraph's last spread to the box's width by widening the word gaps |
| Indent / outdent | The paragraph's left edge in 18 pt steps, never past the box's own left edge |
| Lists | Marker at the left edge, text on a hanging indent; markers • ◦ ▪ – and 1. 1) a. A. i. from the buttons' menus; numbers count consecutive numbered paragraphs; pressing the lit button removes the list; the marker is real text, in the paragraph's font if it has the glyph, else a bundled face |
| Line spacing | Baseline to baseline as a multiple of the size; menu 1.0, 1.15, 1.2, 1.5, 2.0; existing text opens showing what the page has |
| Paragraph spacing | Points after each paragraph; menu 0, 3, 6, 12, 18 |

## 4. Design

### 4a. The model — `pagify_shell/src/rich_text.rs` (new, pure)

- `CharLook { face: String, size: f32, color: [u8; 4], underline: bool, strike: bool, script: Script, letter_spacing_pt: f32, width_pct: f32, baseline_pt: f32 }`, `Script { Normal, Super, Sub }` — the face always named (§3).
- `ParaLook { align: Align, indent_pt: f32, list: Option<ListKind>, line_spacing: f32, space_after_pt: f32 }`, `Align { Left, Center, Right, Justify }`, `ListKind { Bullet(char), Number(NumberStyle) }`.
- `RichText { text: String, runs: Vec<(usize /*end, in chars*/, CharLook)>, paras: Vec<ParaLook> }`. Indices are **characters**, as egui's `CCursor` counts them; runs cover the text end to end and adjacent runs that look the same are merged.
- `edit(&mut self, new_text)` keeps the runs in step with typing: the change is the span between the common prefix and suffix of the old and new text, removed characters take their runs with them, and inserted characters take the look of the character before the insertion (the one after, at the very start). Paragraph looks follow the `\n`s the same way.
- `set_char(range, &CharChange)` and `set_para(range, &ParaChange)`; an empty range means the whole text for `set_char` and the cursor's paragraph for `set_para`.
- `look_of(range) -> LookOf` with each field `Mixed<T>` (`One(T)` or `Mixed`), which is what the panel shows.

### 4b. The layout — `pagify_shell/src/text_layout.rs` (new, pure)

- Built in phase 1 as `place_rows(rich, rows: &[RowIn { chars, baseline_pt }], left_pt, width_pt, RowAlign, &dyn Measure) -> Vec<PlacedRow { spans, rules }>`. **Line breaks are an input**: a new box's rows come from what its editor showed, the Edit Text box's from the page's own lines. The layout places, it does not wrap. Each row's baseline comes from the preview's own glyphs too; on the page a row's normal-script spans share the baseline of its largest glyph. Phase 2 adds the paragraph rules (marker, indent, justify, spacing) to it.
- `Measure` gives a face's advances (`advances_em(face, text)`, one per glyph) and its `LineMetrics`: ascent, descent, underline and strikeout position and thickness. The app implements it with `pdf_core::text::shape` and the face's `post` and `OS/2` tables (falling back to −0.1 em / 0.05 em and 0.3 em / 0.05 em when a font has none); tests use a fixed-width stand-in.
- `PlacedSpan { chars, text, look, x_pt, baseline_pt, size_pt, advances_pt, scale_x }` (its rise is in `baseline_pt`; letter spacing and width are in `advances_pt`, with no spacing after a row's last glyph) and `LineRule { x0_pt, x1_pt, y_pt, thickness_pt, color }`.
- Each rule of §3 is one function here: alignment and justify offsets, indent clamping, marker text and hanging indent (`max(18 pt, marker width + 6 pt)`), numbering, baseline-to-baseline spacing (the multiple times the largest size on the line), space after a paragraph's last line, super/sub size and rise, letter spacing and width on the advances, baseline shift on the rise, and one rule per stretch of underlined or struck spans on a line.

### 4c. The panel and the two previews — `pagify_app`

- `text_style_panel`: `Look` fields become `Mixed<…>`; the spacing fields become real inputs (a `DragValue` and a menu of §3's values); `Changes` gains `letter_spacing`, `width`, `baseline`, `indent` (±1 step), `list`, `line_spacing` and `space_after`.
- `draw_text_style` acts on the box's selection, shows `look_of(selection)`, and applies `Changes` through `set_char` / `set_para`. **egui lets go of a text field's selection when it loses the focus**, which a click on the panel does, so the box keeps its own copy (`NewTextBox::selection`), taken from `egui::TextEditState` only on frames it has the focus.
- Each editor's `TextEdit` gets a `layouter` that builds its `LayoutJob` from the model: one section per run — the run's face (registered with egui, as the Edit Text box already does for its run's face), size, colour, `underline` / `strikethrough` strokes, `valign` and 58% size for super/sub, `extra_letter_spacing` — and, from `lay_out`, each row's leading space for alignment, indent and the marker, and `line_height` for the spacing. List markers are drawn over the row, not typed into the buffer. The caret and the selection therefore sit where the words are drawn.
- **Three approximations**, named in the hover text of the width, baseline-shift and size fields: the preview spaces letters for a character width but does not squeeze their shapes; it does not draw a baseline shift at all (egui has no arbitrary rise, and `valign` cannot move a letter within a row of one size); and on a line of mixed sizes it sits the smaller words a little low (egui aligns their bottoms). The page has all three exactly.
- **The new box's preview uses the chosen faces**, not egui's default font, so its rows are the rows the writer reproduces. `install_fonts` replaces egui's fonts wholesale, so one call carries the Edit Text face and every face the box uses, and a face is drawn from the frame after it is installed — drawing in a family not yet bound panics.

### 4d. Writing a new box (phases 1–2)

- The rows the editor showed → `lay_out` with the real faces → **one `Annotation::Text` for the whole box**: its first span in the existing fields, every other span in a new `more: Vec<TextPart>` (each with its own font, size, colour, glyphs and `scale_x`), and every row's underlines and strike-throughs in a new `rules: Vec<TextRule>`, written as tagged stroked lines the way `frame` is. One id, one `AddAnnotation`, one undo, one mark for the eraser.
- **One mark rather than one per span:** an added text mark is undone by its id and `remove_text` refuses an id already gone, so spans sharing an id would fail a batch undo after its first member; spans with ids of their own would each be a separate mark to erase.
- `scale_x` (default 1.0) goes into each glyph's matrix in `write_text`; `Annotation::bounds` takes in the parts and the rules, so the rule that nothing goes over a lock covers them.
- As built in phase 1: the rows are laid out again with the preview's own arguments — the width the text field last wrapped at (`NewTextBox::wrap_px`, recorded in its layouter) included. Each placed span is a part of its own, in reading order — the page lists its glyphs in the order they are written, which is the order a selection or a copy takes them in. **`write_text` embeds one subset per face across the mark's parts**: PDFium names a page's fonts by their BaseFont as it writes the content, so a subset per part put two fonts of one name on the page and the second part's letters were read through the first's. A span whose face shapes a letter to glyph 0 is written in a face that draws it, and that is said; where none can, it keeps its face and that is said. A box that cannot be written stays open with its words.

### 4e. Writing the page's own words (phases 3–4)

- `TextLineEdit::Retype` gains `spans: Vec<SpanStyle>` (empty = today's single style). **Restyled in place, written as a difference.** A line is cut into stretches, one show operator each (`content::coded` / `show_pieces`); before each stretch the engine writes only what differs from the stretch before it (`Tf`, `rg`, `Ts`, `Tc`, `Tz`) and puts the run's own state back straight after the last, so everything after it is drawn as before. An edit that keeps the piece's words keeps the page's own codes and spacing numbers for every stretch whose font is not asked to change; a retyped line is encoded afresh, with the spacing across a stretch boundary carried over. Still byte-safe: only the operators of the retyped piece change. (The first design wrote a state block and restored it per span; one mechanism now serves an in-place restyle, a retype and a font span.) A span's font is `Face(name)` or `LikeObject(n)`; `LikeObject` is the *preferred* font of its stretch, not a demand, so a letter its subset lacks lands in a neighbouring font (said) instead of refusing the edit. A span's `size` is the size drawn at, with a script's 58% already applied.
- Underline and strike: the spans carry the flags and **the engine computes the rules** from the codes it wrote (their exact advances, tracking included) and appends each as a filled rectangle after the text object, in a marked-content sequence `/PagifyRule <</Kind k>> BDC q … re f Q EMC`. The rule is centred on the face's own underline / strikeout position and thickness (from the font program PDFium holds for the run, the generic -0.1 / 0.3 em and 0.05 em only where there is none) and measured on a copy of the page, so a line kept to its width is measured after it is spread. Rules are found **by their mark in the page's own stream**, never by walking PDFium's path objects. A rewritten piece takes its own rules off (an edit that is not colour-only, and every removal), so a piece that is to stay underlined must say `underline` in its spans; a rule is a path object, so every object after it is renumbered. Text drawn at an angle cannot take a rule (refused before anything is written; such text cannot be picked for editing today in any case).
- **Reading back on pick**: `Session::run_text_looks` gives each run's text state from the stream — `Tc × Th × scale` for spacing (the exact inverse of what is written), `Th × 100` for width, `Ts × scale` for rise, size, colour — and the tagged rules matched to the piece they lie under. The box opens showing what the page has: a piece is a superscript or subscript only when both its size (0.58 of the line's) and its rise (+0.33 / −0.14 em) say so, and the page's font for a piece is remembered so an untouched piece is written in it. A piece whose operator cannot be followed is simply absent (it opens in the look it always had). Rules drawn by other programs (untagged paths) are not recognised. Applying: a box nobody styled whose lines each have one look is written exactly as before, no spans; any other box is written through the paragraph path, a look-only line piece by piece in place (nothing moves), a retyped line as a stretch per look. **If the engine refuses the looks of a box nobody styled** (it only opened with several looks to a line), the edit is tried again as it was written before characters had looks, and the person is told the lines are in one look; a box somebody styled is refused as a whole and changes nothing.
- **Phase 4**: each line's position from `lay_out` through the existing `TextStyle::at` — a retyped line is written there, a line whose words did not change is moved there with `MoveObjects` — and list markers written as text objects of their own at the line's start, tagged so the next edit finds and replaces them.

### 4f. Fonts and refusals

- A span's face works as Bold/Italic do today: the family's face is registered (`pdf_core::text::register`) and embedded as a subset. On existing text the three-tier rule stands — the run's own font if it can spell the span, then a font already on the page, then an embedded one — and the panel says when the font changed.
- Anything the engine cannot follow is refused before anything is written, and the reason is said next to Apply, as today.

## 5. Out of scope (deliberate)

- Moving an underline together with its words in Edit Object: they are two objects (a marquee takes both); the shared tag makes linking them possible later.
- A list over several of the page's paragraphs in one edit: the Edit Text box opens one paragraph at a time.
- Re-opening a placed new box as rich text: placed boxes are page text, as today.
- Recognising underlines other programs drew.
- Right-to-left and complex scripts inside mixed-style lines: shaping works per span; a span boundary inside a joined Arabic word is not designed for.

## 6. Risks

1. **The selection when the panel is clicked.** egui drops it from `TextEditState` when the box loses the focus, so the box keeps its own copy; phase 1's tests prove the panel acts on it.
2. **The new box's preview font.** Today's wraps in egui's default font. Each run's face must be registered with egui first, or the rows on screen are not the rows on the page — phase 1.
3. **Existing text and subset fonts.** A bold or superscript span often needs letters the subset lacks, so it is written in a borrowed or embedded face (said). Pages the engine cannot follow refuse, as retype does.
4. **Preview cost.** Shaping every run on every frame is avoided by laying out again only when the text or a look changes.

## 7. Testing

- **`pagify_shell`, no window**: typing inside a run, at its edge and across runs; a paste over a range; merging runs that look the same; `look_of` mixed; every layout rule against the fixed-width stand-in.
- **`pdf_core`, hermetic pages (phases 3–4)**: spans written as `Tf`/`Ts`/`Tc`/`Tz`/`rg` that PDFium reads back at the right size, rise and position; the run's neighbours unchanged byte for byte; tagged rules found and replaced on a re-edit; qpdf clean; a refusal leaves the page untouched.
- **`pagify_app`, `egui_kittest`**: select a range and press U — the preview has an underlined section; Add to Page writes the words and a rule where the preview drew them, in one undo; the panel shows *mixed*; typing continues the look. Phase 0: the Italic button's letter tessellates with UVs inside the font atlas (fails with today's code).
- **Real files, skipped when absent**: `VIVO mini.pdf` and the datasheet in phases 3–4, with a render saved and looked at.
- House rules throughout: tests first, each behaviour mutated to prove its test can fail, full suites with `PAGIFY_PDFIUM_LIB` and `PAGIFY_QPDF` set and `--no-fail-fast` before each publish.

## 8. Rollout

| Phase | Ships | New boxes | Edit Text box |
|---|---|---|---|
| 0 | The Italic letter drawn with `TextFormat::italics`, `shape_to_sheared` deleted; the font field always names a font | a named font from the start (Arial / Montserrat-Regular) | names the page's font |
| 1 | The model, the selection-aware panel, character styles | font, size, colour, B, I, U, S, super/sub, letter spacing, width, baseline — per selection | unchanged (whole box, as today) |
| 2 | Paragraph styles | alignment and justify, indent, lists, line and paragraph spacing — per paragraph | unchanged |
| 3 | Engine spans; reading the page's styles back | — | every character style, per selection |
| 4 | Line positions through `at`; markers as text | — | alignment, justify, indent, lists, spacing |

Each phase is its own build, published and recorded in the architecture document. Each gets its own implementation plan when it starts; the first covers phases 0 and 1.
