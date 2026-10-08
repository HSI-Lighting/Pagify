# Pagify Desktop — Design Review and Refactor Plan

- **Date:** 2026-10-06
- **Baseline:** `pagify-desktop-windows` @ `bcc562c` ("Pagify Desktop 0.1.46")
- **Scope:** `desktop/crates/pagify_app`, `desktop/crates/pagify_shell`
- **Audience:** the coding agent that will implement this. Line numbers were
  measured at `bcc562c` and will drift as you work — grep for the named item
  rather than trusting the number.

---

## Status (2026-10-07, branch `farzad-debug`)

**Done.** Phase 0: clippy compiles, fixture and `.gitattributes` fixes in, check
commands documented in `desktop/README.md` (no CI workflow yet). Phase 1:
`main.rs` ≈ 15.4k lines (from 46.5k), all 22.9k test lines out in files beside
their code, and the function split — `ui()`, `act()`, `verbs::parse`, `resolve`,
`draw_main_area`, `canvas::interact`, `draw_pages`, `show_page_context_menu`,
`draw_ribbon`, `Tab::buttons` and the big `act_*` arms — is done on the
`farzad-debug` line. The parallel `pagify-windows-refactor` line added
`ToolId`/`Command`-carrying ribbon tables, `ToolEffect`,
`Tool::on_click`/`on_pointer`/`on_cancel`/`preview`, transition tests and bug
fixes, then merged (`82d44d9`, 0.1.50). That merge left `ui` inline again with
its extracted methods dead; repair commits `3fc46fa` (ui re-split over the
merged body) and `3ec8898` (per-tab consts restored) put it back. Clippy's
measure (non-comment lines) at 2026-10-08 after the repair: the largest
functions left are `pick_text_run_traced` 205,
`signature_extract::extract_signature` 196, `draw_signature_list` 184,
`draw_spell_check` 170, `replace_outlined_word` 152 and `blocks::segment` 151,
plus shell test helpers (178/173/159); 46 functions are still over 100 lines.
Phase 2: one `Tool` enum and one `ArmedTool` (`PendingKind`/`Pending` deleted),
with click/pointer/cancel/preview transitions in `tool.rs` and the thin
`picking.rs` apply path; the state fields (`editing_run`, `grab`, `handle`,
selections) are still on `PagifyApp`/`DocTab`. Phase 3: `DocCaches`, one
invalidation entry point. Phase 4a: `paragraph_lines`, `spelling`, editor
arithmetic in `pagify_shell` (`editor.rs`).

**Remaining.** `DocTab` (≈84 fields) and `PagifyApp` (≈53) state sub-structs,
against the §5 targets of 25/15 — including folding `editing_run`, `grab`,
`handle` and the selections into `Tool`; the rest of `main.rs` (≈16.9k lines)
and the >2k-line test files (`lock_wiring_tests` 7.5k, `ui_tests` 4.5k);
keyboard transitions (`Tool::on_key` does not exist yet — clicks, pointer,
cancel and preview do); the shell `Effect` executor for commands (tool-level
`ToolEffect` exists, command-level does not); the `blocks.rs`/`block_input.rs`
(5.3k lines) and `session.rs` (1.8k/116 methods) splits; `clippy.toml` and CI
gates.

**Verification note.** On this Linux machine the suite is red at exactly 12
pre-existing, machine-specific tests (3 `hub`, 4 `lock_wiring`, 5
`paragraph_apply`), which fail identically at the refactor's base commit
`e3585d1`; `instance::tests::a_child_process_does_not_keep_the_lock_alive` is
flaky under parallel load and passes in isolation. Green on the owner's
Windows machine has not been re-checked and is required before release.

---

## 0. How to use this document

The review found that the desktop app is not badly written line-by-line — it is
badly *bounded*. Every feature has been added into the same two structs and the
same file, and the cost is now paid on every change: a one-line feature touches
a 46,000-line file, an 821-line dispatch function and a 959-line frame function.

The refactor is deliberately staged so the tree is green after every step:

1. **Phase 0** — make the checks meaningful (clippy currently fails).
2. **Phase 1** — split `main.rs` into modules, behaviour-free. **Do this first
   and stop adding code to `main.rs`.**
3. **Phase 2** — replace scattered tool state with one `ToolState` machine.
4. **Phase 3** — replace eight hand-cleared caches with one cache registry.
5. **Phase 4** — move pure logic down into `pagify_shell` and turn command
   dispatch into an `Effect`-returning executor.

