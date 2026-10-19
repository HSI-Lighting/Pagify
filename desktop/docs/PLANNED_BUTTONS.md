# The planned buttons — hand-off plan

**Date:** 2026-10-12 · **Branch:** `farzad-phase-implementation` · **Working
tree clean at:** `de3531e` ("plan: the remaining planned buttons, form field
authoring dropped by decision") · Local version `0.1.63`.

**Tier B is built except `comment`.** `crossref`, `transitions`, `calcorder`,
`emailattach` and `toolsettings` are commands now and are in the
[Built](#built-on-this-branch-do-not-redo) table. One verb remains in
`verbs::PLANNED`: `comment`, whose **PDF-native reply-thread model is
decided** but which needs the engine work recorded in
[Remaining plan](#remaining-plan-1-verb--0-buttons) — scheduled, not dropped.

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
| `emailattach` | Share | drafts a message with the document's file attached — `xdg-email --attach` on Linux, Mail through AppleScript on macOS. Windows has no attach-capable command line, so the draft opens and the file is selected in the file manager, and both say it was not attached |
| `toolsettings` | Form | the one stored preference: highlight fillable fields while filling — a wash over the boxes a click can fill, drawn over the page and never written into it, kept in `settings.json` |

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

That covers 17 of the 28 remaining verbs and 17 of the 26 remaining buttons.
Do not remove them from `PLANNED`, do not add half-implemented verbs for them.

---

## Remaining plan: 1 verb · 0 buttons

One planned word is left, and it has no ribbon button of its own — the
Comments panel's button runs `managecomments`, which is built.

### `comment` — model decided: PDF-native reply threads

**Decision (this session):** `comment` is **not** dropped and **not** an alias
of `note`; a thread is PDF annotations — a root note plus `/IRT` replies — not
a sidecar file. Recorded here so the work starts from a decision rather than
repeating it.

**Step 1 — the read — is built.** `Annotation::Note` carries `reply_to`, read
from the file's `/IRT` link through `FPDFAnnot_GetLinkedAnnot` and resolved to
the parent annotation by its `/NM` name (or by subtype, rectangle and words
when it is unnamed — PDFium hands back a *fresh wrapper* for the linked
annotation, so its handle is never pointer-equal to the page's own;
measured). `pagify_shell::comments::thread_rows` groups the list into threads,
the Comments panel indents replies under the note they answer, and the link
survives this engine's own save and reopen. Tests: a hand-written `/IRT` file
read back as a thread, a save round trip, the grouping's ordering/cycle/absent
parent cases, and the panel through the real window.

**Why the write half is not written yet.** The pinned PDFium (chromium/7881)
can *read* a reply link, but its public annotation API has **no setter** for
dictionary references: `FPDFAnnot_SetStringValue` writes strings only, and
there is no `/IRT` (or generic dictionary) writer.

The model, so the next session does not have to choose again:

- **A thread is a text annotation plus replies.** The root is an ordinary
  `/Subtype /Text` annotation (what `note` already makes). A reply is a text
  annotation whose `/IRT` names the annotation it answers. No separate file, no
  sidecar — other readers (Acrobat, Preview, Firefox) show the thread too,
  which is the test of whether it is really threaded.
- **Identity comes from annotation object numbers**, which move when a page
  gains or loses annotations (the architecture's standing hazard): a thread is
  re-read from the page after every edit, never held across one, exactly as
  `foreign_marks` does.
- **The panel groups by root, oldest first**, with the root's words and each
  reply indented under it. `trackreviews`' Comments panel keeps listing marks;
  `comment` is where a thread is read and answered.

The engine work the write half needs, in the order it should be done:

1. ~~**Read**~~ — done, above. (The model variant is `Annotation::Note`: a PDF
   `/Text` annotation is a note in this engine, and that is what a reply is.)
2. **Write:** PDFium cannot set `/IRT`, so a created reply needs a raw
   dictionary edit. The least invasive route is a **save-time appended
   revision**: annotations are created through PDFium as they are today (a
   reply starts as an ordinary text annotation, tagged with a unique `/NM`),
   and after PDFium writes the save, `pdf_core::pdf::File::append_revision` adds
   one revision that replaces each tagged annotation's object dictionary with
   the same one plus `/IRT <parent ref>`. That keeps the live PDFium document
   in charge of the bytes it wrote and appends only the one reference it
   cannot write.
   - **A reply appended after a save still leaves the file broken the same way
     any later edit does**: the signature covers the bytes it covered, and the
     appended revision says so. Document that rather than hide it.
   - The append path must be byte-safe (`append_revision` already is) and must
     refuse rather than guess when the tagged annotation cannot be found — the
     file may have been rewritten between the save and the patch.
3. **Undo/redo:** adding a reply is a `Command` like every other annotation;
   the tag and the pending `/IRT` relation belong to the document session, not
   to the app, so the save patch is deterministic.
4. **UI:** the Comments panel gains replies (indented under their root) and a
   Reply box; `comment` on a page with a selected note opens it; `comment
   <words>` starts a new thread root (the same annotation `note` makes, so the
   answer can point at `note` for the root and reserve `comment` for threads
   that have replies).

Step 1 is committed; the next commit is step 2 (the write) alone, and steps
3–4 with it.

### Tier C — needs engine work (non-form)

- **`comment` replies (write half)** — the save-time `/IRT` appended revision
  described above. The read half is worth doing first.
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
5. `help`/README/this file updated, including the planned counts (28 verbs /
   26 buttons now),
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
cargo test -p pagify_shell --no-fail-fast     # 809 passed, 0 failed, 37 ignored
cargo test -p pagify_app --no-fail-fast       # 995 passed, exactly 12 known failures, 15 ignored
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
