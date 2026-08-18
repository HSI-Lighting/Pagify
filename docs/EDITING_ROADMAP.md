# Pagify — Editing Engine Roadmap

Target audience: a coding agent or developer picking up work on this repo.
Status of the codebase when this was written: **phase 1 complete** (open, view,
navigate, zoom, rotate, cache), phase 2 in progress (text extraction, thumbnails,
annotation layer). There is **no write path at all** — the engine cannot yet
modify or save a PDF.

> **Revision 3 — 2026-08-18.** Amended after auditing the tree rather than assuming
> it. Changed: the `editing` feature does not compile (§4.3); `Command` must be a
> serialisable enum with a separate, non-serialisable undo record (§4.2);
> `EditableDocument` is annotation-shaped and must be redrawn before Phase A (§5,
> Phase A); the Kotlin/Rust model conflict is resolved by a new decision on the
> interaction boundary (§4.7); acceptance criteria must name a tool that can actually
> check the claim (§7); and the memory figures in §8 are corrected — the *uncapped*
> raster pool is the largest consumer, not the smallest. Phase ordering, sizings and
> the feature index are unchanged. The immediate task breakdown lives in
> `Groundwork Instructions`.

---

## 0. How to read this

The feature list this roadmap answers is a *product* list. It is not in dependency
order, and it mixes work that differs by two orders of magnitude in cost. This
document reorders it into the sequence the architecture actually forces, and says
plainly which items are cheap, which are expensive, and which are research.

Three rules govern every phase below:

1. **Nothing ships before the write path exists.** Phase A is not optional and not
   parallelisable.
2. **Every mutation is a `Command`.** This is what makes undo, batch processing and
   scripting fall out for free later. It is ruinous to retrofit.
3. **The model lives in Rust.** Kotlin stays a thin view layer, so iOS and desktop
   reuse the work. This is already the project's stated intent — editing is where
   it either holds or quietly stops holding.

---

## 1. Summary

| Phase | Delivers | Size | Blocked by |
|---|---|---|---|
| **A — Write Path** | Save, page organisation, command plumbing | 4–6 wk | — |
| **B — Object Layer** | Images, shapes, alignment, watermarks, headers/footers | 6–8 wk | A |
| **C — Text Layer** | Text editing, reflow, linked blocks, spell check | 20–30+ wk | B |
| **D — Recognition** | OCR, searchable scans, table editing | 8–12 wk | B |
| **E — Analysis** | Compare, optimise/compress | 6–8 wk | A |
| **F — Automation** | Action Wizard, batch, rich media | 4–6 wk | A + whatever it batches |

Sizings assume one experienced developer and are deliberately wide. Phase C is
open-ended: "Word-like editing" is not a feature, it is a product area that
commercial vendors have staffed for years and still ship with visible failure modes.

Phases **D** and **E** can run in parallel with **C** — they touch different
subsystems. **A → B** is strictly serial.

---

## 2. Reality check: what a PDF actually is

Read this before estimating anything in the list.

**A PDF has no paragraphs.** It has positioned glyph runs. A line of text is a
`Tj`/`TJ` operator in a content stream with a text matrix placing it at absolute
coordinates. There is no concept of "the next line", no concept of "this paragraph
is justified", no concept of a text box with a width.

Everything in the request's §1 and §3 — reflow, linked text blocks, split and join —
requires **reconstructing** a document model that was destroyed when the PDF was
generated, editing that model, then **re-emitting** the content stream. That
reconstruction is heuristic. It will sometimes be wrong. Every editor on the market
has this property.

**Embedded fonts are usually subsets.** A PDF typically embeds only the glyphs it
uses. Typing a character that is not in the subset means there is no glyph to draw.
This is the single most common practical failure of PDF text editing, and it needs a
product decision, not just an engineering one — see §4.6.