Each phase has acceptance criteria. Do not start a phase until the previous
phase's criteria pass. Every step must be behaviour-preserving: no user-visible
change, no test deleted, no test weakened.

---

## 1. Baseline measurements (reproduce before changing anything)

| Metric | Value at `bcc562c` |
|---|---|
| `pagify_app/src/main.rs` | **46,483 lines** (23,588 production + 22,895 test lines in 3 `#[cfg(test)]` modules) |
| `pagify_app` total | ~51,000 lines |
| `PagifyApp` fields | 53 |
| `DocTab` fields | 93 |
| `Doc` fields | ~16, of which 8 are caches |
| Production `self.tab_mut()` / `self.tab()` | 972 / 188 |
| `Verb` variants / `verbs.rs::parse` | 95 / 406 lines |
| `PendingKind` variants | 17, matched by 6 parallel methods |
| clippy functions > 100 lines | 42 (26 in `pagify_app`) |
| Worst functions (clippy count) | `ui` 959, `act` 821, `interact` 422, `draw_pages` 249, `Tab::buttons` 241, `resolve` 229, `pick_text_run_traced` 205, `draw_run_editor` 200, `verbs::parse` 406, `signature_extract::extract_signature` 196 |
| Growth | 29,045 lines (2026-09-28) → 46,483 (2026-10-06): **+17.4k lines in 8 days** |

Reproduction (from `desktop/`):

```bash
find crates -name '*.rs' -exec wc -l {} + | sort -rn | head -20
cargo clippy --workspace --all-targets --message-format short -- \
  -W clippy::too_many_lines -W clippy::too_many_arguments \
  -W clippy::type_complexity -W clippy::large_enum_variant \
  > /tmp/clippy-baseline.txt 2>&1
grep -cE 'this function has too many lines' /tmp/clippy-baseline.txt
```

**Known check failure at baseline:** `cargo clippy --all-targets` does not
compile `crates/pagify_shell/tests/blocks_review_fuzz.rs` because line 1536
contains a boolean expression clippy proves is always `false`
(`clippy::logic_bug`). A fuzz assertion that can never fire is worse than no
assertion. Fix or delete it in Phase 0.

---

## 2. Root causes

### 2.1 God object + god file

**Evidence**

- `main.rs:1643` — `struct PagifyApp` (53 fields: tabs, recorder, libraries,
  recogniser, clipboards, render worker, hub window state, update state …).
- `main.rs:1101` — `struct DocTab` (93 fields: view state, scroll/anchor state,
  selection state, find state, tool state, dialog state, caches, bookmarks …).
- `main.rs:923` — `struct Doc` with 8 derived caches inside it.
- 972 production sites reach through `self.tab_mut()`; a further 188 through
  `self.tab()`.
- Growth: +17.4k lines in the 8 days before this review. There is no ceiling.

**Why this produces spaghetti:** there is no ownership boundary anywhere. Any
method can touch any field of the active tab; any feature is implemented by
adding a field to one of two structs and a branch to one of three functions.
Nothing can be understood, tested or changed in isolation, and merge conflicts
land in the same two places every time.

**Required change:** split state by lifetime and ownership (Phase 1), and cap
it (Section 5 invariants).

### 2.2 Tool state sprawl — there is no state machine

**Evidence**

- `pending: Option<Pending>` plus `markup_armed`, `link_armed`,
  `match_properties_armed`, `match_properties_sample`, `pending_link`,
  `pending_article_box`, `paste_ghost`, `new_text_box`, `editing_run`,
  `selected_image`, `right_clicked_at`, `right_click_text_actions`, `handle`,
  `grab` … (`main.rs:1128–1300`).
- `main.rs:681–921`: six parallel `match` blocks over all 17 `PendingKind`
  variants (`wants`, `prompt`, `repeats`, `command`, `wants_snapping`,
  `ends_on_enter`).
- Transitions are spread across `arm` (`main.rs:12461`), `arm_without_saying`
  (`12478`), `take_pick` (`18243`), `resolve` (`18285`),
  `leave_editor_by_click` (`16490`), `interact` (`22567`) and `act`
  (`10587`).

