# The planned buttons — what is left, and what it would take

**Date:** 2026-10-12 · **Branch:** `farzad-phase-implementation`

Every ribbon button runs a word, and every word in `verbs::PLANNED`
(`crates/pagify_shell/src/verbs.rs`) answers "planned for <phase>, and not
built yet" instead of running. This file is the plan for the ones still there:
what each button is, which of them can be built locally, and which need a
service or a piece of engine work that does not exist.

Counts move; the prose here is the source of intent, and `PLANNED` itself is
the source of truth for the list.

---

## Built (this branch)

| Word | Where | What it does |
| --- | --- | --- |
| `email` | Share | drafts a message in the default mail client (`mailto:`, words + `file://` link) |
| `sendreview` | Share | the same, as a review request |
| `sharelink` | Share | copies a properly escaped `file://` link to the clipboard |
| `cloudstorage` | Share | reveals the document's own file in the OS file manager (no upload) |
| `trackreviews` | Share | opens the Comments panel, where every review mark is listed |
| `readaloud` | View | speaks the page in view with the OS voice; `stop`/Escape stops it |
| `portfolio` | Convert | a panel of the document's embedded files: Extract…, ✕, Add file… |

What deliberately did **not** get built with them: uploading anything,
threaded comments, attaching a file to a mail draft (`emailattach`), or an
Adobe-style portfolio cover sheet and layout. The messages say so rather than
letting the button imply more than it does.

---

## Tier B — buildable locally, next

These need no service and no new PDF-writing primitive; they are reads of
something the engine already parses, or OS glue this app already uses for
`open` and `readaloud`.

| Word | Button | Plan |
| --- | --- | --- |
| `crossref` | Edit → Cross Reference | List what the document already cross-references: for every page, its internal link annotations and the page each one jumps to. Creation of internal links needs a destination-writing primitive PDFium does not expose here (only URI links can be set), so the first step is the report; a "link these two pages" tool is a separate engine task. |
| `calcorder` | Form → Calculation Order | Read `/AcroForm`'s `/CO` through `pdf_core::pdf::File` and list the fields in calculation order, with names. Read-only first: reordering is a form-authoring write (Tier C). |
| `transitions` | View → Page Transitions | Report each page's `/Trans` entry (type, duration, direction) from the raw file. Writing page transitions is a raw-dict edit, Tier C. |
| `emailattach` | Share → Attach to Email | The platform half of the problem: Linux has `xdg-email --attach`, macOS can drive Mail with a short AppleScript, Windows has no attach-capable command line — so Windows falls back to opening the draft and revealing the file, and says so. Needs a decision on how much per-platform untested glue to carry. |
| `toolsettings` | Form → Tool Settings | A real settings panel only if there is a setting worth storing (e.g. highlight fields while filling). Needs one decision — which setting — rather than more code. |

Two words without a ribbon button belong here too: `comment` (threaded
comments — a real model, not an alias of `note`) and `add3d`/`media` (below).

## Tier C — needs engine work

- **Form field creation** — `fieldtext`, `fieldcheckbox`, `fieldradio`,
  `fieldbutton`, `fieldlist`, `fieldcombo`, `fieldimage`, `fielddate`,
  `fieldsignature`, `fieldbarcode` (Form tab), `combfield` (PagiSign), plus
  `formrecognise`, `formdesigner`, `createform`, `editxfa`, `pagetemplates`,
  `javascript`, `add3d`, `media`, `autotag`.
  PDFium's public API reads and fills forms but does not create widget
  annotations; each field type is new PDF objects (annotation + `/AcroForm`
  field tree entry) written by the engine, and then an undo story that
  `add_attachment`/`remove_attachment` do not have today. This is the single
  largest remaining block: 17 of the 31 planned buttons sit here.
- **Autotag Document** — writing a `/StructTreeRoot` from the page's text and
  layout is the structure-tree counterpart: the reader exists (Tags panel),
  the writer does not.
- **Page Transitions and Calculation Order writes** — raw dict edits; read-only
  reports come first (Tier B).
- **Portfolio layout/cover sheet** — the files are managed now; Adobe's cover
  sheet and navigator layout are not.

## Tier D — needs a service or an external program

- **PagiSign** — `requestsignature`, `sendbulk`, `onlineform`,
  `signbranding`: online signing needs an account, a server and a legal
  envelope format. Local signatures already work; these five are the online
  half and cannot be invented locally.
- **Cloud Storage** — uploading needs a provider; the local reveal is built.
- **From Scanner** (`fromscanner`, Home and Convert) — TWAIN/WIA/SANE
  integration; the middle of that work is not Rust.
- **To MS Office** (`tooffice`) — PDF → DOCX/PPTX/XLSX conversion is an engine
  of its own.
- **Assistant** — needs a model service; a local imitation would be worse than
  saying it is planned.

---

## Rule when taking one off the list

Adding a word to `parse`, a `Verb` variant, a plan and an effect is the easy
part; the tests are the contract. A word leaves `PLANNED` only with:

1. a parse test (and the `PLANNED`-exclusion assertion),
2. a `command_plan` mirror case,
3. an app test for the line it says, and
4. a UI test if it opens a window.

`emailattach` and `comment` are the examples of the other honest option: stay
in `PLANNED`, and make the reason in that table match what a person would ask.
