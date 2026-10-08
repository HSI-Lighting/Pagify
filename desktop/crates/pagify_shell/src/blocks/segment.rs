//! Segmentation: splitting runs at their breaks, the row-geometry facts
//! each chain needs, the per-chain segmentation itself, and turning a run
//! into a block draft.
//!
//! Part of the `blocks` module — split out of the single file the design
//! review flagged (Phase 4, file splits).
use super::*;

pub(super) fn split_by(run: Vec<usize>, why: &'static str, mut brk: impl FnMut(usize) -> Option<&'static str>) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    let mut cur = vec![run[0]];
    let mut why = why;
    for i in 0..run.len() - 1 {
        if let Some(r) = brk(i) {
            out.push((std::mem::take(&mut cur), std::mem::replace(&mut why, r)));
        }
        cur.push(run[i + 1]);
    }
    out.push((cur, why));
    out
}

/// Margins: the left edge moves by more than `margin_shift` em from one stretch of lines to the next
/// AND the right edge moves with it, by at least half as much (both stretches at least two lines long). That is a block quote;
/// text wrapped round a figure moves one edge only, a hanging indent or a first-line indent is one
/// line long. `tl` are the left edges of the lines (past list markers), `r` their right edges.
/// Returns the positions where a new stretch (a block) starts.
pub(super) fn margin_starts(tl: &[f64], r: &[f64], em: f64, p: &Params) -> Vec<usize> {
    let n = tl.len();
    let tol = 0.5 * em;
    let mut levels: Vec<(usize, usize)> = Vec::new(); // [start, end)
    let mut s = 0;
    for i in 1..n {
        if (tl[i] - tl[i - 1]).abs() > tol {
            levels.push((s, i));
            s = i;
        }
    }
    levels.push((s, n));
    let mut starts = Vec::new();
    for w in levels.windows(2) {
        let (x, y) = (w[0], w[1]);
        if x.1 - x.0 < 2 || y.1 - y.0 < 2 {
            continue;
        }
        let shift = p.margin_shift as f64 * em;
        let right = |lv: (usize, usize)| r[lv.0..lv.1].iter().cloned().fold(f64::MIN, f64::max);
        let left_move = (tl[y.0] - tl[x.0]).abs();
        let right_move = (right(y) - right(x)).abs();
        // both edges move, the right one about as far as the left one (the lines of ragged text end
        // anywhere: the longest of five lines differs by several em from the longest of the next five)
        if left_move > shift && right_move > shift && right_move >= 0.5 * left_move {
            starts.push(y.0);
        }
    }
    starts
}

/// A horizontal rule right under the line (a ruled cell ends here).
pub(super) fn rule_under(furn: &Furniture, pc: &Piece, p: &Params) -> bool {
    hrule_in(furn, pc.base + 0.05 * pc.size, pc.base + p.band_below as f64 * pc.size, pc.l, pc.r, 0.5)
}