**Why this produces spaghetti:** adding one tool requires edits in ~10
functions plus up to 6 new match arms, each in a different place. Tool
behaviour cannot be reasoned about or tested as a unit because there is no
unit — only distributed flags. The comments in the code are full of
interaction bugs caused by exactly this (two-click picks, tools swallowing the
next click, re-arm races).

**Required change:** one `ToolState` enum owning its sub-state, with pure
transition functions (Phase 2).

### 2.3 Domain logic lives in the UI crate

**Evidence** — all of these are ui-free (`egui` is not referenced) yet live in
`pagify_app`:

- `main.rs`: `reveal_axis` (1076), `run_editor_font_size` / `run_editor_box_grow`
  / `run_editor_glyph_size` (23301–23396), `paragraph_should_justify` (23396),
  `justify_gaps` (23423), `editor_sections` (23453), `append_sectioned`
  (23475), `editor_layout_job` (23510), `join_paragraph_lines` (23583),
  `is_hyphen_mark` (23625), `wrap_hyphen_marks` (23649), `fix_extracted_text`
  (23686), `hyphens_before_drawn_lines` (23721), `majority_look` (23762),
  `tabs_that_fit` (24077).
- Whole modules: `paragraph_lines.rs` (1,142 lines, 0 `egui` references),
  `spelling.rs` (762 lines, 0 `egui`), `system_fonts.rs` (68 lines, 0
  `egui` — platform-specific but not UI).
- Production `pdf_core::` references in `pagify_app`: 394 vs 110
  `pagify_shell::`. The shell is bypassed more than three times as often as it
  is used.

**Why this produces spaghetti:** `pagify_shell`'s own charter
(`pagify_shell/src/lib.rs:12`) says *"If logic can be tested without a window,
it must live here."* The app crate now contains logic that can only be tested
through the app's test modules — which is why those modules are 22,895 lines
long. It also duplicates concerns the shell already owns (`blocks.rs` /
`block_input.rs`).

**Required change:** move pure logic into `pagify_shell` (or a new
`pagify_logic`-style module inside it), leaving `pagify_app` as egui glue
(Phase 4, but move functions opportunistically whenever Phase 1 touches them).

### 2.4 Hand-rolled caches with manual invalidation

**Evidence**

- `Doc` holds `textures`, `thumbs`, `detail`, `locked`, `page_blocks`,
  `sampling`, `weight`, `rect_page` (`main.rs:946–993`), each with its own key
  tuple (`render_epoch`, undo generation, page, zoom step).
- All are cleared by one method, `rendered_is_stale` (`main.rs:1007`), whose
  comment records the failure mode: *"Thirteen places cleared `textures` after
  an edit and two cleared `thumbs`, so almost every edit left a stale
  thumbnail."*
- `DocTab` adds more: `text`, `layers`, `internal_links`, `foreign`,
  `bookmarked_pages`, `joined_groups`.

**Why this produces spaghetti:** every edit path must remember the full cache
matrix or the UI shows stale content; correctness lives in comments and
discipline rather than in one API. This is where user-reported bugs keep
coming from.

**Required change:** one cache registry keyed by `(page, epoch, zoom step)`
with a single `invalidate(...)` entry point (Phase 3).

### 2.5 Command dispatch is two giant switchboards with no boundary

**Evidence**

- `Verb` has 95 variants (`verbs.rs:145`); `verbs.rs::parse` is 406 lines of
  string parsing with alias tables; `PagifyApp::act` (`main.rs:10587`) is an
  821-line `match` that performs PDFium work, file IO, dialog opening and
  error-message construction inline.
- Command *meaning* is therefore written twice (once to parse, once to
  execute), and the UI layer knows which engine calls each command needs.

**Why this produces spaghetti:** ribbon buttons call `act` directly with a
`Verb`, command-box text goes through `parse`, and `interact`/tool completion
calls `act` again — three entry points into one 821-line switch. There is no
place where "what a command means" can be tested without a window, and no
place where "what the UI should change" is stated independently of how.

**Required change:** a shell-level `execute(verb, ctx) -> Vec<Effect>` (or an
`Action` enum) that owns command semantics; `pagify_app` maps `Effect`s
(`Say`, `ScrollTo`, `OpenDialog`, `Refresh`, `AskUnsaved`, `Quit`, …) to UI
operations. Split `parse` by domain into small registered parsers; split `act`
by domain into handler modules (Phase 4).

### 2.6 Tests are welded into `main.rs` — which is what prevents the fix

**Evidence**

