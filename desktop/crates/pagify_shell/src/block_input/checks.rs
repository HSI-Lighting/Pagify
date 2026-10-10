//! The editor invariants: the context a check reads, the check list, the
//! entry points and every `cN_` check.
//!
//! Part of the `block_input` module — split out of the single file the
//! design review flagged (Phase 4, file splits).
use super::*;

pub(super) struct Ctx<'a> {
    pub(super) pb: &'a PageBlocks,
    pub(super) block: &'a Block,
    pub(super) specs: &'a [LineSpec],
    pub(super) texts: &'a [String],
}

impl Ctx<'_> {
    /// The run behind a member. C2 has proved it exists by the time anything else asks, but a guard
    /// must never panic.
    pub(super) fn run(&self, object: usize) -> Result<&TextRun, String> {
        self.pb.runs.get(&object).ok_or_else(|| format!("C2: object {object} is not a text object of this page"))
    }

    /// (line, object) of every member, line by line.
    pub(super) fn members(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.specs.iter().enumerate().flat_map(|(i, s)| s.objects.iter().map(move |&o| (i, o)))
    }
}

pub(super) type Check = fn(&Ctx) -> Result<(), String>;

/// In the order they are reported: C1 and C13 first (they are about the list of lines as a whole), then
/// C15 (a right-to-left block fails half the others for the same one reason, and that reason is the
/// one worth logging), then C2 to C11 in number order, so when a block breaks several of those the
/// lowest-numbered one shows, then the judgements about what the block is (C14, C16, C17).
pub(super) const CHECKS: [Check; 16] = [
    c1_lines,
    c13_no_line_dropped,
    c15_left_to_right_text,
    c2_real_objects,
    c3_each_once,
    c4_distinct_origins,
    c5_readable_text,
    c6_visible_upright,
    c7_left_to_right,
    c8_rows_and_breaks,
    c9_texts,
    c10_frozen,
    c11_rects,
    c14_not_vector_art,
    c16_twins,
    c17_not_a_row_of_cells,
];

/// Refuse a block the editor could corrupt the page with: `Err` names the broken invariant (C1..C17)
/// and the line or object, without any of the page's text. The caller falls back to the single-run
/// editor and logs the message.
pub fn check_editor_invariants(pb: &PageBlocks, block: usize) -> Result<(), String> {
    let Some(b) = pb.blocks.get(block) else {
        return Err(format!("C1: block {block} does not exist ({} blocks)", pb.blocks.len()));
    };
    check_block_invariants(pb, b)
}

/// [`check_editor_invariants`] for a block already in hand — a [`piece_block`] as readily as one of the
/// page's.
pub fn check_block_invariants(pb: &PageBlocks, b: &Block) -> Result<(), String> {
    let specs = editor_lines_of(pb, b);
    let texts = line_texts(pb, &specs);
    check_specs(pb, b, &specs, &texts)
}

/// [`check_editor_invariants`] on lines and texts already made (the seam the tests use to hand it
/// what [`editor_lines`] and [`line_texts`] would never produce).
pub fn check_specs(pb: &PageBlocks, block: &Block, specs: &[LineSpec], texts: &[String]) -> Result<(), String> {
    let cx = Ctx { pb, block, specs, texts };
    CHECKS.iter().try_for_each(|check| check(&cx))
}

pub(super) fn c1_lines(cx: &Ctx) -> Result<(), String> {
    if cx.block.lines.is_empty() && cx.specs.is_empty() {
        return Err("C1: the block has no lines".into());
    }
    for (i, s) in cx.specs.iter().enumerate() {
        if s.objects.is_empty() {
            let placeholder = s.placeholder.as_deref().is_some_and(|p| !p.trim().is_empty());
            if !(s.frozen && placeholder) {
                return Err(format!("C1: line {i} has no text object and is not a frozen outlined line with a placeholder"));
            }
        } else if s.placeholder.is_some() {
            return Err(format!("C1: line {i} has text objects and also a placeholder"));
        }
    }
    Ok(())
}

pub(super) fn c13_no_line_dropped(cx: &Ctx) -> Result<(), String> {
    if cx.specs.len() != cx.block.lines.len() {
        return Err(format!(
            "C13: the block has {} lines but {} were offered to the editor (frozen lines must stay in the list)",
            cx.block.lines.len(),
            cx.specs.len()
        ));
    }
    Ok(())
}

