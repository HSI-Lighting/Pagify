# Compact UI restyle — light/dark theme + denser ribbon

**Built 1 October 2026, same day as the spec below.** Four things changed
from the plan as written, each found while actually wiring it up rather than
guessed in advance — the spec is kept as written beneath this note, since the
reasoning in it is still the reasoning for what shipped, just corrected here
where the concrete code changed the answer:

- **§1's persistence lives in `pagify_shell::appearance`, not `pagify_app`.**
  The spec reasoned appearance has no document meaning so it belongs in
  `pagify_app`; building it surfaced the sharper reason to put it in
  `pagify_shell` instead — that crate's own charter (`lib.rs`) is "if logic
  can be tested without a window, it must live here," and `serde_json` turned
  out to be a `pagify_app` *dev*-dependency only (real code there never needed
  JSON), so this also avoided adding a new real dependency for one struct.
  `pagify_shell::recent`'s own `path()`/`load_from()`/`save_to()` split
  (separate from the `state_dir()`-backed `path()`/`load()`/`save()`) is
  followed exactly, for the same reason: `recent.rs`'s and `signatures.rs`'s
  own tests use explicit temp paths rather than mocking `state_dir()`'s
  environment variables, which isn't safely parallel-test-friendly.
- **§4's `button_groups()` became `button_group_starts()`.** Restructuring
  all 15 tabs' `&[Tool]` into `&[&[Tool]]` for the sake of the one tab
  (`Draw`) that actually has a grouping reference risked the other 14 for no
  reason. An index list of where a divider goes — pinned to the tools it
  names by `draws_divider_group_starts_point_at_the_tools_named_in_their_own_
  comment`, so a future edit to `Draw`'s own list can't silently drift the
  divider — gets the same result touching one tab instead of fifteen. The
  grouping also landed on `Draw`, not `Home`: `Home`'s actual tools (zoom,
  text/object editing, page operations) don't match the mockup's drawing-tool
  clusters at all, while `Draw`'s already-existing tools (Line, Circle,
  Rectangle, Trim, Calibrate, Distance…) are close to a 1:1 match.
- **`TOOL_WIDTH`/`TOOL_HEIGHT` stayed at 66×58, not the mockup's own
  density.** `no_ribbon_label_overflows_its_own_button` — a new test using
  `tool_button`'s own layout calls — is what actually found the floor: this
  app's longest labels ("Run Form Field Recognition") clip at the mockup's
  own tighter sizing regardless of height. 66 wide (from 82) is what survives
  every real label in the ribbon without clipping.
- **The command bar follows the toggle; it doesn't stay pinned dark.** The
  mockup keeps its status bar dark even in the light screenshot, and §6 below
  says so — but every colour inside this bar (`theme::ink()`,
  `theme::violet_bright()`, `theme::danger()`, …) is themed, and pinning only
  the *background* dark while those stay theme-following would put
  light-mode text on a dark-mode background in light mode — illegible, not
  cosmetic. Making the bar's own text colours *also* bypass the toggle was
  the other option; it was declined as a second, parallel "always dark" path
  through the exact code this restyle is meant to simplify, for one bar's
  worth of fidelity to a single mockup screenshot. §6's own stat readout also
  dropped the searchable-character count: it comes from `Session::classify`,
  which walks the page's content — fine once, on request, not sixty times a
  second for a passive number nobody asked to see live. Page count (free,
  already read everywhere) shipped in its place.

Written 1 October 2026 against `pagify-desktop-windows`, scoping the redesign
in `C:\Users\hsili\Desktop\stitch_compact_pdf_app_ui\stitch_compact_pdf_app_ui\
pagify_pdf_compact_{light,dark}_theme\{code.html,screen.png}` — two
Stitch-generated mockups the user asked to be implemented.

**What this is not.** The mockups also show a CAD-specific "Element
Properties" panel (stroke/fill/hatch/layer editing), a global search bar
across commands/annotations/layers, a cloud-sync status badge, and an
author-tracking/"lock object" workflow. Those are new features, not restyling
— the right-hand panel alone is the single biggest piece of new work in
either screenshot — and are explicitly out of scope here, to be scoped as
their own spec(s) later if wanted. This spec is the visual restyle only: a
light/dark theme pair and a denser ribbon/toolbar/thumbnail-rail/status-bar
layout, on the app's existing feature set. The rebrand to "Pagify CAD" shown
in the mockups is also declined — the app keeps its current name.

The two screenshots disagree with each other on some details (different tab
labels — "Draw CAD" vs "Draw" — different open tabs, different selected
object types), which confirms they're a style direction plus two example
screens rather than one literal pixel spec. Where they disagree, this spec
picks whichever is closer to the app's own existing structure, named
explicitly below.

## 1. Theme mechanism: dark (today) + a new light palette, switchable at runtime