- 22,895 of `main.rs`'s 46,483 lines are tests in **3** `#[cfg(test)]`
  modules: `main.rs:59` (`tests_support`), `main.rs:24142` (`mod tests`,
  ~18.7k lines), plus helpers at `42882–45250`.
- Tests reach private internals of `PagifyApp`/`DocTab` directly.
- `hub_tests.rs` (1,194 lines) already demonstrates the better pattern: a
  sibling file testing an extracted module.

**Why this produces spaghetti:** every code move forces a simultaneous,
mechanical test move in the same commit. That cost is why the file only ever
grows: "add another arm and another test at the bottom" is always cheaper than
"extract a module". The test suite is functionally excellent and is now the
main structural liability.

**Required change:** as each module extracts (Phase 1), its tests move with
it; `tests_support` becomes a small `pub(crate)` test-helper module. Never
split tests in a separate commit from the code they exercise.

### 2.7 The shell has smaller versions of the same disease

**Evidence**

- `verbs.rs::parse` 406 lines.
- `blocks.rs` (2,292) + `block_input.rs` (2,985) = 5,277 lines for paragraph
  detection/input.
- `session.rs`: 1,817 lines, 116 methods.
- `signature_extract.rs::extract_signature` 196 lines.

These are cohesive by comparison — the shell split is real — but the same
remedy applies at smaller scale: per-domain modules, one function one job.

### 2.8 Concrete hygiene defects (fix in Phase 0)

1. `crates/pagify_shell/tests/blocks_review_fuzz.rs:1536` — clippy
   `logic_bug`: boolean expression is always `false`, and clippy refuses to
   compile the test target. Fix the assertion or delete the dead check.
2. `main.rs:23380` — empty line after doc comment.
3. 65 copies of the pattern `say_error("nothing open."); return;`. Add
   `fn doc_mut(&mut self) -> Option<&mut Doc>` (or shell equivalent) and use it.
4. Tool identity is stringly typed: `type Tool = (&str, &str, &str)`
   (`main.rs:2365`), `PendingKind::command() -> &'static str` (`main.rs:845`),
   `ribbon_click(&str)` (`main.rs:3538`). The guard tests in
   `tests/command_box.rs` exist because the type system is not carrying this
   weight. Introduce a `ToolId` enum in Phase 2 and make ribbon tables and
   `PendingKind::command` use it.

---

## 3. Target architecture

### 3.1 `pagify_app` module map

```
crates/pagify_app/src/
  main.rs            eframe wiring only; PagifyApp root (< ~400 lines)
  workspace.rs       Workspace { tabs, active_tab, shared } + tab lifecycle
  shared.rs          Shared libraries: recents, signatures, snippets, fonts,
                     session log, clipboards (what hub.rs::Shared lends today)
  doc_tab/
    mod.rs           DocTab { doc, view, tools, edit, panels, caches }
    view.rs          ViewState: page, zoom, rotation, scroll, hover, reveal
    tools.rs         ToolState + transitions (pure where possible)
    edit.rs          run/paragraph editor state and commit paths
    panels.rs        dialogs, menus, rails, find, spell, bookmarks
    caches.rs        DocCaches: one registry, one invalidation API
  canvas.rs          page drawing, pointer routing, snap/guides painting
  ribbon.rs          tab/tool tables (data) + ribbon widgets
  menus.rs           context menus
  dispatch.rs        Verb/Effect application and error saying
  hub.rs             (exists) multi-window
  instance.rs        (exists) single-instance handover
  overlay.rs         (exists)
  home.rs            (exists)
  theme.rs           (exists)
  text_style_panel.rs (exists)
  spelling/          (moved to shell in Phase 4; platform glue stays)
  paragraph_lines.rs (moved to shell in Phase 4)
  tests_support.rs   small shared test helpers (cfg(test))
```

Rules:

- A module owns its state; other modules reach it through methods, not fields.
- `DocTab` fields become four sub-structs; `PagifyApp` keeps only workspace +
  shared + window concerns.
- `hub.rs` is the reference extraction: state, pure `decide`/`plan`
  functions, tests beside it. Copy that pattern.

### 3.2 `ToolState` (Phase 2 sketch)