pub(super) fn c2_real_objects(cx: &Ctx) -> Result<(), String> {
    for (i, o) in cx.members() {
        if o == usize::MAX {
            return Err(format!("C2: line {i} names usize::MAX, the drawn-word placeholder, not an object"));
        }
        match cx.pb.runs.get(&o) {
            None => {
                let what = if cx.pb.shapes.iter().any(|s| s.object == o) { "a drawn path or group, not text" } else { "not a text object with area on this page" };
                return Err(format!("C2: line {i} names object {o}, which is {what}"));
            }
            Some(r) if !has_area(&r.rect) => return Err(format!("C2: line {i} names object {o}, which has no area at all")),
            Some(_) => {}
        }
    }
    Ok(())
}

pub(super) fn c3_each_once(cx: &Ctx) -> Result<(), String> {
    let mut seen: HashMap<usize, usize> = HashMap::new();
    for (i, o) in cx.members() {
        if let Some(first) = seen.insert(o, i) {
            return Err(if first == i { format!("C3: object {o} appears twice in line {i}") } else { format!("C3: object {o} is in line {first} and again in line {i}") });
        }
    }
    Ok(())
}

pub(super) fn c4_distinct_origins(cx: &Ctx) -> Result<(), String> {
    let mut points: Vec<(f32, f32, usize)> = Vec::new();
    for (_, o) in cx.members() {
        if cx.pb.excluded.shadowed.contains(&o) {
            return Err(format!("C4: object {o} is a shadowed twin of an earlier object, and a twin is never a member"));
        }
        let r = cx.run(o)?;
        points.push((r.origin.x, r.origin.y, o));
    }
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)));
    for (i, a) in points.iter().enumerate() {
        for b in &points[i + 1..] {
            if b.0 - a.0 > SAME_ORIGIN_PT {
                break;
            }
            let d = (b.0 - a.0).hypot(b.1 - a.1);
            if d <= SAME_ORIGIN_PT {
                return Err(format!("C4: objects {} and {} share a baseline origin ({d:.2} pt apart): apply would resolve both to one operator", a.2, b.2));
            }
        }
    }
    Ok(())
}

pub(super) fn c5_readable_text(cx: &Ctx) -> Result<(), String> {
    for (_, o) in cx.members() {
        if cx.run(o)?.text.trim().is_empty() {
            return Err(format!("C5: object {o} has blank text (a font with no ToUnicode, or a faux-bold twin)"));
        }
    }
    Ok(())
}

pub(super) fn c6_visible_upright(cx: &Ctx) -> Result<(), String> {
    for (_, o) in cx.members() {
        let r = cx.run(o)?;
        if r.color.a == 0 {
            return Err(format!("C6: object {o} is invisible (alpha 0)"));
        }
        // (a non-finite rect has no area and C2 has already refused it)
        if !r.origin.x.is_finite() || !r.origin.y.is_finite() || !r.size.is_finite() {
            return Err(format!("C6: object {o} has non-finite geometry"));
        }
        if r.size <= 0.0 {
            return Err(format!("C6: object {o} has size {} (an effective size of 0 is a rotated or collapsed run)", r.size));
        }
        let style = cx.pb.styles.get(&o);
        if !upright(style) {
            let axis = style.map_or((1.0, 0.0), |s| s.axis);
            return Err(format!("C6: object {o} is rotated (text axis {:.2},{:.2})", axis.0, axis.1));
        }
        if looks_rotated(&r.rect, r.text.trim().chars().count()) {
            return Err(format!("C6: object {o} looks rotated (its box is taller than it is wide)"));
        }
    }
    Ok(())
}

pub(super) fn c7_left_to_right(cx: &Ctx) -> Result<(), String> {
    for (i, s) in cx.specs.iter().enumerate() {
        for pair in s.objects.windows(2) {
            let (a, b) = (left_of(cx.run(pair[0])?), left_of(cx.run(pair[1])?));
            if b < a {
                return Err(format!("C7: line {i} is not left to right: object {} starts at {b:.1}, left of object {} at {a:.1}", pair[1], pair[0]));
            }
        }
    }
    Ok(())
}

