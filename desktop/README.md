# Pagify Desktop

A PDF viewer and editor for macOS, Windows and Linux. Rust, egui/eframe, built
on the `pdf_core` engine the phone apps already use and the 2D drawing tools
from SIMLUX's `cad_kernel`.

It is **not** a CAD application. The drawing tools draw *on a PDF page*. There
is no drawing document, no model space, no 3D promotion, no walls. When a design
question is ambiguous: *the PDF is the document*, and everything else is a layer
over a page of it.

Full plan: `PAGIFY_DESKTOP_BUILD_PLAN.md`.

## Layout

```
crates/pagify_shell/   UI-free core: session, page space, markup, dispatch. Tests live here.
crates/pagify_app/     the eframe binary: ribbon, panels, canvas, theme.
```

Both cores are consumed as path dependencies and **never forked**:

| | |
|---|---|
| `pdf_core` | `../workspace/Pagify/rust/pdf_core` |
| `cad_kernel` | `../Documents/3D-factory/cad_kernel` |

## Three things that will bite, all silent

Everything else here can be worked out by reading the code. These two cannot.

### 1. The patch stanza in the root `Cargo.toml` is load-bearing

`pdf_core` needs a one-line fork of `pdfium-render` (`PdfDocument::handle()` made
public, which is what makes incremental save reachable — and therefore what lets
a signature survive an edit). It pulls that in through `[patch.crates-io]` in its
own manifest.

**Cargo honours `[patch.crates-io]` only in the workspace root.** The moment
`pdf_core` became a path dependency here rather than a root in its own right, its
copy stopped being read — *with no warning of any kind*. Delete the stanza from
the root manifest and you get 16 instances of `error[E0624]: method 'handle' is
private`, pointing into a registry source file, with nothing naming the cause.

### 2. Every PDFium access goes through `pdf_core::registry`

The build plan says desktop needs no bridge. True, and this workspace has none —
no JNI, no C ABI, no JSON, no `char *`. But the plan also reads the registry as a
bridge artefact to delete, and that part is wrong: the registry is doing **two**
jobs, and only one is about language boundaries.

The second is serialising PDFium access process-wide. PDFium recycles document
and page addresses, and `pdfium-render` keys a process-global page-index cache on
those raw addresses with no purge on close — so an open racing a read can be
answered with *another document's page*. Measured on this machine, eight threads
opening and reading the same fixture 25 times each:

| | correct | opens failed | wrong geometry |
|---|---|---|---|
| direct construction | 0/200 | 192 | 8 |
| through the registry | 200/200 | 0 | 0 |

Among the wrong-geometry reads: `[0, 612, 612, 612, 612]` — US Letter, which is
what PDFium falls back to when it cannot find a page's geometry.

Desktop needs this *more* than the phones do, not less: tabbed documents and
background prefetch are both on the roadmap and each is exactly that two-thread
case. `Session` therefore owns a registry handle and releases it on `Drop` —
which closes the handle leak §5.3 worried about, without giving up the lock.
`tests/render_path.rs` guards it.

### 3. The two command namespaces overlap far more than the plan assumes

§7 presents the split as clean: the kernel owns the drawing verbs (`l`, `pl`,
`tr`, `f`, `o`) and Pagify owns its own (`open`, `save`, `rotate`, `extract`,
`redact`, `sign`). Measured against `cad_kernel::parser`'s actual table, that is
not the shape of the problem. The kernel already claims **`open`, `save`,
`saveas`, `rotate`, `undo`, `redo`, `help`** — most of §7's own examples.

"Try Pagify's table, fall through to the kernel's" therefore *shadows* real
kernel verbs, silently, by default. Shadowing is often correct — the kernel's
`open` takes a `.dxf` or `.rsm`, which this program does not handle at all — but
it must never be accidental. Every collision is listed in
`verbs::DELIBERATE_OVERRIDES` with what the kernel means by the word, why Pagify
overrides it, and which alias still reaches the drawing command
(`ro`, `u`, `y`, `?`). `tests/command_box.rs` fails on any collision that is not
declared, so a SIMLUX release that adds a colliding verb breaks a test rather
than quietly changing what a word means for someone who uses both.