pub(super) fn segment(items: &[It], pieces: &[Piece], link: &Link, furn: &Furniture, fonts: &Fonts, p: &Params, body: f64) -> Vec<BlockDraft> {
    let mut blocks: Vec<BlockDraft> = Vec::new();
    for head in 0..pieces.len() {
        if link.pred[head].is_some() {
            continue;
        }
        let mut chain = vec![head];
        while let Some(nx) = link.succ[*chain.last().unwrap()] {
            chain.push(nx);
        }
        let n = chain.len();
        let pc = |i: usize| &pieces[chain[i]]; // the piece at chain position i; the runs below hold positions

        let RowGeometry { cell, is_item, tl, rights, margins } =
            row_geometry(items, pieces, &chain, furn, p);

        // stage 1: pairwise facts that need no statistics: size, style, margin
        let runs: Vec<Run> = split_by((0..n).collect(), link.head_why[head], |i| {
            let (a, b) = (pc(i), pc(i + 1));
            let size_changes = a.pure && b.pure && (a.size - b.size).abs() > (p.size_break as f64 * a.size.min(b.size)).max(0.075 * body);
            // a line with no dominant look: judged by the looks it has (nothing in common = a different text)
            let mixed = if p.mixed_looks && !(a.pure && b.pure) { looks_differ(a, b, fonts, p, body) } else { None };
            if p.size_rule && (size_changes || mixed == Some("size")) {
                Some("size")
            } else if p.style_rule && !cell && ((a.pure && b.pure && style_differs(a, b, fonts, p, body)) || mixed == Some("style")) {
                Some("style")
            } else if margins.contains(&(i + 1)) {
                Some("margin")
            } else {
                None
            }
        });

        // stage 2: within a run of one look, the structure: line pitch, list items, indents
        let mut runs2: Vec<Run> = Vec::new();
        for (run, why) in runs {
            if run.len() < 2 {
                runs2.push((run, why));
                continue;
            }
            let em0 = pc(run[0]).size;
            let bin = (p.edge_tol as f64 * em0).max(1e-9);
            // the typical pitch of the run: the median of its line pitches (up to `leading` em: anything
            // wider is a paragraph gap), 1.2 em when it has none; a run of at least four equal pitches
            // is trusted whatever they are up to `wide_leading` em (a double-spaced manuscript)
            let median = |v: &[f64]| match v.len() {
                k if k % 2 == 1 => v[k / 2],
                k => 0.5 * (v[k / 2 - 1] + v[k / 2]),
            };
            let mut all: Vec<f64> = run.windows(2).map(|w| pc(w[1]).base - pc(w[0]).base).filter(|d| *d > 0.5 * em0 && *d <= p.wide_leading as f64 * em0).collect();
            all.sort_by(|a, b| a.total_cmp(b));
            let plain: Vec<f64> = all.iter().copied().filter(|d| *d < p.leading as f64 * em0).collect();
            let typ_pitch = if all.len() >= 3 && all[all.len() - 1] - all[0] <= 0.3 * em0 {
                median(&all)
            } else if plain.is_empty() {
                1.2 * em0
            } else {
                // (the upper middle one when the count is even: the first pitch of a run is often the
                // taller line box of a mark or a larger first word)
                plain[plain.len() / 2]
            };
            let lefts: Vec<f64> = run.iter().map(|&i| tl[i]).collect();
            let (ml, ml_count) = mode_bin(&lefts, bin).unwrap_or((lefts[0], 1));
            let flush_edge = ml_count >= p.edge_lines && ml_count as f64 >= p.edge_dominance as f64 * run.len() as f64;
            // how many lines of the run sit on the left edge `x` (within a bin)
            let mut sorted_lefts = lefts.clone();
            sorted_lefts.sort_by(|a, b| a.total_cmp(b));
            let on_edge = |x: f64| sorted_lefts.partition_point(|&v| v <= x + bin) - sorted_lefts.partition_point(|&v| v < x - bin);
            let entries = p.edge_lines.saturating_sub(1); // (three entries make a reference list)
            let len = run.len();
            runs2.extend(split_by(run.clone(), why, |i| {
                let (a, b) = (pc(run[i]), pc(run[i + 1]));
                let em = b.size.max(a.size);
                if p.pitch_rule && (b.base - a.base) - typ_pitch > p.pitch_extra as f64 * em {
                    Some("pitch")
                } else if p.list_items && (is_item[run[i + 1]] || b.lead_script) {
                    Some("item")
                } else if p.indent_rule
                    && flush_edge
                    && tl[run[i + 1]] - ml > p.indent as f64 * em
                    && (tl[run[i]] - ml).abs() <= 0.5 * em
                    && i + 2 < len
                    && (tl[run[i + 2]] - ml).abs() <= 0.5 * em
                {
                    Some("indent")
                } else if p.hanging_items
                    && i + 2 < len
                    && tl[run[i]] - tl[run[i + 1]] > p.indent as f64 * em
                    && (tl[run[i + 2]] - tl[run[i]]).abs() <= 0.5 * em
                    && on_edge(tl[run[i + 1]]) >= entries
                    && 5 * (on_edge(tl[run[i + 1]]) + on_edge(tl[run[i]])) >= 4 * len
                {
                    // a flush line between two lines that hang in as far as each other, in a run whose
                    // lines sit on these two left edges (four in five): an entry starts. Ragged-left
                    // text has many left edges, a few of them shared by chance.
                    Some("item")
                } else {
                    None
                }
            }));
        }

        // stage 3: short lines, judged against the right margin of the column the run sits in: the lines
        // of the stretch between two margin moves that are not much wider than the run's own and of about
        // its size (a heading, a title or a paragraph across two columns says nothing about the margin of
        // one column); a run is justified when half of its lines reach that margin
        let mut seg_bounds: Vec<(usize, usize)> = Vec::new();
        let mut s = 0;
        for &m in &margins {
            seg_bounds.push((s, m));
            s = m;
        }
        seg_bounds.push((s, n));
        for (run, why) in runs2 {
            let (first, last) = (run[0], run[run.len() - 1]);
            let seg = seg_bounds.iter().position(|&(lo, hi)| first >= lo && first < hi).unwrap_or(0);
            let (lo, hi) = (seg_bounds[seg].0.max(first.saturating_sub(1000)), seg_bounds[seg].1.min(last + 1001));
            let mut widths: Vec<f64> = run.iter().map(|&i| pc(i).r - pc(i).l).collect();
            widths.sort_by(|a, b| a.total_cmp(b));
            let wide = 1.3 * widths[(widths.len() - 1) / 2];
            let mut sizes: Vec<f64> = run.iter().map(|&i| pc(i).size).collect();
            sizes.sort_by(|a, b| a.total_cmp(b));
            let size = sizes[(sizes.len() - 1) / 2];
            let bin = (p.edge_tol as f64 * size).max(1e-9);
            // (lines of another size, a title over an abstract, do not tell where this text ends)
            let col: Vec<f64> = (lo..hi).filter(|&i| (first..=last).contains(&i) || (pc(i).r - pc(i).l <= wide && (pc(i).size - size).abs() <= 0.1 * size)).map(|i| rights[i]).collect();
            // the right margin: the lines that end within a bin of the longest line of the column sit on
            // it, and it is where the shortest of them ends
            let max_r = col.iter().cloned().fold(f64::MIN, f64::max);
            let flush: Vec<f64> = col.iter().cloned().filter(|&r| r >= max_r - bin).collect();
            let mr = flush.iter().cloned().fold(max_r, f64::min);
            // justified: half of the run's lines (the last one counts: a paragraph that flows into the
            // next column ends on a full line) sit on the margin, and the margin is proved by enough
            // lines (`edge_lines` of the run or `justify_evidence` of the column): ragged lines end
            // together by chance, the longest line always sits on the margin
            let reaching = run.iter().filter(|&&i| rights[i] >= max_r - bin).count();
            let justified = run.len() >= 3 && reaching * 2 >= run.len() && (reaching >= p.edge_lines || flush.len() >= p.justify_evidence);
            for (r2, why2) in split_by(run.clone(), why, |i| {
                let (a, b) = (pc(run[i]), pc(run[i + 1]));
                let em = b.size.max(a.size);
                let slack = mr - a.r;
                if ends_hyphen(&a.text) || !p.short_lines {
                    None
                } else if justified && slack > p.short_slack as f64 * em {
                    Some("short-line")
                } else if !justified && ends_sentence(&a.text) && slack > first_word_width(items, b) + p.short_margin as f64 * em {
                    Some("short-line")
                } else if !justified
                    && !a.opaque
                    && (1..=p.tiny_line).contains(&nonspace(&a.text))
                    && a.text.chars().any(|c| c.is_ascii_digit())
                    && nonspace(&b.text) > p.tiny_line
                    && slack > 2.0 * first_word_width(items, b) + p.short_margin as f64 * em
                {
                    // a number of a few characters over a long line (a tick value, an amount): the next word would
                    // have fitted on it twice over. (A short WORD may end a line the author broke on purpose, inside a
                    // table cell: "LED" over "DRIVER (8 LIGHTS ...)" is one description.)
                    Some("short-line")
                } else {
                    None
                }
            }) {
                blocks.push(draft(items, pieces, r2.iter().map(|&i| chain[i]).collect(), why2));
            }
        }
    }
    blocks
}