/// How much of the horizontal stretch `lo..hi` the outlined words of line `line` fill: the length of
/// the union of their boxes' x extents, clipped to it. Only asked for when a gap is already wider than
/// a line may span, so the one pass over the page's paths is rare.
pub(super) fn drawn_width_between(cx: &Ctx, line: usize, lo: f32, hi: f32) -> f32 {
    let Some(l) = cx.block.lines.get(line) else { return 0.0 };
    if l.outlined.is_empty() || hi <= lo {
        return 0.0;
    }
    let wanted: HashSet<usize> = l.outlined.iter().copied().collect();
    let mut spans: Vec<(f32, f32)> = cx
        .pb
        .shapes
        .iter()
        .filter(|s| s.depth == 0 && wanted.contains(&s.object) && finite_rect(&s.rect))
        .map(|s| (s.rect.left.min(s.rect.right).max(lo), s.rect.left.max(s.rect.right).min(hi)))
        .filter(|(a, b)| b > a)
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut filled = 0.0;
    let mut reach = lo;
    for (a, b) in spans {
        filled += (b - a.max(reach)).max(0.0);
        reach = reach.max(b);
    }
    filled
}

pub(super) fn c8_rows_and_breaks(cx: &Ctx) -> Result<(), String> {
    // per line: the baseline of its first object and its em; `None` for a line with no text object,
    // which has no measured baseline (the detector estimates one from the rect: an estimate is not
    // something to hold a margin of 0.4 em against)
    let mut rows: Vec<Option<(f32, f32)>> = Vec::with_capacity(cx.specs.len());
    for (i, s) in cx.specs.iter().enumerate() {
        if s.objects.is_empty() {
            rows.push(None);
            continue;
        }
        let first = cx.run(s.objects[0])?;
        let mut em = 0.0f32;
        for &o in &s.objects {
            em = em.max(cx.run(o)?.size);
        }
        for &o in &s.objects[1..] {
            let d = (cx.run(o)?.origin.y - first.origin.y).abs();
            if d > ROW_EM * em {
                return Err(format!("C8: line {i} mixes baselines: object {o} sits {d:.1} pt from the line's first object"));
            }
        }
        // Frozen lines too: a currency sign drawn as a path froze a whole receipt row, and 12.7 em
        // between its cells went unmeasured. What a line's outlined words fill of a gap does not
        // count as gap: a hole where a long outlined word sits is not two columns.
        let mut right = f32::MIN;
        for pair in s.objects.windows(2) {
            let (a, b) = (cx.run(pair[0])?, cx.run(pair[1])?);
            right = right.max(a.rect.left.max(a.rect.right));
            let gap = left_of(b) - right;
            let most = MAX_LINE_GAP_EM * a.size.max(b.size);
            if gap > most {
                let open = gap - drawn_width_between(cx, i, right, left_of(b));
                if open > most {
                    return Err(format!("C8: line {i} spans a {open:.1} pt gap between objects {} and {}: two columns in one line", pair[0], pair[1]));
                }
            }
        }
        rows.push(Some((first.origin.y, em)));
    }
    for i in 1..rows.len() {
        match (rows[i - 1], rows[i]) {
            (Some(above), Some(here)) => {
                let em = here.1.max(above.1);
                if here.0 - above.0 <= LINE_BELOW_EM * em {
                    return Err(format!("C8: line {i} does not lie below line {}", i - 1));
                }
            }
            // two lines drawn as shapes: nothing about either is measured (both are the detector's
            // estimates), and nothing in them is ever written, so there is nothing to hold to an order
            (None, None) => {}
            // a line drawn as shapes beside a line of text: ordered by what is measured, the rects. A
            // line below another starts lower and ends lower; a symbol standing beside a label (the
            // same rows, or a rect reaching above it) is not a line below it. (Rects that are not
            // finite are C11's to refuse.)
            _ => {
                let (above, here) = (&cx.specs[i - 1].rect, &cx.specs[i].rect);
                if finite_rect(above) && finite_rect(here) {
                    let (above, here) = (normalised(above), normalised(here));
                    if !(here.top > above.top && here.bottom > above.bottom) {
                        return Err(format!("C8: line {i} does not lie below line {}: a line drawn as shapes has to start and end lower than the line above it", i - 1));
                    }
                }
            }
        }
    }
    if let Some(i) = cx.texts.iter().position(|t| t.contains('\n') || t.contains('\r')) {
        return Err(format!("C8: the text of line {i} holds a line break"));
    }
    let buffer_lines = cx.texts.join("\n").split('\n').count();
    if buffer_lines != cx.specs.len() {
        return Err(format!("C8: {} line texts joined by newlines make {buffer_lines} lines, not {}", cx.texts.len(), cx.specs.len()));
    }
    Ok(())
}

