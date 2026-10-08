# Pagify Desktop — Remaining Work (handoff for a Windows machine)

- **Date:** 2026-10-08
- **Branch:** `farzad-debug`, pushed to `origin` (20 commits ahead of
  `pagify-windows-refactor`, 0 behind its own upstream). Base history:
  `bcc562c` (review baseline) → `e3585d1` (refactor branch base, also on
  `pagify-desktop-windows`) → the review's phases → this branch.
- **Read first:** `desktop/docs/DESIGN_REVIEW.md` (the full review, phases,
  acceptance criteria and invariants) and `desktop/docs/ARCHITECTURE.md`
  (module map and current status). This file only covers what is *left* and
  how to do it without breaking things.

All commits: `git log --oneline origin/pagify-windows-refactor..farzad-debug`.

---

## 0. Verify before changing anything (Windows)

From `desktop\` (the commands documented in `README.md`'s Checks section):

```powershell
$env:PAGIFY_PDFIUM_LIB = "third_party\pdfium\pdfium-win-x64\bin\pdfium.dll"
cargo test -p pagify_app --release --bin pagify_app     # ~970 tests
cargo test -p pagify_shell --release                     # ~640 tests
cargo test -p pdf_core --release --manifest-path ..\rust\pdf_core\Cargo.toml --no-fail-fast
```

- **This branch has never been run green on Windows.** The only machine it
  has run on (Linux) is red at exactly 12 tests that also fail identically at
  `e3585d1`, plus two load-flaky tests. Establish the real baseline on the
  Windows machine **before** touching code, and record it.
- Expect these to possibly pass on Windows and fail on Linux (or vice versa):
  - `hub::tests::*` (3) — egui texture/layer debug asserts, environment-linked.
  - `lock_wiring_tests` (4) — font/paragraph behaviour after picking a run.
  - `paragraph_apply_tests` (5) — four of them look for real fixture files
    under `C:\Users\hsili\Downloads\…` ("neither real file is on this
    machine" elsewhere); the fifth asserts the old one-object-per-letter
    writing and fails at the base commit too.
  - `instance::tests::a_child_process_does_not_keep_the_lock_alive` and the
    two `spell_check_tests` timing tests flake under parallel load on Linux;
    they pass in isolation.
- clippy check (compiles clean; **not** `-D warnings` clean — do not add a
  blanket gate yet; ~195 warnings, 47 of them `too_many_lines`):

```powershell
cargo clippy --workspace --all-targets --message-format short -- `
  -W clippy::too_many_lines -W clippy::too_many_arguments `
  -W clippy::type_complexity -W clippy::large_enum_variant