```rust
pub(crate) enum Tool {
    None,
    Draw(DrawTool),            // line, circle, rectangle, arrow, polyline, spline
    PendingText { text: String },
    PlaceText,
    PlaceImage { path: PathBuf },
    Modify(ModifyTool),        // move/copy/rotate/scale/mirror/trim/…
    Measure(MeasureTool),
    Calibrate { .. },
    Redact, Whiteout, Lock, ArticleBox,
    Fill(FillMark),
    Signature, SignRectangle, SignLine,
    EraseMark,
    PickText,
    Link,                      // replaces link_armed + pending_link
    MatchProperties { sample: Option<MatchPropertiesSample> }, // replaces flag+sample
    PasteGhost(PasteGhost),    // replaces paste_ghost
    EditingRun(EditingRun),    // replaces editing_run
}
```

- One function decides each event:
  `on_click(&mut self, at, hit) -> ToolEffect`, `on_key(..)`,
  `on_pointer(..)`, `on_cancel(..)`.
- Queries (`needs_points`, `prompt`, `repeats`, `snapping`, `ribbon_id`)
  become methods on the payloads or one small trait — not six global switches.
- Tests: a `tools.rs` unit-test module drives transitions directly, no window,
  including the currently-commented interaction rules (click-away applies an
  editor; placing a signature disarms; a failed pick re-arms quietly).

### 3.3 Cache registry (Phase 3 sketch)

```rust
struct Epoch { render: u64, undo: u64 }
struct DocCaches {
    textures: HashMap<TextureKey, TextureHandle>,
    thumbs:   HashMap<usize, TextureHandle>,
    detail:   Option<DetailTile>,
    locked:   Option<(usize, Vec<LockedItem>)>,
    blocks:   Option<(usize, Epoch, Rc<PageBlocks>)>,
    sampling: Option<(usize, Epoch, Rc<PageRaster>)>,
    weight:   Option<(usize, Epoch, PageScale)>,
    rect_page: Option<(usize, Epoch, Rc<PageBlocks>)>,
}
impl DocCaches {
    fn invalidate(&mut self, scope: Invalidate); // Document | Page(usize) | Zoom
}
```

- `Doc::rendered_is_stale` becomes `self.epoch.render += 1;
  self.caches.invalidate(Invalidate::Document);` — the field list lives in one
  place and a new cache cannot be forgotten by an edit path.
- The cache API is the only way to obtain a derived value (`fn locked(&mut
  self, page) -> Option<&[LockedItem]>` style), so a caller cannot read stale
  data by skipping invalidation.

### 3.4 Command execution (Phase 4 sketch)

```rust
// pagify_shell
pub enum Effect {
    Say(Severity, String),
    ScrollTo { page: usize, rect: Option<Rect> },
    OpenFileDialog,
    SavePassphrasePrompt,
    RefreshDocument,
    AskUnsaved { intent: Closing },
    Quit { force: bool },
    // … one variant per UI-visible consequence, not per command
}
pub fn execute(verb: &Verb, doc: Option<&mut DocumentState>, ctx: &mut CommandCtx)
    -> Vec<Effect>;
```

- `parse` splits into `verbs/document.rs`, `verbs/edit.rs`,
  `verbs/organize.rs`, `verbs/draw.rs`, `verbs/review.rs`, `verbs/measure.rs`,
  `verbs/sign.rs`, each registering its own names/aliases; one test asserts no
  duplicate registrations (this is what `DELIBERATE_OVERRIDES`/`REFUSED`
  already protect — keep those guarantees).
- `act` in the app becomes a ~50-line `match effect { … }` loop, split into
  `dispatch/document.rs`, `dispatch/edit.rs`, etc. if it grows.

---

## 4. Phases and acceptance criteria

### Phase 0 — Make the checks meaningful (do first, ~1 day)

Tasks:

1. Fix `blocks_review_fuzz.rs:1536` so the test target compiles under clippy.
   If the expression was meant to assert something, write the real assertion;
   if it was dead, delete it and say so in the commit message.
2. Fix `main.rs:23380` (empty line after doc comment).
3. Add a CI job (or document the command in `desktop/README.md` if CI is not
   yet set up) running, from `desktop/`:
   `cargo clippy --workspace --all-targets -- -D warnings` and
   `cargo test --workspace`.
4. Optionally add `#![deny(clippy::too_many_lines)]`-style gates per file only
   after Phase 1 (see Section 5); do not enable repo-wide yet — it would fail.
5. Record the Section 1 table in the PR description as the baseline.