/// The per-row facts `segment` needs before it splits runs: whether the stack
/// is a ruled cell, which lines are list items, the text-left edges to
/// measure from, the right edges, and where margins start. Moved out of
/// `segment`; needs no statistics of its own.
pub(super) struct RowGeometry {
    cell: bool,
    is_item: Vec<bool>,
    tl: Vec<f64>,
    rights: Vec<f64>,
    margins: Vec<usize>,
}

pub(super) fn row_geometry(items: &[It], pieces: &[Piece], chain: &[usize], furn: &Furniture, p: &Params) -> RowGeometry {
    let pc = |i: usize| &pieces[chain[i]];
    let n = chain.len();
        // a ruled cell: a short stack of lines with a rule right under it (a label line and its value
        // line between two table rules) is one unit whatever its weights and colours
        let cell = p.ruled_bands && n >= 2 && n <= p.cell_lines && rule_under(furn, pc(n - 1), p);

        // list markers: "1." or "a." only count when the column has another of their kind (a wrapped
        // line that happens to start with "a." is no list); bullets, dashes and bracketed markers
        // count on their own. The continuation lines of an item hang from its text, so `tl` is the
        // left edge of the text proper.
        let mut kinds = [0usize; 3];
        for i in 0..n {
            if let Some(k) = dot_kind(&pc(i).text) {
                kinds[k] += 1;
            }
        }
        let is_item: Vec<bool> = (0..n).map(|i| !pc(i).opaque && starts_item(&pc(i).text) && dot_kind(&pc(i).text).map_or(true, |k| kinds[k] >= 2)).collect();
        let tl: Vec<f64> = (0..n).map(|i| if is_item[i] { text_left(pc(i), items) } else { pc(i).l }).collect();
        let rights: Vec<f64> = (0..n).map(|i| pc(i).r).collect();

        // margins: where a block quote starts or ends. Chain-wide, so that the statistics of the right
        // margin below see the whole column and not a handful of lines
        let margins: Vec<usize> = if p.margin_rule && n >= 4 {
            let mut sizes: Vec<f64> = (0..n).map(|i| pc(i).size).collect();
            sizes.sort_by(|a, b| a.total_cmp(b));
            margin_starts(&tl, &rights, sizes[(n - 1) / 2], p)
        } else {
            Vec::new()
        };
    RowGeometry { cell, is_item, tl, rights, margins }
}


pub(super) fn draft(items: &[It], pieces: &[Piece], run: Vec<usize>, why: &'static str) -> BlockDraft {
    let lines = run
        .iter()
        .map(|&pi| {
            let pc = &pieces[pi];
            let mut ld = LineDraft { members: Vec::new(), outlined: Vec::new(), rect: [f64::MAX, f64::MAX, f64::MIN, f64::MIN], baseline: pc.base };
            for &k in &pc.items {
                let f = &items[k];
                if f.opaque {
                    ld.outlined.push((f.l, f.obj));
                } else {
                    ld.members.push((f.l, f.obj));
                }
                ld.rect = [ld.rect[0].min(f.l), ld.rect[1].min(f.t), ld.rect[2].max(f.r), ld.rect[3].max(f.b)];
            }
            ld
        })
        .collect();
    BlockDraft { lines, why }
}