**Consequently:** §2 (objects), §5 (styling), §7 (organisation) are *manipulating
what is already there* and are tractable. §1 and §3 are *rebuilding what was thrown
away* and are not. Sequence accordingly.

---

## 3. The forced ordering

```
                    ┌──────────────────────────────┐
                    │  A. Write Path               │  save, incremental update,
                    │     Command plumbing         │  page tree ops
                    └──────────────┬───────────────┘
                                   │ everything needs a way to persist
              ┌────────────────────┼────────────────────┐
              │                    │                    │
    ┌─────────▼─────────┐  ┌───────▼────────┐  ┌────────▼────────┐
    │  B. Object Layer  │  │  E. Analysis   │  │  F. Automation  │
    │  enumerate,       │  │  compare,      │  │  Action Wizard  │
    │  transform, stamp │  │  optimise      │  │  (needs cmds)   │
    └─────────┬─────────┘  └────────────────┘  └─────────────────┘
              │
      ┌───────┴────────┐
      │                │
┌─────▼──────┐  ┌──────▼───────┐
│ C. Text    │  │ D. Recognition│
│    Layer   │  │    OCR/tables │
└────────────┘  └───────────────┘
```

Why B before C: the text layer needs object enumeration, bounding boxes, transforms
and content-stream re-emission. All of that is built in B for the much easier case of
images and shapes. Building it first against simple objects, then reusing it for
text, is the difference between one hard problem and two.

---

## 4. Decisions to make before writing Phase A code

These are cheap now and very expensive later. Record the answers in the repo.

### 4.1 Incremental update, not full rewrite — **decide: incremental**

A PDF can be saved two ways: rewrite the whole file, or append a delta (an
*incremental update*) that overrides objects in the original.

This is not a performance detail. **A digital signature covers a byte range of the
file.** Rewriting the file destroys every existing signature and makes it impossible
to add one that survives further edits. Signatures are explicitly on the roadmap for
a later stage — choosing full rewrite now silently removes that option.

Incremental update also gives, for free:
- fast saves on huge files (append only, no rewrite of the 2.9 GB fixture),
- crash-safe autosave,
- a natural undo-on-disk story.

Cost: file size grows with each save. Mitigate with an explicit "Save optimised
copy" that does a full rewrite, offered as a user action, never as the default.

PDFium: `FPDF_SaveWithVersion` with the `FPDF_INCREMENTAL` flag.

### 4.2 Every mutation is a `Command` — **decide: yes, no exceptions**

`rust/pdf_core/src/command/` already has the trait and a tested history stack. Make
it the *only* path to mutating a document. Direct calls to a mutate-the-page function
from the JNI bridge must not exist.

A `Command` should be:
- serialisable (`serde`) — this is what makes §8 Action Wizard a for-loop,
- reversible (`undo`), or explicitly marked irreversible,
- declarative about what it invalidates (`invalidate_page(n)` already exists in the
  cache).

If this holds, the Action Wizard in Phase F is roughly two weeks. If it does not,
Phase F is a rewrite of every operation.

**Make `Command` an enum, not a trait object.** You cannot derive `Serialize` on a
trait, and the usual workaround — `typetag` — is a trap in this build specifically:
it registers implementations through link-section tricks, and the release profile
here is `lto = true`, `codegen-units = 1`, `strip = "symbols"`, built as a `cdylib`
for Android. That combination is exactly where static registration gets stripped or
never runs, giving a green build and empty deserialisation at runtime.

An enum with `#[derive(Serialize, Deserialize)]` and `match` dispatch has no
link-time magic, works on every target, and makes an Action Wizard script a plain
`Vec<Command>`. The cost is losing open extensibility for third-party commands; the
existing `Plugin` trait can carry its own escape hatch if that is ever needed.

**And separate intent from the undo record.** Commands self-invert today — the
history stores `Box<dyn Command>` and calls `command.undo(doc)`. That works for
annotations, where the command knows what it added. It cannot work for a page
deletion: the content is gone, and a serialisable enum by definition cannot carry a
removed page's bytes.