The same file holds `verbs::REFUSED` — §6's "Leave" column as tokens. `wall`,
`wallstyle`, `blockdiff` and `units` cannot be left behind by not importing
them, because `Wall` is a variant of `Geom` itself and the kernel's parser is one
function that claims all of them. They are declined at dispatch instead, with a
reason and an alternative. One list, one place, for §9's "scope drift back into
CAD" to be reviewed against.

## Scope: PDFs, not drawings

Pagify opens PDFs and files related to them. `.dxf` and `.rsm` are not formats
this program handles. The kernel has an `open`/`save` pair that reads and writes
them; both are unreachable here, guarded twice — Pagify's table claims the words
first, and `dispatch` refuses the kernel's file commands outright even if that
ordering ever changes. `no_route_through_the_box_can_reach_a_drawing_file` holds
the guarantee.

## Running

```
cargo test                      # the whole suite, no window needed
cargo run -p pagify_app         # opens a fixture
cargo run -p pagify_app -- FILE
```

PDFium is found next to the executable first (what packaging will ship), then in
the vendored Pagify tree (development). `PAGIFY_PDFIUM_LIB` overrides both.
`pagify_shell::pdfium::describe()` reports which was used, and the app shows it —
a wrong PDFium presents as "some pages render oddly", so it is worth being able
to read the answer.

**Only `pdfium-mac-arm64` is vendored today**, alongside the two iOS slices.
Windows, Linux and mac-x86-64 have never been fetched, and `tools/fetch_pdfium.ps1`
in the Pagify repo fetches the Apple slices only. Building for those targets
fails at compile time with a message naming the missing slice — deliberately, so
it fails while it is a chore rather than in month seven.

## Status

Phases 0 through 12 of the build plan are implemented. **150 tests, none of
which need a window.**

| Phase | | |
|---|---|---|
| 0 | Walking skeleton | workspace, both cores as plain crates, render path proven end to end |
| 1 | The command box | three-part bar, two-namespace dispatch, submit semantics, history, recall, focus guard |
| 2 | Reader | continuous scroll, viewport-driven cache with neighbour prefetch, thumbnail rail, text selection |
| 3 | Shell chrome | dark violet theme, ribbon tabs, thumbnail panel |
| 4 | The markup layer | live `cad_kernel` object table per page, per-shape hit testing, window/crossing selection |
| 5 | The Draw rail | typed coordinates for every kernel shape; interactive line and circle |
| 6 | The Modify rail | move, copy, rotate, scale, mirror, trim, extend, fillet, chamfer, offset, join, erase |
| 7 | Snap and aids | object snap with badges, one-shot overrides, ortho, grid snap |
| 8 | Commit and round-trip | markup written as real ink **and** as live geometry; reopened, rebuilt, still editable |
| 9 | Measurement | two-point page-scale calibration, distance and area, always shown with their unit |
| 10 | Pagify's tabs | extract, import, delete, insert, reorder, highlight, note |
| 11 | Automate | record command lines, save, replay, stopping at the first failure |
| 12 | Packaging | macOS bundle + nested-first codesign + notarize, Windows, Linux AppImage, and a slice fetcher |

### The window

- **File is backstage, and only File.** Selecting it covers the document with
  the Tool Wizard and Recent Documents, the way File behaves in every ribbon
  application. The document stays open behind it; any other tab brings it back,
  and opening a file from there lands you on it. With nothing open the app
  starts *on* File — where the wizard is — rather than letting every tab borrow
  File's view, which made the tab highlight lie about what you were looking at.
  A non-File tab with nothing open says so and offers to open something.
- **The command box collapses** to the single line the mockup draws, and opens
  to show its history. It opens itself when something goes wrong — a command
  that failed silently because its explanation was folded away is worse than one
  that never ran.
- **Panels resize.** Drag the edge of the thumbnail rail (104–420pt) or the top
  of the command box (96–520pt).

### Installing it

```bash
./packaging/macos/install.sh
```

