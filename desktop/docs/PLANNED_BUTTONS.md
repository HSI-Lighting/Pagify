# The planned buttons — hand-off plan

**Date:** 2026-10-12 · **Branch:** `farzad-phase-implementation` · **Working
tree clean at:** `de3531e` ("plan: the remaining planned buttons, form field
authoring dropped by decision") · Local version `0.1.63`.

**Tier B's three read-only reports are built since this plan was written** —
`crossref`, `transitions` and `calcorder` are commands now and are in the
[Built](#built-on-this-branch-do-not-redo) table. Three verbs remain in
`verbs::PLANNED`, and **every one of them is waiting on a product decision,
not on code**: `emailattach`, `toolsettings` and `comment`. See
[the remaining plan](#remaining-plan-3-verbs--2-buttons).

Every ribbon button runs a word, and every word left in `verbs::PLANNED`
(`crates/pagify_shell/src/verbs.rs`) answers "planned for <phase>, and not
built yet" instead of running. This file is the plan for everything still
there. **Form field authoring (Tier C's old biggest block) is dropped by
decision — see [Dropped](#dropped-by-decision) — and must not be picked back
up without a new decision.**

Counts move; `PLANNED` is the source of truth for the list, this file is the
source of intent.

---

## Built on this branch (do not redo)

| Word | Where | What it does |
| --- | --- | --- |
| `email` | Share | drafts a message in the default mail client (`mailto:`, words + `file://` link) |
| `sendreview` | Share | the same, as a review request |
| `sharelink` | Share | copies a properly escaped `file://` link to the clipboard |
| `cloudstorage` | Share | reveals the document's own file in the OS file manager (no upload) |
| `trackreviews` | Share | opens the Comments panel, where every review mark is listed |
| `readaloud` | View | speaks the page in view with the OS voice; `readaloud stop` or Escape stops it |
| `portfolio` | Convert | a panel of the document's embedded files: Extract…, ✕, Add file… |
| `crossref` | Edit | reports the internal page links the document already has: page, click point, target page; "no cross-references" when there are none. Read-only |
| `transitions` | View | reports each page's `/Trans` effect (type, duration, direction) and `/Dur` display time **from the file on disk**; "no page transitions" when there are none. Read-only |
| `calcorder` | Form | reports the fields in `/AcroForm`'s `/CO` calculation order **from the file on disk**; "no calculation order" when there is none. Read-only |

Those three reports are the read halves of what this plan called Tier B's
first three items. The write halves — creating an internal link, writing a
transition, reordering the calculations — are still Tier C, below.

Also already on the branch from earlier work: the Notes reading window
(`panels.rs::draw_note_view`) and the RTL/bidi text-field crash fix
(`text_fields.rs`, `theme.rs`). Both are tested; leave them alone.

Attachment plumbing added alongside `portfolio`, **already present**: 
`pdf_core::document::Document::{attachment_details, attachment_bytes,
remove_attachment}` (`pdfium_doc.rs`), `Session::{attachment_details,
attachment_bytes, remove_attachment}` (`session/attachments.rs`).

---

## Dropped by decision

**Form field authoring is out of scope.** The Form tab's tools that *create*
fields need new widget annotations plus `/AcroForm` field-tree entries written
by the engine (PDFium's public API reads and fills forms, but does not create
widget annotations), and each type needs its own undo story. That is an
engine-scale feature, not a button wiring job.

These 17 verbs stay in `PLANNED` and must keep answering as planned:

`fieldtext` · `fieldcheckbox` · `fieldradio` · `fieldbutton` · `fieldlist` ·
`fieldcombo` · `fieldimage` · `fielddate` · `fieldsignature` ·
`fieldbarcode` (Form) · `combfield` (PagiSign) · `formrecognise` ·
`formdesigner` · `createform` · `editxfa` · `pagetemplates` · `javascript`
(Form/Convert)

That covers 17 of the 30 remaining verbs and 17 of the 28 remaining buttons.
Do not remove them from `PLANNED`, do not add half-implemented verbs for them.

---

## Remaining plan: 3 verbs · 2 buttons

Everything left needs a **decision** before it needs code. Nothing below is
blocked on an engine primitive this build could improvise.

### Tier B — the three left, each a decision first

1. **`emailattach`** — Share tab, "Attach to Email". Needs one decision, not
   much code: how much untested platform glue to carry. Linux has
   `xdg-email --attach`; macOS can drive Mail with a short AppleScript;
   Windows has no attach-capable command line, so it would fall back to
   opening the draft and revealing the file (saying that). **Do not fake it**:
   if the platform cannot attach, the message must say so. The button must not
   claim an attachment the OS never made.

2. **`toolsettings`** — Form tab, "Tool Settings". Only worth building once
   there is a setting worth storing (e.g. highlight fields while filling).
   Product decision first; if there is no decision, leave planned.

3. **`comment`** — no button; the planned table says "threaded review
   comments". A real model (threads + replies on annotations), not an alias
   of `note`; needs a design decision before code — where threads live, how a
   reply is anchored, and whether they are annotations or a sidecar.

### Tier C — needs engine work (non-form)

- **`autotag`** (Accessibility): writing a `/StructTreeRoot` from page text and
  layout — the reader exists (Tags panel), the writer does not.
- **`add3d`**, **media**: 3D and audio/video annotations. PDFium's creation
  API here has no such annotation types; needs raw annotation objects.
- **Write halves of the three reads just built**: page transitions,
  calculation order, and internal-link creation (see `crossref`).
- **Portfolio cover sheet / navigator layout**: files are managed now; Adobe's
  layout is not written.

### Tier D — needs a service or an external program

- **PagiSign online**: `requestsignature`, `sendbulk`, `onlineform`,
  `signbranding` — account, server, legal envelope format. Local signatures
  already work.
- **Cloud Storage upload**: provider needed; the local reveal is built.
- **`fromscanner`** (Home and Convert): TWAIN/WIA/SANE integration.
- **`tooffice`** (Convert): PDF → DOCX/PPTX/XLSX is its own engine.
- **`assistant`** (View): model service; a local imitation is worse than
  saying it is planned.

---

## How to take one off the list

Adding a word to `parse`, a `Verb` variant, an `Effect` and the app handler is
the easy part; tests are the contract. A word leaves `PLANNED` only with:

1. a parse test, plus the assertion that it is no longer in `PLANNED`
   (`verbs.rs` `*_tests` modules — copy `the_portfolio_word_parses_and_is_not_planned`),
2. a `command_plan` mirror case (`command_plan.rs::the_files_and_text_domains_mirror_their_verbs`
   or the matching domain test),
3. an app test for the line it says (`crates/pagify_app/src/tests.rs`; `said(&app)` helper),
4. a UI test if it opens a window (`crates/pagify_app/src/ui_tests.rs`),
5. `help`/README/this file updated, including the planned counts (30 verbs /
   28 buttons now),
6. every suite run, no new failures, one commit pushed.

The three reports show the shape a read-only verb takes: a shell module that
returns the lines (`pagify_shell::{crossref,transitions,calcorder}`), an
`Effect` per verb, and an app handler that says the lines — with the positive
and negative paths tested separately, because no shipped fixture has an
internal link, a page transition or a `/CO` array.

Free functions of the kind these use live in `main.rs` near `open_in_browser`
and `file_uri`/`mailto_uri`; note both have `cfg!(test)` seams so tests never
launch a browser, mail client, file manager or voice — keep that rule.

## Verify before and after

From `desktop/`:

```sh
export PAGIFY_PDFIUM_LIB="$PWD/third_party/pdfium/pdfium-linux-x64/lib/libpdfium.so"
export XDG_CONFIG_HOME="$PWD/target/audit/test-config"   # isolates the real ~/.config/Pagify
mkdir -p "$XDG_CONFIG_HOME"
cargo test -p pagify_shell --no-fail-fast     # 798 passed, 0 failed, 37 ignored
cargo test -p pagify_app --no-fail-fast       # 986 passed, exactly 12 known failures, 15 ignored
cargo check --workspace --all-targets         # clean
```

The 12 app failures are the documented Linux baseline and are not ours:
`hub::tests::*` (3), `lock_wiring_tests` (4), `paragraph_apply_tests` (5). No
new failure is acceptable; fix anything else before committing.

Do **not** run `cargo fmt` (the tree is not rustfmt-clean and was not before
this work); match the surrounding style by hand.

## Repo notes

- Commit style: `area: lowercase summary`, bullet body, bold
  reported-from-use claims, tests list, suite counts. Push to
  `farzad-phase-implementation` (`git push origin farzad-phase-implementation`);
  the credential-helper "read-only file system" warning is cosmetic and the
  push still succeeds — check the ref update line.
- The design-review/architecture docs are `desktop/docs/DESIGN_REVIEW.md` and
  `desktop/docs/ARCHITECTURE.md`; the user-facing status is `desktop/README.md`.
- Keep temp probe files and temp tests out of the tree; `desktop/target/` is
  ignored, `target/` at the repo root is a trap.