These are two different things:

| | **Intent** | **Undo record** |
|---|---|---|
| What | the `Command` enum | `UndoRecord` |
| Serialisable | yes | **no** |
| Scope | replayable against any document | one document, one execution |
| Holds | parameters only | removed pages, prior state |
| Stored by | Action Wizard scripts | `CommandHistory` only |

So `execute` returns an `UndoRecord`, and `CommandHistory` stores `(Command,
UndoRecord)` pairs. An Action Wizard script stores `Vec<Command>` alone — a script
replaying against a *different* document must not carry the first document's undo
payloads, which would be a correctness bug rather than merely bloat.

One consequence to decide rather than default into: undo for deletions cannot survive
a process restart, because the removed page lives in memory. Accept it and clear undo
history on restart, spill undo records to the autosave sidecar, or — since §4.1
already commits to incremental save — make undo truncate to the previous xref instead
of holding page bytes. The last is elegant but constrains save cadence to one
incremental section per command.

### 4.3 One writer — **decide: PDFium, and delete the `editing` feature**

`lopdf` sits behind the `editing` feature flag. Resist using both writers. Two
independent object models writing one file is a class of corruption bug that is
extremely hard to diagnose.

**That feature does not currently compile.** `document/mod.rs` declares
`pub mod editable_doc;` under `#[cfg(feature = "editing")]` and the file does not
exist, so `cargo check --features editing` fails on a missing module. Nothing behind
that flag is dormant — it is aspirational.

Do not fix it by creating the file. The module's own header describes it as "the
`lopdf`-backed editable one", which is the architecture this decision reverses;
making it compile would cement that behind a green build. Delete the module
declaration, delete the `editing` feature and the optional `lopdf` dependency, and
build the mutation trait against PDFium with **no feature flag at all**. Editing is
not optional any more — it is the product, and a flag around it guarantees the
default build never type-checks the editing path.

PDFium covers Phase A and B almost entirely:
- page tree: `FPDF_ImportPagesByIndex`, `FPDFPage_Delete`, `FPDF_CreateNewDocument`
- objects: `FPDFPage_GetObject`, `FPDFPageObj_Transform`, `FPDFPage_RemoveObject`,
  `FPDFPageObj_NewImageObj`, `FPDFPageObj_CreateNewPath`
- text: `FPDFPageObj_NewTextObj`, `FPDFText_SetText`, `FPDFTextObj_GetFont`
- save: `FPDF_SaveWithVersion`

Keep `lopdf` for read-only structural analysis if useful (Phase E optimisation
inspection), and **never let it write a user file** until it has been fuzzed — see §7.

### 4.4 Coordinate space — **decide: top-left origin, y down, everywhere above the engine**

Already chosen correctly in `TextSegment` (the engine flips PDF's bottom-left origin
once). Make it a documented, repo-wide law with a single conversion site. Every
editing feature multiplies the number of places this can be got wrong.

### 4.5 Where the model lives — **decide: Rust core**

The page-object model, the text/layout model, geometry, and all commands live in
`pdf_core`. Kotlin receives serialised snapshots and sends commands. The temptation
in Phase B will be to do alignment maths in Kotlin because it is closer to the
gesture handler. Resist: that is the work iOS and desktop would have to redo.

### 4.6 Font policy on missing glyphs — **decide and surface it**

When the user types a glyph absent from the embedded subset, pick a ladder and stick
to it:

1. If the full font is available on the device, re-embed and extend the subset.
2. Otherwise substitute a metric-compatible fallback for **the edited run only**,
   and mark the run visually as substituted.
3. Never silently drop the character or draw `.notdef`.

The UI must tell the user when a substitution happened. Editors that hide this
produce documents that look fine on the editing device and wrong everywhere else.

Crates: `skrifa` / `ttf-parser` (parsing), `subsetter` or `allsorts` (subsetting),
`rustybuzz` (shaping).