```

- Keep `.gitattributes` committed (line-ending trap for binary fixtures — see
  `README.md`'s Checks section).

---

## 1. What is already done (do not redo)

- **Phase 0:** checks compile; fixture path and `.gitattributes` fixes; check
  commands documented.
- **Phase 1 (mostly):** `main.rs` 46,483 → ~15.5k lines; all 22.9k test lines
  moved beside their code; **all original monster functions decomposed**
  (`ui` 959, `act` 818, `verbs::parse` 406, `canvas::interact` 422,
  `draw_pages` 246, `Tab::buttons` 241, `picking::resolve` 233,
  `draw_main_area` 340, `show_page_context_menu` 166, `draw_ribbon` 155,
  `draw_command_bar` 181 → its history out, plus many act arms).
- **Phase 2 (state):** one `Tool` enum and one `ArmedTool` in `pending.rs`;
  `PendingKind`/`Pending` deleted; queries unified. Transition functions still
  live in `picking.rs` as `take_pick` + `resolve` + six `resolve_*` helpers.
- **Phase 3:** `DocCaches` in `caches.rs`, one `invalidate` entry point.
- **Phase 4a:** `paragraph_lines`, `spelling`, editor arithmetic
  (`pagify_shell/src/editor.rs`), `reveal_axis` moved to the shell.
- **Guard:** `parse_domain_tests::no_command_is_claimed_by_two_domain_parsers`
  in `verbs.rs` re-establishes the parse-domain collision guarantee.

---

## 2. What remains

### 2.1 Function decomposition (finish the ≤150-line invariant)

Regenerate the list any time with the clippy command above and:

```bash
grep -E 'this function has too many lines' clippy.txt | sort -t'(' -k2 -rn
```

Current >150 (clippy non-comment lines) — **shell test helpers only**:

| Function | Where | Size |
|---|---|---|
| `mode_bin`-area test helpers | `shell/tests/blocks_synthetic.rs` | 178 and 159 (plus several 100–125) |
| a sweep helper | `shell/tests/replace_lines_sweep.rs:364` | 173 |
| fuzz helpers | `shell/tests/blocks_review_fuzz.rs` | 136/121/112/103 |

No production function exceeds 150 lines. The nearest are `draw_ribbon` 147
(its action-row half is separable again), `draw_passcode_dialog` 145,
`main.rs:3728` 139, `draw_pages` 138, `blocks::segment` 136 and
`draw_signature_list` 136. Split test helpers only when touching those files.

Below 150 but still large if you want to keep going: `canvas.rs` 129/125/110/105/103/102,
`main.rs` 139/135/123/118/108/105/102/102/102, `panels.rs` 136/123/102/102,
`dispatch.rs` 121/106, `shell/tools.rs:164` 108, `workspace.rs:107` 107,
`edit.rs:552` 102, `canvas.rs:1277` 129.

### 2.2 Structural tier (the bigger jobs)

1. **`ToolId` — DONE** on the refactor line (`fe97c64`): the ribbon tables
  carry `Command::Verb(...)`/`ToolId` instead of bare strings, and
  `Tool::id()`/`ToolId` are in `tool.rs`. Keep the command-box string parser
  as the boundary where strings become ids.
2. **State sub-structs and folding into `Tool`.** `DocTab` has ~84 fields,
  `PagifyApp` ~53; targets are 25 and 15 (DESIGN_REVIEW §5). The tool-kind
  migration itself is finished — the remaining Phase 2 work is moving the
  *fields* `Tool` belongs with (`editing_run`, `new_text_box`, `paste_ghost`,
  `grab`, `handle`, `markup_armed`/`object_tool`, the selections) into `Tool`
  or a `ToolState`, then grouping the rest into `ViewState`/`EditState`/
  `PanelsState`. `Doc` already delegates caches.
3. **Event-returning transitions — DONE.** `ToolEffect` +
  `Tool::on_click`/`on_pointer`/`on_cancel`/`on_key`/`preview` exist in
  `tool.rs`; `canvas::draw_pending_preview`/`drag_stopped` delegate to them;
  the Enter/Escape rules live in `Tool::on_key`; both property tests are in
  `tool_transition_tests.rs` (cancel, and the `on_key` matrix).
4. **`Effect`-returning command executor (Phase 4b).** Move command semantics
  into the shell: `execute(verb, ...) -> Vec<Effect>` with `Effect` covering
  Say/ScrollTo/OpenDialog/Refresh/AskUnsaved/Quit; `pagify_app::dispatch`
  becomes an effect applier. `verbs::parse` is already split by domain.
5. **File splits — session done, blocks pending.** `shell/src/session.rs`
  (1,817 lines) is now `session/`: `mod.rs` ~1,300 (text/recognise, marks and
  sensitivity, history/undo, typing fonts still to come) plus `io.rs` (48),
  `render.rs` (117), `locking.rs` (125), `signing.rs` (218), `objects.rs` (66)
  — each a family of `impl Session` methods; shell suite green throughout.
  Remaining splits: `shell/src/blocks.rs` (2,292) and `block_input.rs` (2,985)
  into families the same way, and the big shell test files. Move behaviour-free,
  one module per commit.
6. **CI gates.** Only once the Windows suite is green: add a workflow running
  `cargo test` (Windows runner, with `PAGIFY_PDFIUM_LIB`) plus clippy without
  `-D warnings`; add `clippy.toml` (`too-many-lines-threshold = 150`,
  `too-many-arguments-threshold = 7`, `type-complexity-threshold = 250`) and
  gate new/changed files, not the whole tree.

### 2.3 Known traps and small follow-ups

- **Always re-check the enclosing condition when moving a block.** A recent
  extraction of the command-history block silently dropped its
  `if self.command_open` guard; three tests caught it (`g4_edit_error_tests` ×2,
  `ui_tests::an_error_leaves_the_history_shut…`). Keep the guard at the call
  site and run the suite after every slice.
- **`Tool::cancel_drops_checkpoint`** (`pending.rs`) preserves an old
  asymmetry: cancelling some kinds pops a markup undo step even with no
  checkpoint open. It is documented as a likely latent bug; fix it with a
  regression test, or keep it documented.
- **"nothing open." guards**: ~70 repeated `say_error("nothing open.")`
  sites across `main.rs`/`dispatch.rs`. A `doc_mut()` helper is tempting but
  the `let ... else` borrow must compile; only do it if a confirming pattern
  is found, otherwise leave it.
- **Merging parallel refactors re-verifies nothing by itself.** The
  `82d44d9` merge compiled and passed the suite while silently reverting the
  `ui` split and the per-tab consts: the extracted methods stayed in the file,
  dead, while the merged `ui` inlined everything again. After any merge,
  re-measure (`clippy` inventory), check that extracted methods actually have
  call sites, and re-run the docs comparison. Repairs: `3fc46fa`, `3ec8898`.
- **Untracked files, deliberately not committed:** `.obsidian/` and
  `desktop/docs/SECURITY_AUDIT.md`.
- **Merge topology:** `farzad-debug` is already merged into
  `pagify-windows-refactor` (`82d44d9`), which sits on
  `pagify-desktop-windows` @ `e3585d1`. Continue on the refactor branch;
  `farzad-debug` is history.

---

## 3. Rules (from DESIGN_REVIEW §6, condensed)

1. Behaviour-preserving by default; user-visible changes ship separately.
2. Green between commits; never leave the suite failing at a commit boundary.
3. Tests move with the code, in the same commit.
4. One extraction or one module move per commit; small diffs.
5. Preserve comments and rationale — this codebase's "why" comments are its
   best documentation.
6. No new dependencies for refactoring.
7. `pagify_shell` stays UI-free (`rg egui crates/pagify_shell/src` must stay
   empty); engine access stays behind `Session`/`pdf_core`.
8. Update the inventory in `ARCHITECTURE.md` / `DESIGN_REVIEW.md` whenever a
   phase lands, with clippy-measured numbers rather than estimates.

## 4. Suggested first day on Windows

1. Build, set `PAGIFY_PDFIUM_LIB`, run all three suites, and record the exact
   baseline (message + names for anything red).
2. If line-ending or fixture-path failures appear, fix those first — see
   `README.md`'s `.gitattributes` note.
3. Work §2.1 top-down, one function per commit, running
   `cargo test -p pagify_app --release --bin pagify_app` before committing.
4. Re-run the clippy inventory every few functions and keep the docs current.
5. When the suite is green on Windows, start §2.2 by folding the tool state
   fields into `Tool` (item 2), then the command `Effect` executor; `ToolId`
   and the click/pointer transitions are already done. Leave CI gates
   (`clippy.toml` + workflow) until last.