**The constraint.** `theme.rs`'s colours are `pub const Color32` values —
`theme::VIOLET`, `theme::PAPER`, and so on — referenced as plain compile-time
constants at roughly 150 call sites (123 in `main.rs`, 27 in `home.rs`; counted
directly via `grep -o "theme::[A-Z_]*" | sort | uniq -c`, 1 October 2026). Rust
constants cannot be runtime-conditional, so a genuine toggle means touching
every one of those sites — there is no way around it, and no cleverness in
`theme.rs` alone substitutes for it.

**The change:** every `pub const NAME: Color32 = ...;` becomes
`pub fn name() -> Color32 { ... }`, reading a module-level
`static MODE: AtomicU8` (`0 = Dark, 1 = Light`) that
`theme::set_mode(mode)` flips. Every call site changes from `theme::VIOLET`
to `theme::violet()` — mechanical, and a miss is a compile error (`Color32`
vs `fn() -> Color32` do not unify), not a silent bug. `theme::icon_tile`'s
`top`/`bottom` parameters are unaffected; callers already pass it resolved
colours.

Rejected: threading a `Mode` parameter through every drawing function
instead of a flag. "More proper" in the abstract, but means changing the
signature of every function that currently reaches for `theme::SOMETHING` —
a far larger diff for no behavioural difference, and out of step with how
this file already reads other cross-cutting UI state (`self.organize_open`,
`self.show_thumbs` are read, not threaded, from wherever they're needed).

**The two palettes.** `theme::dark` keeps today's exact values (zero visual
change for anyone who never opens the toggle). `theme::light` is built from
the mockup's own Tailwind config
(`pagify_pdf_compact_light_theme/code.html` lines 13–27): brand purple
`#7c3aed`/`#8b5cf6`/`#6d28d9` for violet/violet-bright/violet-deep — notably
closer to identical than different, since the app's own
`VIOLET = 0x8B5CF6` already matches the mockup's `brand-500` exactly —
and the slate scale (`#f8fafc`…`#0f172a`) for paper/chrome/panel/ink. Values
for constants the mockup's CSS doesn't name directly (`SNAP`, `MARKUP`,
`CHEQUER_LIGHT/DARK`) are chosen to keep the same relative contrast against
the new light backgrounds that they have against the dark ones today, since
no mockup page exercises them.

`egui::Visuals` (`theme::apply`) is rebuilt from whichever palette `MODE`
currently names — `apply` already takes `ctx`, so it becomes
`apply(ctx)` reading the flag rather than taking a palette argument, keeping
every existing call site (`theme::apply(&ctx)` at startup) unchanged.

## 2. Persistence: remembered across launches

A new file, `%APPDATA%\Pagify\appearance.json`, holding `{"mode":"dark"}` or
`{"mode":"light"}` — the exact shape `pagify_shell::recent::Recent` and
`pagify_shell::signatures` already use (`path()`/`load()`/`save()` over
`pagify_shell::state::state_dir()` + `state::write_own()`; see
`crates/pagify_shell/src/recent.rs:91-111`). Lives in `pagify_app`, not
`pagify_shell` — appearance is a UI preference with no document semantics,
unlike recents or signatures, so it doesn't belong in the shell crate; it
only *reuses* the shell's already-public `state_dir`/`write_own` helpers.

Loaded once in `PagifyApp::new`, before the first `theme::apply(&ctx)` call,
so the very first frame already paints in the remembered mode — no
dark-then-flash-to-light on startup. A missing or corrupt file defaults to
Dark, the same "never fail to start over a preferences file" rule
`Recent::load` already follows, and the right default for not surprising
anyone who upgrades without ever touching the new toggle.

## 3. The toggle: View tab

A new button in `Tab::View`'s button list (`fn buttons`, the `Tab::View =>
&[...]` arm), rendered through the existing `tool_button` helper exactly
like every other ribbon button, command `"appearance"` (parsed as a new
`Verb::ToggleAppearance` — mirrors how `Verb::Thumbnails` was added for
Organize this session: a `verbs.rs` enum variant, a parser arm, removed from
`PLANNED`). Its "active" highlight (the same violet fill `tool_button` already
gives any armed/standing-choice button, like today's `fill` toggle) shows
when the current mode is Light, so the button itself reads as "Light theme:
on/off" without needing separate light/dark icons.

## 4. Ribbon/toolbar: compacting and grouping

**Sizing.** `TOOL_WIDTH`/`TOOL_HEIGHT` (currently 82×58, `main.rs:2541-2542`)
shrink toward the mockup's own density — its `.tool-btn` is `padding: 0.25rem
0.4rem` with a `1.05rem` icon and `0.65rem` label, which at a typical 96dpi
scale works out to roughly 56×40. Exact target values get tuned against the
real font metrics during implementation rather than fixed here, since
`pagify-icons.ttf`'s glyph widths aren't the same as Lucide's.

**Grouping.** Today, `fn buttons(self) -> &'static [Tool]` returns one flat
slice per tab, drawn as one continuous `horizontal_wrapped` row
(`main.rs:15685-15706`) — the only existing divider is the one already
separating `tab.leading()` (Select/Hand, the standing tools) from the tab's
own buttons (`main.rs:15677-15684`), which stays exactly as is. `buttons`
becomes `fn button_groups(self) -> &'static [&'static [Tool]]`; the drawing
loop at `main.rs:15685` iterates groups, drawing `ui.separator()` with the
same spacing between groups that already separates `leading()` from the rest.
Every existing `Tab::X => &[...]` arm becomes `Tab::X => &[&[...], &[...],
...]`, grouped by the same clusters the mockup's own `data-purpose` comments
name for Home (navigation / CAD markup / measure / annotate / layers,
`pagify_pdf_compact_light_theme/code.html:179-276`) — other tabs keep their
current single group (nothing in either mockup screen shows Convert, Edit,
Comment, Form, Protect, or PagiSign open, so there's no reference for how
*their* tools cluster).