### 4.7 The interaction boundary — **decide: split at the commit, not the gesture**

§4.5 says the model lives in Rust. The annotation model was built in Kotlin, and
those conflict. The Kotlin choice was not careless: PDFium is bound `thread_safe`,
every call into a session serialises against page rendering, and a render on the
catalogue takes 300–780 ms. A drag emits an event per frame and cannot sit behind
that lock.

The resolution is a boundary, not a side:

> Ephemeral interaction state lives in Kotlin. Committed state is a `Command`
> against the Rust document.

Kotlin keeps the wet stroke, the live selection, hit-testing and drag preview — pure
geometry over already-cached data, never touching the engine. On commit, the change
becomes a `Command`, and undo/redo moves to the existing `CommandHistory`. The
Kotlin store stops being the model and becomes a projection of it.

**This is not an annotation decision.** It is the interaction architecture for all of
Phase B and C: in B the ephemeral state is a drag handle on an image, in C a text
cursor and in-progress typing. Settling it once, while there is one edit shape, is
about a week. Rediscovering it per feature is the rest of the project.

Two things to get right when implementing it:

1. **Wire invalidation immediately.** `Command::affected_pages()` already exists and
   is exactly the hook that stops the Kotlin projection drifting from the Rust model.
2. **The registry lock split is still separately needed.** This boundary is correct
   regardless, but it does not stop a cache *hit* queueing behind a long render
   inside `registry::with_session`.

---

## 5. Phases

### Phase A — Write Path

**Goal:** the engine can modify a document and save it, and the first shippable
editing features land.

**Scope**
- A mutation trait implemented against PDFium. **Do not reuse `EditableDocument`** —
  it is annotation-shaped (`add_annotation`, `add_signature`, `remove_annotation`,
  `save`) and carries no page-tree operations. Because `Command` and `CommandHistory`
  are both typed against it, redrawing it is a prerequisite of Phase A, not something
  to discover during it. Name the replacement for what it does (`DocumentMut`), and
  keep annotation operations off it — they arrive as commands against the Phase B
  object layer.
- Save / Save-As / incremental save; dirty-state tracking; autosave to a sidecar.
- `Command` plumbing end to end: JNI → registry → command stack → cache invalidation.
- Page tree operations: reorder, delete, insert blank, extract range, rotate
  (persisted, not just view rotation), merge documents, split document.

**Covers from the request:** §7 page organisation (merge, split, reorder, extract).

**Why this is first:** it is the smallest change that produces a *shippable* feature
set, and it forces the save-architecture decision that signatures depend on.

**Acceptance**
- Round-trip test: open → command → save → reopen → model matches expectation.
- `qpdf --check` passes on every saved output in the corpus (see §7).
- Existing signatures on a signed test PDF survive an unrelated edit.
- Saving the 2.9 GB fixture after a one-page edit completes in under 2 s.

**Risks:** descriptor and lifetime handling on save when the document was opened from
a detached fd (see `openDocumentFd` — ownership already transfers to native). Saving
must not invalidate the open handle.

**Size:** 4–6 weeks.

---

### Phase B — Object Layer

**Goal:** anything on a page can be selected, moved, and restyled.

**Scope**
- Enumerate page objects with type, bounding box, transform, z-order.
- Hit-testing from a screen point (reuse the aperture-style depth-tolerance idea:
  smaller objects should win over the large background object beneath them).
- Transform commands: move, resize, rotate, flip, delete, reorder z.
- Insert / replace image objects; insert path and shape objects.
- Align and distribute (pure geometry — fully unit-testable in the core).
- Text-to-path conversion (`FPDFFont_GetGlyphPath`).
- Stamping: watermarks, backgrounds, headers, footers, page numbers, Bates numbering.

**Covers from the request:** §2 entirely, §5 entirely, §6 partially.