Acceptance: `cargo clippy --workspace --all-targets -- -D warnings` succeeds;
`cargo test --workspace` succeeds; no behaviour changes.

### Phase 1 — Split `main.rs` into modules, no behaviour change (1–2 weeks)

The goal is to end Phase 1 with `main.rs` under ~2,000 lines (ideally ~400) and
every other file under 2,000 lines, with all 22,895 test lines moved along
with the code they test.

Order of extraction (each is one commit or one small PR; green after each):

1. **`tests_support`** (`main.rs:59`) → `tests_support.rs` as
   `#[cfg(test)] pub(crate) mod tests_support;`.
2. **`workspace.rs`** — `PagifyApp` fields that are workspace-wide:
   `tabs`, `active_tab`, `recorder`, libraries, `clipboards`, `update_*`.
   Methods: `tab`/`tab_mut`, `open`, `open_with`, `close_tab`, `remove_tab`,
   `bring_active_tab_into_the_strip`, `leave`, `exit_program`.
3. **`doc_tab/view.rs`** — `page`, `zoom`, `rotation`, `scroll_*`,
   `last_view`, `view`, `anchor_offset`, `hover_view`, `viewport_rect`,
   `canvas_pt`, `settling`, `zoom_basis`, `last_drawn_zoom`,
   `zoom_changed_at`. Methods: `resolved_zoom`, `set_zoom`, `go_to`,
   `rotate_view`, zoom-anchor code in `draw_pages` (`main.rs:20656`) —
   extract the anchor arithmetic as a pure function taking numbers, so it is
   unit-testable (this also satisfies 2.3).
4. **`doc_tab/tools.rs`** — `pending`, `markup_armed`, `link_armed`,
   `pending_link`, `match_properties_*`, `pending_article_box`, `paste_ghost`,
   `new_text_box`, `editing_run`, `grab`, `handle`, `selected_*`, plus
   `PendingKind` and all six query methods (`main.rs:681–921`). Move
   `arm`/`arm_without_saying`/`take_pick`/`resolve`/`leave_editor_by_click`
   bodies here, still in their current shape — Phase 2 will redesign them.
5. **`doc_tab/edit.rs`** — editor build/apply paths:
   `build_editor_from_lines` (14914), `apply_one_edit` (16012),
   `apply_paragraph_edit` (16283), `replace_outlined_word` (16793),
   `pick_text_run_traced` (14443), run/paragraph helpers in `main.rs`
   23301–23784.
6. **`doc_tab/panels.rs`** — `draw_passcode_dialog` (13253),
   `draw_signature_pad` (13132), `draw_signature_list` (12927),
   `draw_snippet_list` (12801), `draw_find_replace` (8063),
   `draw_spell_check` (8411), `draw_bookmark_panel`, `draw_link_prompt`,
   `draw_extract_dialog`, `draw_article_box_prompt`, `draw_update_prompt`,
   `draw_shape_properties` (22077), `draw_detail_overlay`.
7. **`canvas.rs`** — `draw_pages` (20656), `draw_thumbnail_cell` (9936),
   `draw_organize_grid`, `draw_object_selection` (7066),
   `draw_pending_preview` (21140), `draw_move_guides`, `draw_lock_badges`
   (22341), `interact` (22567), `interact_objects` (6106),
   `interact_signatures`, `interact_placed_images`, `paint_signature`,
   `paint_arrow`.
8. **`ribbon.rs`** — `Tab` enum and its tables (`main.rs:3006–3378`),
   `Tool` type, `tool_button`, `ribbon_click`, `history_toggle`,
   `more_tools_button`, `ribbon_overflow_at`, `doc_tab_*` helpers
   (23901–24116), `hub::TabButton` usage.
9. **`dispatch.rs`** — `act` (10587) and the `say_*` plumbing, until Phase 4
   moves its semantics to the shell. Also `draw_*` home/menu leftovers.
10. Finish when `main.rs` contains only: `mod` declarations, `main()`,
    `PagifyApp` root struct, `impl eframe::App`, and thin delegating methods.

Rules for Phase 1:

- **Move tests with code.** A commit that moves a function but leaves its
  tests in `main.rs` is not accepted.
- Private items across modules: use `pub(crate)`, not `pub`. Do not widen
  visibility beyond the crate.
- Do not "improve while moving" beyond mechanical edits (imports, `self.`
  qualification). Design changes happen in Phases 2–4.