**Icons stay Pagify's own** (`pagify-icons.ttf`), per the earlier decision —
each `tool_button` call keeps its current glyph; only layout and colour
change.

## 5. Thumbnail rail: card restyle

The existing cards already show a page image, a page number, and a label
(`main.rs:15694-15735`, rewritten earlier this session to add Organize's
select/drag support — see `draw_thumbnail_cell`). Restyled, not
rebuilt: the active-page violet outline it already draws
(`ui.painter().rect_stroke(..., theme::violet())`) gains the mockup's ring
(a second, wider, lower-opacity stroke outside the first —
`ring-2 ring-brand-100` in the mockup, `code.html:326`), and the per-page
label gains a second line for the page's size/format (`"A1 Landscape"`,
`"A4"` in the mockup) where the document exposes it — `Session::page_crop`
already returns page dimensions in points; formatting those as a paper-size
label (A4/A3/A1/…) is a small new pure function, the same "pure and
testable" pattern `nearest_drop`/`apply_organize_click` already established
this session. This styling is shared with the Organize grid's own cells
through `draw_thumbnail_cell`/`draw_organize_grid`, so both surfaces pick it
up from one change, not two.

## 6. Status/command bar: restyle + stat readouts

The command bar already sits at the bottom (`main.rs:15716-15718`,
`egui::Frame::new().fill(theme::CHROME)`) — this is "the single line the
mockup draws" the code's own comment already references, confirming the
command bar's *position* was already built from an earlier mockup. What
changes is colour (toward the mockup's dark-slate/monospace look, in both
themes — the mockup keeps its status bar dark even in the light screenshot,
`code.html:576`, so this one bar does not follow the light/dark toggle) and
the addition of the stat readouts the mockup shows beside it: page count
(`doc.page_count`, already read everywhere) and searchable-character count
(`Session::classify`'s `verdict.chars`, already computed for the
`textlayer`/`extracttext` messages this session's own work touched — see
`fn report_text_layer`). "Vector Shape Mode: N primitives rendered" is
declined — `glyph_paths`/`verdict.paths` exist but surfacing a raw primitive
count has no established meaning in this app's own vocabulary (CAD
primitive count is the *other* mockup's elaborate Properties panel's idea of
useful, not Pagify's), and wiring it live on every frame costs doc
re-classification for a number nobody asked to see.

## Testing

- `theme::dark()`/`theme::light()` accessors: a test per constant that
  `set_mode` actually changes what each function returns (mirrors how
  `raster_scale`/`quantise_zoom` already get direct unit tests rather than
  only being exercised through the UI).
- `appearance.json` round-trip: save, reload, matches — same shape as
  whichever test already covers `Recent::save`/`Recent::load`.
- The paper-size-label function: pure, unit-tested directly against known
  point dimensions (A4, A3, A1, and a non-standard size falling back to
  raw mm) — same pattern as this session's `nearest_drop`/
  `detail_tile_covers` tests.
- `button_groups` for every tab: a test that every `Tool` currently reachable
  through `buttons()` is still reachable through some group in
  `button_groups()` — the one regression this refactor could silently cause
  is a tool dropped while regrouping.
- Full `cargo test -p pagify_app --release` — zero new regressions beyond
  the known-flaky `editing_is_fast_on_a_real_busy_page` (ambient load) and
  the three pre-existing cross-reference-stream failures already present on
  this branch.
- Manual: toggle the View-tab button, confirm every screen (ribbon, thumbnail
  rail, command bar, canvas) repaints correctly in both modes with no panel
  stuck in the other mode's colours; relaunch and confirm the remembered mode
  survives restart.

## Out of scope (explicitly, for whoever scopes it next)

- The CAD "Element Properties" panel (stroke/fill/hatch/layer editing,
  dimensions, author tracking, lock object) — real new functionality, the
  largest single piece of either mockup, and was decomposed out in the first
  brainstorming question of this project.
- The top search bar across commands/annotations/layers.
- The cloud-sync status badge.
- Rebranding to "Pagify CAD".
- Per-tab tool grouping for any tab besides Home, beyond "keep the single
  group it has today" — no reference exists for how Convert/Edit/Comment/
  Form/Protect/PagiSign's own tools should cluster.