**Note on §5:** watermarks, headers, footers and Bates numbering are all "generate
objects and stamp them onto a page range". They are cheap once this layer exists and
have high perceived value. Ship them as soon as B lands, not later.

**Acceptance**
- Align/distribute has property tests: distributing N objects always yields equal
  gaps; aligning is idempotent; both are invariant under document rotation.
- Golden-render tests: transform an object, render, compare to a stored PNG within
  tolerance.
- Round-trip: transform → save → reopen → bounding box matches within 0.01 pt.

**Size:** 6–8 weeks.

---

### Phase C — Text Layer

**Goal:** editing text on the page, with reflow.

This is the largest item in the roadmap by a wide margin. Break it into shippable
steps rather than attempting reflow directly; each step below is independently
useful.

**C1 — In-run editing (no reflow).** Replace characters inside an existing text run,
same font, same size. Line width changes; nothing else moves. Ships as "fix a typo".
*2–3 wk.*

**C2 — Font machinery.** Subset inspection, subset extension, fallback substitution,
shaping. Implements the §4.6 ladder. This is the prerequisite that makes everything
after it possible. *4–6 wk.*

**C3 — Line model.** Glyph runs → words → lines. Line-level re-layout: a run that
grows re-breaks within its own line box. *3–4 wk.*

**C4 — Paragraph model.** Line clustering into paragraphs; block detection
(indentation, leading, alignment inference); justification and hyphenation. This is
the reconstruction step and the main source of "it got it wrong" bugs. *6–10 wk.*

**C5 — Flow.** Linked text blocks, split and join. Natural once C4 exists, because
both are operations on a flow model rather than on the PDF. *3–4 wk.*

**C6 — Spell check.** Cheap once a text model exists. `symspell` (fast, small) or
`hunspell-rs` (better quality, larger dictionaries). Watch APK size: an en_US
hunspell dictionary is roughly 1 MB. *1–2 wk.*

**Covers from the request:** §1 text editing, §1 spell check, §3 entirely.

**Layout engine:** do not write a line breaker from scratch. `parley` (Linebender)
gives Rust text layout with shaping via `swash`/`skrifa`, or `cosmic-text` as an
alternative. Adopting one of these is worth several weeks.

**Acceptance**
- Reflow fidelity corpus: a set of documents with known paragraph structure; assert
  that an edit that does not change text length produces a byte-identical rendered
  page.
- Substitution is always reported to the UI, never silent — assert on the event.
- Round-trip through save preserves the reconstructed model.

**Size:** 20–30+ weeks, and treat that as a floor.

---

### Phase D — Recognition

**Goal:** scanned pages become selectable, searchable and editable.

**Scope**
- OCR pipeline behind a trait, so platform engines can be swapped in.
- Invisible text layer: OCR output written as text objects in render mode 3 (`3 Tr`),
  positioned over the page image. This is what makes a scan searchable without
  changing how it looks.
- Suspect correction: surface per-word confidence, let the user step through
  everything below a threshold.
- Table detection: ruling-line detection plus text-alignment clustering, producing a
  grid model that Phase C can then edit.

**Covers from the request:** §4 entirely, §1 table editing.

**Engine choice:** `ocrs` (pure Rust, ONNX via `rten`) fits the platform-neutral core
and needs no C++ toolchain. Android's ML Kit text recognition is faster and free but
platform-specific. Recommendation: define `trait OcrEngine` in the core with `ocrs`
as the default, and let the Android layer inject ML Kit — this is exactly what the
existing `plugins` module is shaped for.

**Table detection is research.** There is no good off-the-shelf Rust crate. Budget it
separately and treat the first version as best-effort with manual grid correction.

**Size:** 8–12 weeks (OCR 5–7, tables 3–5 and open-ended).

---

### Phase E — Analysis and Optimisation

**Goal:** understand and shrink documents.

**Scope**
- Document comparison: text diff (`similar` crate) plus visual raster diff of
  rendered pages; side-by-side view with change highlighting.