Builds, bundles, and puts one copy in `~/Applications` — then double-click it,
or `open -a ~/Applications/Pagify.app file.pdf`.

It unregisters the staging bundle in `target/` afterwards, and the previous
install before replacing it. Without that you get a second app in Launchpad per
rebuild, some of them pointing at bundles that have since been replaced. (The
badged Pagify icons with a no-entry sign are a different thing entirely: iOS
build products from Xcode and the Simulator, which macOS lists but cannot
launch.)

### Driving it without a pointer

`pick <x,y>` supplies a click to whatever tool is waiting for one, in the same
coordinates a typed draw command uses — the kernel's page space, y up from the
bottom-left. So a fillet is as typeable as it is clickable:

```
fillet 25
pick 100,250      the first object
pick 170,320      the second
```

That is not a convenience. §7's claim is that anything clickable is typeable;
without it every interactive tool was unscriptable, unrecordable by the Automate
tab, and untestable without a window — which is how the tools stayed broken.

`--run` does the same from the command line:

```bash
pagify plan.pdf --run "l 30,250 170,250" --run "fillet 25" --run "pick 100,250"
```

### Opening a document

Four ways, because for a while there were none:

- **File ▸ Open…**, or `open` with no path — a native file dialog
- **Recent Documents** on the start screen — one click
- **Drag a PDF onto the window**
- `open <path>` in the command box, or as a command-line argument

Paths survive spaces, `~`, quotes and Finder's backslash escaping. Without that
last part most files on a Mac could not be opened at all: splitting the line on
whitespace truncated `~/My Documents/report.pdf` at the first space, and nothing
expanded the `~`.

### Not done, and honestly so

- **The mockup's third Tool Wizard card says "Convert".** There is no conversion
  engine and none is planned, so the slot holds Extract instead — real, and the
  operation people reach for next to Merge.
- **The AI Tools tab is present and empty**, per §10: it needs a decision about
  what happens to a customer's document before it needs a feature.
- **`redact`, `sign` and `comment`** are recognised words that name their phase
  rather than doing anything. Redaction that only covers content is worse than
  none, and neither it nor signing has an engine API yet.
- **Notarization and code signing have not been run** — they need credentials.
  The scripts are written and the ordering is right; they have not been executed.
- **Interactive drawing covers line and circle.** Every other shape draws from
  typed coordinates (`l 0,0 100,100`, `ci 50,50 20`), which the kernel's parser
  already supports in full.
- **Ellipses, splines, hatches, dimensions and text** can be drawn but are not
  yet carried by the save format. They are *reported* on save rather than
  dropped silently — see `commit::Unstorable`.

## Commands

```
document   open close save saveas quit
view       page <n|next|prev|first|last>   zoom <percent|in|out|fit|width|actual>   rotate [90]
organize   extract <pages> <file>   import <file> [pages]   deletepage <pages>   insertpage   movepage <pages> <before>
review     highlight   note <text>
measure    calibrate <distance> [unit]   pagescale   measure <distance|area>
automate   record [name]   stop   replay <script.json>
edit       undo redo
drawing    every SIMLUX command — line, polyline, trim, fillet, offset, … — with the same aliases
```

Page operations are spelt out — `deletepage`, `insertpage`, `pagescale` —
because `delete`, `insert` and `scale` are live drawing commands in the kernel.
The guard test in `tests/command_box.rs` is what caught all six of those
collisions.

## PDFium

All four slices are fetched by `tools/fetch_pdfium.sh`, pinned to
chromium/7881 to match `pdfium-render`'s `pdfium_latest` feature — changing one
without the other is an ABI mismatch that crashes inside PDFium with no Rust
frame to look at.

```
pdfium-mac-arm64   pdfium-mac-x64   pdfium-win-x64   pdfium-linux-x64
```

They are fetched rather than committed. At runtime the library is looked for
next to the executable first (what packaging ships), then in this tree, then
`PAGIFY_PDFIUM_LIB` overrides both. Building for a target with no slice mapped
fails at compile time with a message naming it, deliberately — so it fails while
it is a chore rather than in month seven.