pub(super) fn c9_texts(cx: &Ctx) -> Result<(), String> {
    if cx.texts.len() != cx.specs.len() {
        return Err(format!("C9: {} line texts for {} lines", cx.texts.len(), cx.specs.len()));
    }
    for (i, (s, t)) in cx.specs.iter().zip(cx.texts).enumerate() {
        if s.objects.is_empty() {
            // (C1 has already refused a blank placeholder)
            if s.placeholder.as_deref() != Some(t.as_str()) || t.ends_with('-') {
                return Err(format!("C9: the text of placeholder line {i} must be its placeholder and must not end in '-'"));
            }
        } else {
            let mut want = String::new();
            for &o in &s.objects {
                want.push_str(&sanitise(&cx.run(o)?.text));
            }
            if *t != want {
                return Err(format!(
                    "C9: the text of line {i} is not the plain concatenation of its {} objects' text ({} characters, expected {})",
                    s.objects.len(),
                    t.chars().count(),
                    want.chars().count()
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn c10_frozen(cx: &Ctx) -> Result<(), String> {
    for (i, (s, l)) in cx.specs.iter().zip(&cx.block.lines).enumerate() {
        if !l.outlined.is_empty() && !s.frozen {
            return Err(format!("C10: line {i} carries {} outlined words but is not frozen: apply would rewrite it around paths that stay drawn", l.outlined.len()));
        }
    }
    Ok(())
}

pub(super) fn c11_rects(cx: &Ctx) -> Result<(), String> {
    for (i, s) in cx.specs.iter().enumerate() {
        if !finite_rect(&s.rect) {
            return Err(format!("C11: the rect of line {i} is not finite"));
        }
        if s.rect.left > s.rect.right || s.rect.top > s.rect.bottom {
            return Err(format!("C11: the rect of line {i} is not normalised (left > right or top > bottom)"));
        }
    }
    Ok(())
}

/// **Vector art is not a paragraph.** A QR code under a label, a chart, a CAD symbol: paths the size of
/// a word that the detector bridges into the label's lines as "outlined words", one phantom line per
/// row of modules. Such a block has more lines drawn as shapes than lines with text. (A real paragraph
/// can have lines drawn as outlines too, a line with a ligature in it, and the datasheet has one with
/// two of its four lines drawn as outlines: only a majority of them is vector art. A tie is not: the
/// literal "at least as many" would refuse that paragraph, and, in a reading without the thin hyphen
/// that ends one of its drawn lines, it is exactly a tie.) The detector is the first place to stop it
/// bridging them; this is the second.
///
/// A block with no text object at all is not judged: there is nothing in it to open or write, and no
/// click can reach it. "Fewer than three text objects with three or more drawn lines" is a case of
/// this rule (it has at most two lines with text), not a second rule.
pub(super) fn c14_not_vector_art(cx: &Ctx) -> Result<(), String> {
    let with_text = cx.specs.iter().filter(|s| !s.objects.is_empty()).count();
    let drawn = cx.specs.len() - with_text;
    if with_text > 0 && drawn > with_text {
        return Err(format!("C14: {drawn} of the block's {} lines are drawn shapes and only {with_text} hold text: vector art, not a paragraph", cx.specs.len()));
    }
    Ok(())
}

/// Whether `c` is a strong right-to-left character: Hebrew, Arabic, Syriac, Thaana, N'Ko, Samaritan,
/// Mandaic and their extensions and presentation forms, the right-to-left scripts of the supplementary
/// planes (Phoenician, Kharoshthi, Adlam, ...), and the marks that make text run right to left.
/// Arabic-Indic digits are in the Arabic block: a block holding them comes from a right-to-left document.
pub(super) fn is_rtl(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF | 0x200F | 0x202B | 0x202E | 0x2067)
}

/// **No right-to-left text.** The detector lays a right-to-left line out by geometry but its margin
/// rules (indent, short last line) read the left edge, the editor shows the visual order of the
/// objects (left to right: the words of a Farsi line in reverse), and apply writes the typed line into
/// the leftmost object, which is the line's last word. Refused, so the click opens the single run.
pub(super) fn c15_left_to_right_text(cx: &Ctx) -> Result<(), String> {
    for (i, o) in cx.members() {
        // (a member that is not a run is C2's to refuse)
        if cx.pb.runs.get(&o).is_some_and(|r| r.text.chars().any(is_rtl)) {
            return Err(format!("C15: line {i} holds right-to-left text (object {o}): the editor reads lines left to right"));
        }
    }
    Ok(())
}

/// **Twins are real.** Apply removes them with their line, so a wrong one deletes something that is
/// not a copy of anything: each must be a blank or shadowed text object of this page that no block
/// uses as a member, listed once in one line, and coincide with an object of that line.
pub(super) fn c16_twins(cx: &Ctx) -> Result<(), String> {
    let members: HashMap<usize, usize> = cx.members().map(|(i, o)| (o, i)).collect();
    let mut listed: HashMap<usize, usize> = HashMap::new();
    for (i, s) in cx.specs.iter().enumerate() {
        for &t in &s.twins {
            if let Some(line) = members.get(&t) {
                return Err(format!("C16: object {t} is a member of line {line} and also a twin in line {i}"));
            }
            if cx.pb.by_object.contains_key(&t) {
                return Err(format!("C16: line {i} lists object {t} as a twin, but it is a member of another block"));
            }
            let Some(twin) = cx.pb.runs.get(&t) else {
                return Err(format!("C16: line {i} lists object {t} as a twin, which is not a text object with area of this page"));
            };
            if !matches!(cx.pb.excluded.reason(t), Some("blank" | "shadowed")) {
                return Err(format!("C16: line {i} lists object {t} as a twin, but it is neither blank nor shadowed"));
            }
            if let Some(first) = listed.insert(t, i) {
                return Err(if first == i { format!("C16: twin {t} is listed twice in line {i}") } else { format!("C16: twin {t} is listed in line {first} and again in line {i}") });
            }
            let twin_box = normalised(&twin.rect);
            let coincides = s.objects.iter().filter_map(|m| cx.pb.runs.get(m)).any(|m| {
                (m.origin.x - twin.origin.x).hypot(m.origin.y - twin.origin.y) <= SAME_ORIGIN_PT && edge_gaps(&normalised(&m.rect), &twin_box).iter().all(|g| *g <= TWIN_RECT_PT)
            });
            if !coincides {
                return Err(format!("C16: line {i} lists object {t} as a twin, but it coincides with none of the line's objects"));
            }
        }
    }
    Ok(())
}

/// A list item's number or bullet standing alone as a text object: "1." "2)" "(a)" "[12]" "iv." (at most five
/// characters, closed by a full stop or a bracket, with a letter or digit in it) or one bullet glyph or dash.
pub(super) fn is_list_marker(text: &str) -> bool {
    let t = text.trim();
    let n = t.chars().count();
    match t.chars().last() {
        Some(last) if (2..=5).contains(&n) => matches!(last, '.' | ')' | ']') && t.chars().any(char::is_alphanumeric),
        Some(last) if n == 1 => "•·▪◦●‣∙-–—*".contains(last),
        _ => false,
    }
}

/// **A short block with a wide gap inside a line is a row of cells, not a sentence.** Retyping a line
/// writes all of its words into the first piece and removes the others, so two table cells that the
/// detector let share a line (an amount and the date beside it, 1.9 em apart and no rule between) would
/// be run together and the second one deleted. A paragraph's justified lines have wide gaps of their own
/// (1.7 em in the datasheet's narrow columns) and every other line says they are justified, so only
/// blocks of at most [`CELL_ROW_MAX_LINES`] lines are judged; a frozen line is never written, so it is
/// not either. Measured on 55 real pages (quotations, purchase orders, invoices, a lux report, datasheets),
/// against two hand-labelled truths: of the 22 blocks on the quotation, purchase-order and invoice pages
/// that put cells of one row into one line, 19 were opened by the app and now all 22 are refused (the click
/// then opens the one word, as it always did); no block that the labelers call one block is lost to it
/// (the numbered terms are exempt, see below) and the datasheet loses no paragraph. A list item's own number ([`is_list_marker`]) stands apart from its words and is exempt.
pub(super) fn c17_not_a_row_of_cells(cx: &Ctx) -> Result<(), String> {
    if cx.specs.len() > CELL_ROW_MAX_LINES {
        return Ok(());
    }
    for (i, s) in cx.specs.iter().enumerate() {
        if s.frozen {
            continue;
        }
        let mut right = f32::MIN;
        for (k, pair) in s.objects.windows(2).enumerate() {
            let (a, b) = (cx.run(pair[0])?, cx.run(pair[1])?);
            right = right.max(a.rect.left.max(a.rect.right));
            let gap = left_of(b) - right;
            // a list item's own number stands 1.5 em before its words ("1." and the clause, in the terms of every
            // quotation and order): the item is one block, and it is the number's line that gets retyped whole
            if k == 0 && is_list_marker(&a.text) {
                continue;
            }
            if gap >= CELL_GAP_EM * a.size.max(b.size) {
                return Err(format!("C17: line {i} has a {gap:.1} pt gap between objects {} and {}: cells of a row, not one sentence", pair[0], pair[1]));
            }
        }
    }
    Ok(())
}