- Optimisation: image downsampling and recompression, content-stream recompression,
  duplicate object detection, unused-object collection, metadata stripping.

**Covers from the request:** §7 comparison, §7 optimisation.

**Note:** optimisation is directly relevant to this project's own hardest problem.
The 2.9 GB test fixture is roughly 31 MB *per page*; that file is the strongest
possible demo of the feature, and making it lighter would also make every other
subsystem easier to test.

**Crates:** `similar` (diff), `image` / `zune-image` (decode), `mozjpeg` or
`jpeg-encoder` (recompress), `oxipng`.

**Size:** 6–8 weeks.

---

### Phase F — Automation and Rich Media

**Goal:** repeat any operation across many documents.

**Scope**
- Action Wizard: a saved, ordered list of serialised `Command`s.
- Batch runner over a folder or selection, with a progress and failure report.
- Rich media insertion.

**Covers from the request:** §8 entirely, §6.

**On §8:** if §4.2 held, this is assembling existing pieces. The `Command` trait is
already serialisable-shaped and `plugins/mod.rs` already exists. Do not design a
separate scripting language; the command list *is* the script.

**On §6 — be honest with the user.** PDF 2.0 deprecated RichMedia annotations, the
Flash-based rich media that Acrobat once supported is dead, and essentially no viewer
outside Acrobat renders embedded video. Recommended implementation:
- embed the media as a file attachment,
- place a poster image on the page,
- optionally add a `Screen` annotation for viewers that support it.

Set the expectation in the UI that playback outside Pagify is unreliable. Do not
invest heavily here; it is the lowest-value item in the list.

**Size:** 4–6 weeks.

---

## 6. Feature index

Every item from the request, mapped.

| Requested feature | Phase | Difficulty | Note |
|---|---|---|---|
| Word-like text editing / reflow | C3–C4 | **Very hard** | Rebuilds a destroyed model |
| Table editing | D | **Very hard** | Detection is research |
| Spell check | C6 | Easy | Needs a text model first |
| Move/resize/rotate/flip/replace graphics | B | Moderate | Best value per effort |
| Align and distribute | B | Easy | Pure geometry, property-testable |
| Text-to-path | B | Moderate | `FPDFFont_GetGlyphPath` |
| Linked text blocks | C5 | Hard | Free-ish after C4 |
| Split / join text | C5 | Hard | Free-ish after C4 |
| Editable scans (OCR) | D | Hard | Invisible text layer, render mode 3 |
| OCR suspect correction | D | Moderate | Confidence threshold + review UI |
| Watermarks and backgrounds | B | Easy | Stamped objects |
| Headers, footers, Bates numbering | B | Easy | Stamped objects |
| Rich media insertion | F | Moderate | Poor portability — see above |
| Merge / split / reorder / extract | **A** | Easy | **Ship first** |
| Document comparison | E | Moderate | `similar` + raster diff |
| Optimisation / compression | E | Moderate | Fixes this project's own fixture |
| Action Wizard / batch | F | Easy *if* §4.2 held | Otherwise a rewrite |

---

## 7. Testing strategy for an editor

The existing test tiers stay: 68+ Rust host tests, the JVM unit tier in
`app/src/test/`, and the instrumented tests. Editing adds four kinds that do not
exist yet and are not optional.

**Round-trip tests.** open → apply command → save → reopen → assert. This is the
primary defence for every phase. A command that appears to work but does not survive
a save is the most common editing bug.

**Corpus tests.** Keep a directory of real PDFs — generated, scanned, signed,
encrypted, malformed, CJK, right-to-left, huge. Run every operation across all of
them and assert no corruption. Validate saved output with an *external* tool:

```bash
qpdf --check output.pdf          # structural validity ONLY
pdfsig output.pdf                # signatures (poppler-utils)
veraPDF --flavour 2b output.pdf  # if PDF/A conformance matters
```