- Keep comments with their code — this project's comment culture is a asset;
  do not summarise or delete rationale comments during the move.
- After each extraction: `cargo test --workspace` green, `cargo clippy
  --workspace --all-targets` no new warnings.

Acceptance:

- `wc -l` shows no `.rs` file over 2,000 lines except shell test data files
  (justify in the PR if any remain).
- `PagifyApp` has ≤ 15 fields; `DocTab` delegates to four sub-structs.
- Test count and test names are unchanged (`cargo test` output diff).

### Phase 2 — One `ToolState` machine (2–3 weeks)

Tasks:

1. Introduce `Tool` (Section 3.2), one field replacing at least:
   `pending`, `markup_armed`, `link_armed`, `pending_link`,
   `match_properties_armed`, `match_properties_sample`,
   `pending_article_box`, `paste_ghost`, `new_text_box`, `editing_run`.
2. Convert the six parallel `PendingKind` switches to methods on the payload
   types. Delete `PendingKind` once empty.
3. Route every pointer/key event through `on_*` transitions returning a
   `ToolEffect`; `canvas::interact` and `dispatch::verb` apply the effect.
4. Introduce `ToolId` (2.8.4) for ribbon tables, `Tool::ribbon_id()`, and the
   command box's "which button is lit" query.
5. Write transition tests in `tools.rs` covering the rules currently encoded
   in comments:
   - click-away from an open editor applies it, and does not double-handle the
     same click (the "two identical errors" bug, `main.rs:22591–22633`);
   - placing a signature/picture/text box disarms the tool;
   - a failed pick re-arms quietly;
   - `Escape` cancels without applying;
   - polyline/spline/area end on Enter and re-arm per `repeats`.
6. Property test: for every `Tool` variant, `on_cancel` returns to
   `Tool::None` and leaves no armed ribbon button (`Tool::ribbon_id() == None`).

Acceptance: no `match` in the codebase switches over tool kinds outside
`tools.rs`; tool transitions have direct unit tests; all pre-existing
interaction tests pass unchanged.

### Phase 3 — One cache registry (≈1 week)

Tasks:

1. Move the eight `Doc` caches into `DocCaches` with private fields
   (Section 3.3); remove direct field access from outside the module.
2. Replace `rendered_is_stale` with `Caches::invalidate(scope)`; delete the
   field list from the call site.
3. Make every cache read go through an accessor that checks its key first.
4. Delete `DocTab::layers`/`text`/`internal_links` in favour of the registry
   (same keys: page + epoch).
5. Add a test that an edit followed by a read returns fresh data for every
   cache kind (table-driven; the stale-thumbnail audit bug is the regression
   case).

Acceptance: exactly one function clears caches; adding a new cache requires
touching only `caches.rs`; the thumbnail-staleness regression test passes.

### Phase 4 — Shell purity and effect-based dispatch (ongoing)

Tasks, in priority order:

1. Move ui-free logic to `pagify_shell` (2.3): the `run_editor_*` maths,
   `paragraph_should_justify`, `justify_gaps`, `editor_sections`,
   `join_paragraph_lines`, `wrap_hyphen_marks`, `fix_extracted_text`,
   `hyphens_before_drawn_lines`, `majority_look`, `reveal_axis`,
   `tabs_that_fit`; whole modules `paragraph_lines.rs` (→ shell
   `editor.rs`/`blocks` support), `spelling.rs` (→ `shell/spelling.rs`,
   keeping the dictionary `include_str!` assets beside it), `system_fonts.rs`
   (stays in app — platform IO, no logic).
2. Split `verbs.rs::parse` by domain, one registration point per domain, and
   keep a single test that fails on duplicate/alias collisions.
3. Introduce `Effect` + `execute` in the shell (Section 3.4); turn `act` into
   an effect applier. Verify with the existing `command_box.rs` and
   `hub_tests.rs` suites; add an effect-level test per command group that
   asserts on the returned `Effect`s with no window.
4. Split `blocks.rs`/`block_input.rs` into per-concern modules; split
   `session.rs` by operation family (`session/open.rs`, `session/save.rs`,
   `session/organize.rs`, …) with `Session` as a thin handle.
5. Delete `pagify_app`'s duplicate spelling/line logic once the shell versions
   are wired; the app should contain no algorithm that does not touch egui.

