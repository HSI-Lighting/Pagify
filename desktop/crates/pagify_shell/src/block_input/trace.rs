//! The pick trace: what path a click took, the timing facts recorded for
//! the status line, and their formatting.
//!
//! Part of the `block_input` module — split out of the single file the
//! design review flagged (Phase 4, file splits).
use super::*;

/// Which way a click went.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PickPath {
    /// The seed's whole block opened in the paragraph editor.
    Block,
    /// One run opened alone (a block of one object, an unreadable or invisible seed, a failed guard).
    Single,
    /// A paragraph the person joined by hand.
    Joined,
    /// A word drawn as outlines.
    Drawn,
    /// Rotated text, refused.
    RefusedRotated,
    /// Nothing was there.
    #[default]
    None,
}

impl PickPath {
    pub fn as_str(self) -> &'static str {
        match self {
            PickPath::Block => "block",
            PickPath::Single => "single",
            PickPath::Joined => "joined",
            PickPath::Drawn => "drawn",
            PickPath::RefusedRotated => "refused-rotated",
            PickPath::None => "none",
        }
    }
}

/// What one click decided, content-free (no word of the page): ids, counts, geometry, timings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PickTrace {
    /// 0-based, as everywhere in the app; [`format_pick_line`] prints it 1-based like the page label.
    pub page: usize,
    pub click: (f32, f32),
    pub seed: Option<usize>,
    pub rule: Option<SeedRule>,
    pub path: PickPath,
    /// Admitted objects on the page, and blocks the detector made of them.
    pub frags: usize,
    pub blocks: usize,
    pub block: Option<usize>,
    pub lines: usize,
    /// Text objects of the block, and outlined words bridged into its lines.
    pub objects: usize,
    pub outlined: usize,
    /// Faux-bold twins the block's members carry (see [`LineSpec::twins`]): removed with their lines.
    pub twins: usize,
    /// Indices of the lines that carry outlined words.
    pub frozen: Vec<usize>,
    /// Why this block starts, and why the next block (in detector order) starts: the second says why
    /// this paragraph stopped.
    pub starts: Option<&'static str>,
    pub next_starts: Option<&'static str>,
    /// The block's rect: left, top, right, bottom.
    pub rect: Option<[f32; 4]>,
    /// The distinct font ids of the block's objects (`u32::MAX`, an object with no style, prints as `?`).
    pub fonts: Vec<u32>,
    pub cache_hit: bool,
    pub build_ms: f32,
    pub detect_ms: f32,
    pub total_ms: f32,
}

impl PickTrace {
    pub fn new(page: usize, click: (f32, f32)) -> PickTrace {
        PickTrace { page, click, ..PickTrace::default() }
    }

    /// The page-wide facts, from the cache entry the click was resolved against.
    pub fn page_facts(&mut self, pb: &PageBlocks) {
        self.page = pb.page;
        self.frags = pb.frags.len();
        self.blocks = pb.blocks.len();
        self.build_ms = pb.build_ms;
        self.detect_ms = pb.detect_ms;
    }

    /// The facts of the block the click opened (or tried to).
    pub fn block_facts(&mut self, pb: &PageBlocks, block: usize) {
        let Some(b) = pb.blocks.get(block) else {
            self.block = None;
            return;
        };
        self.block = Some(block);
        self.lines = b.lines.len();
        self.objects = b.lines.iter().map(|l| l.objects.len()).sum();
        self.outlined = b.lines.iter().map(|l| l.outlined.len()).sum();
        self.twins = b.lines.iter().flat_map(|l| l.objects.iter()).map(|o| pb.twins.get(o).map_or(0, Vec::len)).sum();
        self.frozen = b.lines.iter().enumerate().filter(|(_, l)| !l.outlined.is_empty()).map(|(i, _)| i).collect();
        self.starts = Some(b.starts_because);
        self.next_starts = pb.blocks.get(block + 1).map(|n| n.starts_because);
        self.rect = Some([b.left, b.top, b.right, b.bottom]);
        let mut fonts: Vec<u32> = b.lines.iter().flat_map(|l| l.objects.iter()).map(|o| pb.styles.get(o).map_or(u32::MAX, |s| s.font)).collect();
        fonts.sort_unstable();
        fonts.dedup();
        self.fonts = fonts;
    }
}

/// The one line the session log gets per click, kind `"pick"`. Key=value pairs, no page content:
///
/// `page 1 click=(212.4,501.2) seed=1029 rule=exact path=block frags=770 blocks=129 block=106 lines=13
/// objects=57 outlined=2 twins=0 frozen=[8,10] starts="style" next_starts="style" rect=[187.0,421.0,306.0,538.0]
/// fonts=[1] cache=miss build_ms=38.1 detect_ms=1.2 total_ms=41.0`
pub fn format_pick_line(t: &PickTrace) -> String {
    pub(super) fn list<I: IntoIterator<Item = String>>(items: I) -> String {
        format!("[{}]", items.into_iter().collect::<Vec<_>>().join(","))
    }
    let some = |v: Option<usize>| v.map_or("none".to_string(), |v| v.to_string());
    let reason = |v: Option<&'static str>| v.map_or("none".to_string(), |v| format!("{v:?}"));
    let rect = t.rect.map_or("none".to_string(), |r| format!("[{:.1},{:.1},{:.1},{:.1}]", r[0], r[1], r[2], r[3]));
    format!(
        "page {} click=({:.1},{:.1}) seed={} rule={} path={} frags={} blocks={} block={} lines={} objects={} outlined={} twins={} frozen={} starts={} next_starts={} rect={} fonts={} cache={} build_ms={:.1} detect_ms={:.1} total_ms={:.1}",
        t.page + 1,
        t.click.0,
        t.click.1,
        some(t.seed),
        t.rule.map_or("none", SeedRule::as_str),
        t.path.as_str(),
        t.frags,
        t.blocks,
        some(t.block),
        t.lines,
        t.objects,
        t.outlined,
        t.twins,
        list(t.frozen.iter().map(|i| i.to_string())),
        reason(t.starts),
        reason(t.next_starts),
        rect,
        list(t.fonts.iter().map(|f| if *f == u32::MAX { "?".to_string() } else { f.to_string() })),
        if t.cache_hit { "hit" } else { "miss" },
        t.build_ms,
        t.detect_ms,
        t.total_ms,
    )
}