External validation matters because the failure mode is "opens fine in Pagify,
corrupt in Acrobat". A check written against your own parser cannot catch that — the
same reasoning that made the asymmetric-orange fixture necessary for the channel-order
bug.

**Name a tool that can actually check the claim.** `qpdf --check` validates structure
and cannot inspect a signature at all; asserting "the signature still verifies" under
it is worse than no criterion, because it reads as covered. Signatures need `pdfsig`.

**And never assert something time-bound.** Certificates expire. A criterion that
checks trust-chain validity will start failing on a date with no code change. Assert
*byte-range integrity* instead — that the signed region is unmodified and its digest
still matches — which is the property an edit could actually break.

**Golden-render tests.** Render a page before and after an operation, compare against
a stored PNG within tolerance. The only practical way to catch visual regressions in
B and C.

**Fuzzing.** Mandatory the moment any Rust code parses untrusted document bytes —
that means the day `lopdf` is enabled, or the day a hand-written content-stream parser
appears. `cargo-fuzz` on the parse entry points. Until then the C++ parser is
PDFium's problem and its fuzzing is already done upstream.

**Property tests.** Geometry in Phase B is pure maths — `proptest` over transforms,
alignment and distribution invariants is far more valuable than hand-written cases.

---

## 8. Risk register

| Risk | Phase | Impact | Mitigation |
|---|---|---|---|
| Full-rewrite save chosen by default | A | Signatures become impossible | Decide §4.1 before writing save |
| Mutations bypass `Command` | A | Phase F becomes a rewrite | Make it the only mutation path |
| Two writers (PDFium + lopdf) | A | Silent file corruption | One writer, §4.3 |
| Missing glyphs handled silently | C2 | Documents wrong on other devices | Ladder + visible UI state, §4.6 |
| Reflow fidelity on real documents | C4 | Feature perceived as broken | Corpus with fidelity thresholds; ship C1–C3 first |
| Table detection under-scoped | D | Schedule slip | Treat as research; manual correction in v1 |
| Memory on large documents | all | OOM on the 2.9 GB fixture | Three cache budgets already need unifying — fix before adding a fourth |
| `lopdf` on untrusted input | any | Security | Fuzz before it touches a user file |
| Rich media portability | F | User disappointment | Set expectations in UI; keep investment low |

**The memory item is a live problem, not a future one**, and the pool that matters is
the one with no budget at all:

| Pool | Cap | Reached by `onTrimMemory` |
|---|---|---|
| Native page cache | 160 MB | yes |
| `ThumbnailCache` | 48 MB | no — `trim()` exists but is never called |
| `recentPageRasters` | **none** | no |

`recentPageRasters` holds four full-page bitmaps. A page measured at 4465 × 3157 on
this device is ~54 MB at ARGB_8888, so that pool alone reaches **~215 MB — more than
both capped pools combined**. It is the largest consumer in the app and the only one
nobody budgeted.

Worse, the per-bitmap ceiling it *should* be bounded by is not enforced:
`RenderScale.forPage` clamps to `maxScale` and then rounds up past it, which is what
keeps `theSixteenMegapixelCeilingIsActuallyEnforced` red. Fix that first, or any
budget is arithmetic over a number that does not hold. An editing model then becomes
the fourth consumer. Resolve all of it in Phase A.

---

## 9. Deferred: signatures, authentication, forms

Explicitly out of scope for now, but two hooks must exist earlier or they become
impossible:

- **Incremental save** (§4.1) — a signature covers a byte range; without incremental
  update, no signature survives a subsequent edit.
- **Byte-range preservation** — the save path must be able to leave a region of the
  file untouched and record its offsets.

Beyond that, when the time comes: signing needs a certificate store and a PKCS#7
implementation, form filling needs the AcroForm object model plus appearance-stream
generation, and both need the object layer from Phase B. Nothing else needs doing
now.