Acceptance: `rg -l 'egui' crates/pagify_shell/src` is empty (it already is —
keep it); `rg 'pub fn' crates/pagify_app/src` exposes no parser/layout
algorithm; dispatch tests assert effects, not UI state.

---

## 5. Invariants and gates (enforce in CI after Phase 1)

| Invariant | Target | Check |
|---|---|---|
| Max file size | 2,000 lines | `find crates -name '*.rs' -exec wc -l {} +` |
| Max function size | 150 lines | clippy `too_many_lines` with `too-many-lines-threshold = 150` in `clippy.toml` (or `#![deny]` per module) |
| `PagifyApp` fields | ≤ 15 | code review |
| `DocTab` fields | ≤ 25 (sub-struct delegates) | code review |
| No new code in >2k-line files | hard rule | review |
| clippy | `-D warnings` clean | CI |
| Shell purity | no egui in `pagify_shell` | `rg egui crates/pagify_shell/src` |
| Tests | all pass, count non-decreasing | CI |
| Pure logic | lives in shell | review + the "ui-free" test pattern |

Suggested `clippy.toml` (add after Phase 1):

```toml
too-many-lines-threshold = 150
too-many-arguments-threshold = 7
type-complexity-threshold = 250
```

Do not add exclusions for existing offenders before they are refactored; a
temporary allowlist is acceptable only with a tracking comment naming the
phase that removes it.

---

## 6. Refactor rules (apply to every phase)

1. **Behaviour-preserving by default.** User-visible changes ship separately.
2. **Green between commits.** Never leave `cargo test --workspace` failing at
   the end of a commit; use `#[cfg(test)]` gates if a step is unavoidably
   partial.
3. **Tests move with code.** Same commit, always.
4. **One extraction per commit.** Small diffs are reviewable; a 9,000-line
   move PR is not.
5. **Preserve comments and rationale.** The codebase's long "why" comments are
   the best documentation it has. Move them, do not rewrite them; update line
   references only.
6. **No new dependencies** for the refactor itself (no `thiserror`, no DI
   framework). Plain Rust enums, structs and functions.
7. **Keep the engine contracts.** `Session` stays the only PDFium entry point
   through `pdf_core::registry`; do not move engine calls into the UI layer
   while "simplifying".
8. **Public API discipline.** `pagify_shell` is consumed by `pagify_app` only;
   new shell items are `pub` only when the app needs them.

---

## 7. Verification commands

From `desktop/`:

```bash
cargo test --workspace                     # must stay green at every commit
cargo clippy --workspace --all-targets -- -D warnings
find crates -name '*.rs' -exec wc -l {} + | sort -rn | head -15
grep -cE 'this function has too many lines' /tmp/clippy-after.txt
```

From the repo root (engine tests need a PDFium; see `desktop/README.md`):

```bash
PAGIFY_PDFIUM_LIB=$PWD/desktop/third_party/pdfium/pdfium-linux-x64/lib/libpdfium.so \
  cargo test --manifest-path rust/pdf_core/Cargo.toml
```

Manual smoke test after any UI-touching phase: open a fixture, draw a line,
type text, edit a paragraph, place a signature, save, reopen — the same path
`desktop/README.md` describes.

---

## 8. Non-goals

- No feature work in this refactor.
- No rewrite of the rendering worker, the registry/session model, or the
  engine crates.
- No replacement of egui, no reactive framework, no async runtime.
- No test deletion to make numbers look better; test names and count are a
  tracked baseline.
- No change to `pagify_shell`'s dependency policy (it must stay ui-free).

---

## Appendix A — Evidence index (at `bcc562c`)

| Item | Location |
|---|---|
| `PagifyApp` | `main.rs:1643` |
| `DocTab` | `main.rs:1101` |
| `Doc` + caches | `main.rs:923–1021` |
| `PendingKind` + six switches | `main.rs:342–921` |
| `act` | `main.rs:10587` |
| `interact` | `main.rs:22567` |
| `draw_pages` | `main.rs:20656` |
| `resolve` | `main.rs:18285` |
| `arm` / `arm_without_saying` | `main.rs:12461` / `12478` |
| `take_pick` | `main.rs:18243` |
| `verbs.rs::parse` | `verbs.rs:865` |
| clippy logic bug | `tests/blocks_review_fuzz.rs:1536` |
| Shell charter | `pagify_shell/src/lib.rs:12` |
| Good extraction example | `pagify_app/src/hub.rs` and `hub_tests.rs` |
