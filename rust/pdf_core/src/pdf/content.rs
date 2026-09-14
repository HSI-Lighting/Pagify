//! A page's content stream: reading it, and putting text back without it.
//!
//! # What this is for
//!
//! Taking locked text off a page. An image is its own object, so
//! [`super::File::rewrite`] can replace it and touch nothing else; text is
//! drawn **inline**, in the sequence of operators that paints the page. Removing
//! some of it means editing that sequence.
//!
//! The rule is the same one the rest of `crate::pdf` follows: **everything not
//! being removed is copied through byte for byte.** The operators that draw the
//! surviving text are never re-emitted, never re-spaced and never re-encoded —
//! they are the same bytes they always were. Only the operators that showed the
//! locked words are cut out.
//!
//! That is the whole difference from letting PDFium do it.
//! `FPDFPage_GenerateContent` rebuilds the entire stream from its object model,
//! and measured on a catalogue page it reordered a paragraph nobody had
//! touched.

use crate::error::{PdfError, Result};

use super::object::{Lexer, Object};

/// One operator and the operands in front of it, and where it sat.
#[derive(Debug, Clone, PartialEq)]
pub struct Operation {
    pub operator: Vec<u8>,
    pub operands: Vec<Object>,
    /// Where this whole operation lies in the stream, operands included, so it
    /// can be cut out without disturbing a byte either side.
    pub span: std::ops::Range<usize>,
}

impl Operation {
    /// Whether this operation paints text.
    ///
    /// The four that take a string: `Tj` and `'` and `"` show one, `TJ` shows
    /// an array of them with spacing numbers between.
    pub fn shows_text(&self) -> bool {
        matches!(self.operator.as_slice(), b"Tj" | b"TJ" | b"'" | b"\"")
    }
}

/// Split a content stream into its operations.
///
/// Operands accumulate until an operator ends them, which is how PostScript's
/// postfix notation works and why this cannot be a simple token walk: the
/// operator comes *after* the values it takes.
///
/// Inline images are skipped whole. `BI … ID <binary> EI` puts raw image bytes
/// straight in the stream, and those bytes are as likely to contain `EI` as
/// anything else — so the scan looks for a delimiter followed by whitespace,
/// and a stream this cannot resolve is refused rather than mis-cut.
pub fn parse(bytes: &[u8]) -> Result<Vec<Operation>> {
    let mut lexer = Lexer::new(bytes, 0);
    let mut out = Vec::new();
    let mut operands: Vec<Object> = Vec::new();
    let mut start = 0usize;

    loop {
        lexer.skip_space();
        if lexer.at >= bytes.len() {
            return Ok(out);
        }
        if operands.is_empty() {
            start = lexer.at;
        }

        match bytes[lexer.at] {
            // Everything that begins a value rather than a keyword.
            b'/' | b'(' | b'[' | b'<' | b'0'..=b'9' | b'+' | b'-' | b'.' => {
                operands.push(lexer.object()?);
            }
            b']' | b')' | b'>' | b'}' | b'{' => {
                // A stray delimiter: the stream is not what it claims to be,
                // and guessing past it would cut the wrong bytes.
                return Err(PdfError::InvalidArgument(format!(
                    "a stray {:?} in the content stream at {}",
                    bytes[lexer.at] as char, lexer.at
                )));
            }
            _ => {
                let operator = lexer.token().to_vec();
                if operator.is_empty() {
                    return Err(PdfError::InvalidArgument(
                        "the content stream stopped making sense".into(),
                    ));
                }
                if operator == b"BI" {
                    lexer.at = skip_inline_image(bytes, lexer.at)?;
                    operands.clear();
                    continue;
                }
                // `true`, `false` and `null` are values, not operators.
                match operator.as_slice() {
                    b"true" => {
                        operands.push(Object::Bool(true));
                        continue;
                    }
                    b"false" => {
                        operands.push(Object::Bool(false));
                        continue;
                    }
                    b"null" => {
                        operands.push(Object::Null);
                        continue;
                    }
                    _ => {}
                }
                out.push(Operation {
                    operator,
                    operands: std::mem::take(&mut operands),
                    span: start..lexer.at,
                });
            }
        }
    }
}

/// Past an inline image, to just after its `EI`.
fn skip_inline_image(bytes: &[u8], from: usize) -> Result<usize> {
    // The data begins after `ID` and one whitespace byte.
    let id = bytes[from..]
        .windows(2)
        .position(|w| w == b"ID")
        .map(|n| from + n + 2)
        .ok_or_else(|| PdfError::InvalidArgument("an inline image with no data".into()))?;
    let mut at = id + 1;

    while at + 1 < bytes.len() {
        if &bytes[at..at + 2] == b"EI"
            && bytes.get(at.wrapping_sub(1)).is_some_and(|b| b.is_ascii_whitespace())
            && bytes.get(at + 2).is_none_or(|b| b.is_ascii_whitespace())
        {
            return Ok(at + 2);
        }
        at += 1;
    }
    Err(PdfError::InvalidArgument("an inline image that never ends".into()))
}

/// Where a show-text operator draws, in the page's own space.
///
/// # Why the origin is enough
///
/// Knowing how *far* a string reaches would need the font's glyph widths, and
/// those live in the font dictionary behind an encoding. The **origin** does
/// not: it falls out of the text matrix and the graphics state alone, which are
/// both right there in the stream. That is enough to say which run PDFium is
/// talking about, and PDFium already knows how far each run reaches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Origin {
    /// Index into the operations this came from.
    pub operation: usize,
    /// Where the text starts, in unscaled PDF user space — y up from the
    /// bottom-left, as the file has it.
    pub x: f32,
    pub y: f32,
}

/// A show-text operator, where it draws, and in which font.
///
/// The font is the resource *name* — `/F3` — as the stream selects it. Turning
/// that into a font takes the page's resource dictionary, which lives in the
/// file rather than the stream, so it is left to the caller.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub origin: Origin,
    /// The name last given to `Tf`, if any.
    pub font: Option<Vec<u8>>,
    pub size: f32,
    /// Which line of text this belongs to.
    ///
    /// **Not a guess from where it sits.** Two show-text operators are on the
    /// same line when nothing between them moved the text down — that is a fact
    /// about the stream's own state machine, and it is what says a line's
    /// hyphen, drawn by a `Tj` of its own, is part of the line before it.
    /// Grouping by proximity instead swept up neighbouring runs and made
    /// matching worse.
    pub line: usize,
    /// How much the text matrix and the graphics state between them magnify
    /// this text on the page.
    ///
    /// **Not cosmetic.** A `TJ` displacement is in thousandths of *unscaled*
    /// text space and reaches the page multiplied by the font size **and** by
    /// this. Plenty of producers write `1 Tf` and put the real size in the
    /// matrix, so leaving it out overstated every gap by an order of magnitude
    /// — which looked exactly like the text having been shoved sideways.
    pub scale: f32,
}

/// Multiply two PDF matrices, `a b c d e f`.
fn multiply(m: [f32; 6], n: [f32; 6]) -> [f32; 6] {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

fn numbers(operands: &[Object], count: usize) -> Option<Vec<f32>> {
    let start = operands.len().checked_sub(count)?;
    operands[start..].iter().map(|o| o.as_f64().map(|n| n as f32)).collect()
}

/// Walk the stream and report where each show-text operator begins.
///
/// Tracks exactly what decides that and no more: the graphics state stack
/// (`q`/`Q`/`cm`) and the text state (`BT`, `Tm`, `Td`, `TD`, `T*`, `TL`, and
/// the two operators that move to the next line before showing).
pub fn origins(operations: &[Operation]) -> Vec<Origin> {
    placed(operations).into_iter().map(|p| p.origin).collect()
}

/// The graphics and text state in force **while an operation draws**.
///
/// One entry per operation, in the same order. A positioning operator's own
/// effect is already in its entry — `states[i]` for a `Tm` is the matrix that
/// `Tm` just set — because the question every caller asks is "what was in force
/// when this drew", and for the operators that draw, the two are the same
/// thing.
///
/// **Why this is exposed rather than kept inside [`placed`].** Moving a run
/// without re-emitting the page means writing a `Tm` that puts the text back
/// where it was, shifted — and that needs the absolute text matrix, which only
/// this walk knows. See `PdfiumDocument::move_run_in_stream`.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    /// The current transformation matrix, from `q`/`Q`/`cm`.
    pub ctm: [f32; 6],
    /// The text matrix: where the next glyph goes.
    pub text: [f32; 6],
    /// The line matrix, which `Td`, `TD` and `T*` move from.
    ///
    /// **Not the same as [`State::text`] once anything has been drawn**, and
    /// the difference is what a restoring `Tm` has to respect: putting the text
    /// matrix back but leaving the line matrix shifted moves every line after
    /// the one that was touched.
    pub line: [f32; 6],
    /// The name last given to `Tf`, if any.
    pub font: Option<Vec<u8>>,
    /// The size last given to `Tf`.
    pub size: f32,
    /// Horizontal scaling, as a factor — `Tz` divided by a hundred.
    ///
    /// Every horizontal displacement in text space is multiplied by it,
    /// including the one a lone number in a `TJ` array makes. Left out, a run
    /// set at 50% would move twice as far as it was asked to.
    pub horizontal_scale: f32,
    /// The name last given to `gs`, if any — the ExtGState in force, which is
    /// where an object's opacity lives. Saved and restored with `q`/`Q` like
    /// the rest of the graphics state.
    pub ext_gstate: Option<Vec<u8>>,
    /// Text rise, from `Ts`: how far glyphs are drawn above the baseline.
    ///
    /// **Not a pen movement.** Rise offsets where a glyph is painted and leaves
    /// the pen exactly where it was, which is what makes it the one way to move
    /// a run vertically without disturbing the words after it on the same line.
    pub rise: f32,
    /// Which line of text this belongs to — see [`Placed::line`].
    pub line_number: usize,
}

/// Walk the stream, keeping every matrix it sets.
pub fn states(operations: &[Operation]) -> Vec<State> {
    let mut out = Vec::with_capacity(operations.len());
    let mut ctm = IDENTITY;
    let mut stack: Vec<([f32; 6], Option<Vec<u8>>)> = Vec::new();
    // The text matrix, and the line matrix each new line starts from.
    let (mut text, mut line) = (IDENTITY, IDENTITY);
    let mut leading = 0.0f32;
    let (mut font, mut size) = (None::<Vec<u8>>, 0.0f32);
    let (mut horizontal_scale, mut rise) = (1.0f32, 0.0f32);
    let mut ext_gstate: Option<Vec<u8>> = None;
    // Bumped by everything that starts a new line of text, so operators that
    // continue one another share a number.
    let mut line_number = 0usize;

    for operation in operations {
        match operation.operator.as_slice() {
            b"q" => stack.push((ctm, ext_gstate.clone())),
            b"Q" => {
                let (c, g) = stack.pop().unwrap_or((IDENTITY, None));
                ctm = c;
                ext_gstate = g;
            }
            b"gs" => {
                if let Some(Object::Name(name)) = operation.operands.first() {
                    ext_gstate = Some(name.clone());
                }
            }
            b"cm" => {
                if let Some(n) = numbers(&operation.operands, 6) {
                    ctm = multiply([n[0], n[1], n[2], n[3], n[4], n[5]], ctm);
                }
            }
            b"BT" => {
                text = IDENTITY;
                line = IDENTITY;
                line_number += 1;
            }
            b"Tm" => {
                if let Some(n) = numbers(&operation.operands, 6) {
                    let moved = (n[5] - line[5]).abs() > 0.01;
                    line = [n[0], n[1], n[2], n[3], n[4], n[5]];
                    text = line;
                    // A `Tm` that only moves along the line continues it; one
                    // that changes the vertical starts a new one.
                    if moved {
                        line_number += 1;
                    }
                }
            }
            b"Tf" => {
                if let [.., Object::Name(name), rest] = operation.operands.as_slice() {
                    font = Some(name.clone());
                    size = rest.as_f64().unwrap_or(0.0) as f32;
                }
            }
            b"TL" => {
                if let Some(n) = numbers(&operation.operands, 1) {
                    leading = n[0];
                }
            }
            b"Tz" => {
                if let Some(n) = numbers(&operation.operands, 1) {
                    horizontal_scale = n[0] / 100.0;
                }
            }
            b"Ts" => {
                if let Some(n) = numbers(&operation.operands, 1) {
                    rise = n[0];
                }
            }
            b"Td" => {
                if let Some(n) = numbers(&operation.operands, 2) {
                    line = multiply([1.0, 0.0, 0.0, 1.0, n[0], n[1]], line);
                    text = line;
                    if n[1].abs() > 0.01 {
                        line_number += 1;
                    }
                }
            }
            b"TD" => {
                if let Some(n) = numbers(&operation.operands, 2) {
                    // `TD` sets the leading to the negated vertical move, then
                    // does what `Td` does.
                    leading = -n[1];
                    line = multiply([1.0, 0.0, 0.0, 1.0, n[0], n[1]], line);
                    text = line;
                    if n[1].abs() > 0.01 {
                        line_number += 1;
                    }
                }
            }
            b"T*" => {
                line = multiply([1.0, 0.0, 0.0, 1.0, 0.0, -leading], line);
                text = line;
                line_number += 1;
            }
            _ => {}
        }

        // `'` and `"` move to the next line before drawing.
        if operation.shows_text() && matches!(operation.operator.as_slice(), b"'" | b"\"") {
            line = multiply([1.0, 0.0, 0.0, 1.0, 0.0, -leading], line);
            text = line;
            line_number += 1;
        }

        out.push(State {
            ctm,
            text,
            line,
            font: font.clone(),
            size,
            horizontal_scale,
            ext_gstate: ext_gstate.clone(),
            rise,
            line_number,
        });
    }
    out
}

/// The same walk, keeping the font each operator draws in.
pub fn placed(operations: &[Operation]) -> Vec<Placed> {
    states(operations)
        .into_iter()
        .enumerate()
        .filter(|(index, _)| operations[*index].shows_text())
        .map(|(index, state)| {
            let at = multiply(state.text, state.ctm);
            // The length of the transformed x-axis: what one unit of text space
            // measures on the page.
            let scale = (at[0] * at[0] + at[1] * at[1]).sqrt();
            Placed {
                origin: Origin { operation: index, x: at[4], y: at[5] },
                font: state.font,
                size: state.size,
                scale,
                line: state.line_number,
            }
        })
        .collect()
}

/// One piece of what a show-text operator draws.
#[derive(Debug, Clone, PartialEq)]
pub enum Piece {
    /// Character codes, as bytes — `width` of them to a code.
    Codes(Vec<u8>),
    /// A `TJ` spacing number, in thousandths of the font size.
    Kern(f32),
}

/// What an operator shows, as codes and the spacing between them.
///
/// Literal strings come back with their escapes resolved, so a caller counting
/// codes counts glyphs rather than backslashes.
pub fn pieces(operation: &Operation) -> Vec<Piece> {
    fn string_bytes(object: &Object) -> Option<Vec<u8>> {
        match object {
            Object::LiteralString(raw) => Some(unescape(raw)),
            Object::HexString(raw) => {
                let digits: Vec<u8> = raw.iter().copied().filter(u8::is_ascii_hexdigit).collect();
                Some(
                    digits
                        .chunks(2)
                        .map(|pair| {
                            // An odd trailing digit is padded with zero, as the
                            // specification says.
                            let high = (pair[0] as char).to_digit(16).unwrap_or(0) as u8;
                            let low = pair
                                .get(1)
                                .and_then(|b| (*b as char).to_digit(16))
                                .unwrap_or(0) as u8;
                            high << 4 | low
                        })
                        .collect(),
                )
            }
            _ => None,
        }
    }

    let mut out = Vec::new();
    for operand in &operation.operands {
        match operand {
            Object::Array(items) => {
                for item in items {
                    if let Some(bytes) = string_bytes(item) {
                        out.push(Piece::Codes(bytes));
                    } else if let Some(number) = item.as_f64() {
                        out.push(Piece::Kern(number as f32));
                    }
                }
            }
            other => {
                if let Some(bytes) = string_bytes(other) {
                    out.push(Piece::Codes(bytes));
                }
            }
        }
    }
    out
}

/// A literal string's real bytes, escapes resolved.
fn unescape(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut at = 0usize;
    while at < raw.len() {
        if raw[at] != b'\\' {
            out.push(raw[at]);
            at += 1;
            continue;
        }
        at += 1;
        let Some(next) = raw.get(at) else { break };
        match next {
            b'n' => {
                out.push(b'\n');
                at += 1;
            }
            b'r' => {
                out.push(b'\r');
                at += 1;
            }
            b't' => {
                out.push(b'\t');
                at += 1;
            }
            b'b' => {
                out.push(0x08);
                at += 1;
            }
            b'f' => {
                out.push(0x0C);
                at += 1;
            }
            // A backslash before a newline continues the line and adds nothing.
            b'\n' => at += 1,
            b'\r' => {
                at += 1;
                if raw.get(at) == Some(&b'\n') {
                    at += 1;
                }
            }
            b'0'..=b'7' => {
                let mut value = 0u16;
                let mut digits = 0;
                while digits < 3 && raw.get(at).is_some_and(|b| (b'0'..=b'7').contains(b)) {
                    value = value * 8 + u16::from(raw[at] - b'0');
                    at += 1;
                    digits += 1;
                }
                out.push(value as u8);
            }
            other => {
                out.push(*other);
                at += 1;
            }
        }
    }
    out
}

/// A show-text operator that draws nothing and moves the pen by `advance`
/// points — what stands in for an operator taken out whole when something
/// continues its line.
///
/// A `Tj` removed outright takes its advance with it, and a fragment that
/// shares the text object — `HSI` then ` Lighting`, one operator each — slides
/// left into the gap. This keeps the gap: a `TJ` with only a spacing number,
/// negated, in thousandths of the font size, as the numbers in `without_codes`
/// are. Nothing where the size cannot carry it.
pub fn advance_only(font: &[u8], size: f32, scale: f32, advance: f32) -> Vec<u8> {
    let per_point = size * scale;
    if per_point.abs() <= f32::EPSILON || advance == 0.0 {
        return Vec::new();
    }
    let mut out = b"/".to_vec();
    out.extend_from_slice(font);
    out.extend_from_slice(format!(" {size} Tf [ {} ] TJ", -advance / per_point * 1000.0).as_bytes());
    out
}

/// Rebuild a show-text operator with some of its codes removed.
///
/// # Holding the rest of the line still
///
/// Taking codes out of the middle of a string would slide everything after them
/// to the left. `advance` says how wide the removed run was on the page, in
/// points; a `TJ` number of the same size — negated, in thousandths of the font
/// size — puts the gap back, so what survives stays exactly where it was.
///
/// Codes are re-emitted as a **hex string** whatever they arrived as. It is the
/// one form that carries any byte without an escape to get wrong, and the
/// bytes themselves are unchanged — this re-encodes how they are written, never
/// what they say.
pub fn without_codes(
    operation: &Operation,
    dropped: &[(usize, f32)],
    font: &[u8],
    size: f32,
    scale: f32,
    width: usize,
) -> Vec<u8> {
    let mut kept: Vec<Piece> = Vec::new();
    let mut code = 0usize;
    // How much has been taken out since the last surviving glyph, in points.
    // Flushed as one spacing number **where the gap is**, which is the whole
    // point: put it anywhere else and the surviving text either side moves.
    let mut gap = 0.0f32;

    // A `TJ` number reaches the page as `number/1000 x size x scale`, and is
    // *subtracted* from the horizontal position — so closing a gap of `w`
    // points takes a negative number of `w x 1000 / (size x scale)`.
    let per_point = size * scale;
    let mut flush = |kept: &mut Vec<Piece>, gap: &mut f32| {
        if *gap != 0.0 && per_point.abs() > f32::EPSILON {
            kept.push(Piece::Kern(-*gap / per_point * 1000.0));
        }
        *gap = 0.0;
    };

    // Whether the last glyph seen was one being removed. A spacing number that
    // follows a removed glyph sat *inside* the removed run, and its
    // displacement is already counted — the advance handed in is measured from
    // one glyph's origin to the next, across whatever kerning lay between. Kept
    // as well, it is applied twice and the survivors drift.
    let mut after_dropped = false;

    for piece in pieces(operation) {
        match piece {
            Piece::Kern(number) => {
                if !after_dropped {
                    kept.push(Piece::Kern(number));
                }
            }
            Piece::Codes(bytes) => {
                let mut run: Vec<u8> = Vec::new();
                for chunk in bytes.chunks(width.max(1)) {
                    match dropped.iter().find(|(index, _)| *index == code) {
                        Some((_, advance)) => {
                            after_dropped = true;
                            // Everything before the gap is emitted first, so the
                            // spacing number lands between the two halves.
                            if !run.is_empty() {
                                kept.push(Piece::Codes(std::mem::take(&mut run)));
                            }
                            gap += advance;
                        }
                        None => {
                            after_dropped = false;
                            flush(&mut kept, &mut gap);
                            run.extend_from_slice(chunk);
                        }
                    }
                    code += 1;
                }
                if !run.is_empty() {
                    kept.push(Piece::Codes(run));
                }
            }
        }
    }
    // A gap at the very end needs no number: nothing follows it to be moved.

    if !kept.iter().any(|p| matches!(p, Piece::Codes(bytes) if !bytes.is_empty())) {
        // Nothing survived, so nothing is drawn — and no font is selected for a
        // string that is not there.
        return Vec::new();
    }

    let mut out = Vec::new();
    out.push(b'/');
    out.extend_from_slice(font);
    out.extend_from_slice(format!(" {size} Tf [").as_bytes());
    for piece in &kept {
        match piece {
            Piece::Codes(bytes) => {
                out.push(b'<');
                for byte in bytes {
                    out.extend_from_slice(format!("{byte:02X}").as_bytes());
                }
                out.push(b'>');
            }
            Piece::Kern(number) => out.extend_from_slice(format!(" {number} ").as_bytes()),
        }
    }
    out.extend_from_slice(b"] TJ");
    out
}

/// Rebuild a show-text operator with a stretch of its codes replaced.
///
/// The sibling of [`without_codes`], for changing words rather than removing
/// them. Everything outside `range` keeps its own bytes and its own kerning;
/// only the codes named are swapped for `replacement`.
///
/// No gap is closed here. Removing text leaves a hole that has to be held open
/// or the rest of the line slides; **replacing** it legitimately changes the
/// width, which is what editing a word means.
pub fn replacing_codes(
    operation: &Operation,
    range: std::ops::Range<usize>,
    replacement: &[u8],
    font: &[u8],
    size: f32,
    width: usize,
) -> Vec<u8> {
    let mut kept: Vec<Piece> = Vec::new();
    let mut code = 0usize;
    let mut put = false;

    for piece in pieces(operation) {
        match piece {
            // Kerning inside the replaced stretch goes with it; the new text
            // brings its own spacing.
            Piece::Kern(number) => {
                if !(code > range.start && code < range.end) {
                    kept.push(Piece::Kern(number));
                }
            }
            Piece::Codes(bytes) => {
                let mut run: Vec<u8> = Vec::new();
                for chunk in bytes.chunks(width.max(1)) {
                    if range.contains(&code) {
                        if !run.is_empty() {
                            kept.push(Piece::Codes(std::mem::take(&mut run)));
                        }
                        if !put {
                            kept.push(Piece::Codes(replacement.to_vec()));
                            put = true;
                        }
                    } else {
                        run.extend_from_slice(chunk);
                    }
                    code += 1;
                }
                if !run.is_empty() {
                    kept.push(Piece::Codes(run));
                }
            }
        }
    }
    if !put {
        kept.push(Piece::Codes(replacement.to_vec()));
    }

    let mut out = Vec::new();
    out.push(b'/');
    out.extend_from_slice(font);
    out.extend_from_slice(format!(" {size} Tf [").as_bytes());
    for piece in &kept {
        match piece {
            Piece::Codes(bytes) => {
                out.push(b'<');
                for byte in bytes {
                    out.extend_from_slice(format!("{byte:02X}").as_bytes());
                }
                out.push(b'>');
            }
            Piece::Kern(number) => out.extend_from_slice(format!(" {number} ").as_bytes()),
        }
    }
    out.extend_from_slice(b"] TJ");
    out
}

/// Cut some operations out, copying every other byte through unchanged.
///
/// **The point of the whole module.** What is left is not re-emitted: the bytes
/// between the cuts are the bytes that were there, so the surviving text draws
/// exactly as it drew before.
pub fn remove(bytes: &[u8], operations: &[&Operation]) -> Vec<u8> {
    let edits: Vec<(std::ops::Range<usize>, Vec<u8>)> =
        operations.iter().map(|o| (o.span.clone(), Vec::new())).collect();
    splice(bytes, &edits)
}

/// Swap some spans for new bytes, copying every other byte through unchanged.
///
/// The general form of [`remove`] — an empty replacement is a cut. What is not
/// named here is not touched, which is the guarantee the whole module rests on.
pub fn splice(bytes: &[u8], edits: &[(std::ops::Range<usize>, Vec<u8>)]) -> Vec<u8> {
    let mut edits: Vec<&(std::ops::Range<usize>, Vec<u8>)> = edits.iter().collect();
    edits.sort_by_key(|(span, _)| span.start);

    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0usize;
    for (span, replacement) in edits {
        if span.start < at {
            // Overlapping spans would rewrite the same bytes twice and corrupt
            // what follows. Skipped rather than trusted.
            continue;
        }
        out.extend_from_slice(&bytes[at..span.start]);
        // Newlines around it, so what is put in — or the join left by taking
        // something out — cannot fuse with a neighbouring token.
        out.push(b'\n');
        out.extend_from_slice(replacement);
        out.push(b'\n');
        at = span.end;
    }
    out.extend_from_slice(&bytes[at..]);
    out
}

/// Inflate a stream, if it says it is deflated.
///
/// Only `FlateDecode`, and only on its own — with or without a predictor.
/// Anything else — a filter chain, an encoding this does not know — comes
/// back as `None` rather than as a guess, and the caller leaves that page
/// alone.
pub fn decode(dict: &super::Dict, raw: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;

    match dict.get(b"Filter") {
        None => return Some(raw.to_vec()),
        Some(Object::Name(name)) if name == b"FlateDecode" => {}
        Some(Object::Array(items)) if items.len() == 1 => match &items[0] {
            Object::Name(name) if name == b"FlateDecode" => {}
            _ => return None,
        },
        _ => return None,
    }
    // A `/DecodeParms` names a predictor, which changes the bytes after
    // inflating: undone below, or refused where its parameters are ones
    // this does not read. One dictionary, or an array of one for the
    // one-filter array form.
    let parms = match dict.get(b"DecodeParms") {
        None | Some(Object::Null) => None,
        Some(Object::Dict(d)) => Some(d.clone()),
        Some(Object::Array(items)) if items.len() == 1 => match &items[0] {
            Object::Dict(d) => Some(d.clone()),
            Object::Null => None,
            _ => return None,
        },
        _ => return None,
    };
    let predictor = parms.as_ref().map(Predictor::from).unwrap_or_default();
    if !predictor.readable() {
        return None;
    }

    // Bounded. Deflate manages about a thousand to one on the right input,
    // so a megabyte of stream became a gigabyte of content on the first
    // edit; fonts and `/ToUnicode` come through here too. Found by audit.
    // Over the limit reads as "cannot decode", and the caller leaves the
    // page alone — the same as for a filter it does not know.
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(raw)
        .take(INFLATED_LIMIT + 1)
        .read_to_end(&mut out)
        .ok()?;
    if out.len() as u64 > INFLATED_LIMIT {
        return None;
    }
    predictor.undo(out)
}

/// A predictor from a stream's `/DecodeParms`: the transform a writer applied
/// to the bytes before deflating them, to make them deflate better, which a
/// reader undoes after inflating.
///
/// Two families. The PNG predictors (10 to 15) put a filter-type byte in front
/// of every row and filter each row against itself and the row above, exactly
/// as PNG does — 15 means each row says which. The TIFF predictor (2) is
/// horizontal differencing: each sample is stored as the difference from the
/// one before it in the row. 1 is none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Predictor {
    kind: u32,
    colors: u32,
    bits_per_component: u32,
    columns: u32,
}

impl Default for Predictor {
    fn default() -> Self {
        Predictor { kind: 1, colors: 1, bits_per_component: 8, columns: 1 }
    }
}

impl From<&super::Dict> for Predictor {
    fn from(parms: &super::Dict) -> Self {
        let number = |key: &[u8], fallback: u32| -> u32 {
            parms
                .get(key)
                .and_then(Object::as_f64)
                .filter(|n| *n >= 0.0)
                .map(|n| n as u32)
                .unwrap_or(fallback)
        };
        Predictor {
            kind: number(b"Predictor", 1),
            colors: number(b"Colors", 1),
            bits_per_component: number(b"BitsPerComponent", 8),
            columns: number(b"Columns", 1),
        }
    }
}

impl Predictor {
    /// Whether this is a predictor the reader undoes: none, TIFF at eight bits
    /// a sample, or any PNG one — with parameters that describe a row.
    fn readable(&self) -> bool {
        let sane = (1..=64).contains(&self.colors)
            && matches!(self.bits_per_component, 1 | 2 | 4 | 8 | 16)
            && (1..=1 << 20).contains(&self.columns);
        match self.kind {
            1 => true,
            2 => sane && self.bits_per_component == 8,
            10..=15 => sane,
            _ => false,
        }
    }

    /// Bytes in one row of samples.
    fn row_length(&self) -> usize {
        let bits = self.columns as usize * self.colors as usize * self.bits_per_component as usize;
        bits.div_ceil(8)
    }

    /// Bytes in one pixel, rounded up — what the PNG filters look back by.
    fn bytes_per_pixel(&self) -> usize {
        (self.colors as usize * self.bits_per_component as usize).div_ceil(8).max(1)
    }

    /// The data as it was before the predictor was applied.
    fn undo(&self, data: Vec<u8>) -> Option<Vec<u8>> {
        match self.kind {
            1 => Some(data),
            2 => Some(self.undo_tiff(data)),
            10..=15 => self.undo_png(&data),
            _ => None,
        }
    }

    fn undo_tiff(&self, mut data: Vec<u8>) -> Vec<u8> {
        let row = self.row_length().max(1);
        let colors = self.colors as usize;
        for line in data.chunks_mut(row) {
            for i in colors..line.len() {
                line[i] = line[i].wrapping_add(line[i - colors]);
            }
        }
        data
    }

    fn undo_png(&self, data: &[u8]) -> Option<Vec<u8>> {
        let row = self.row_length();
        let bpp = self.bytes_per_pixel();
        let mut out = Vec::with_capacity(data.len());
        let mut previous = vec![0u8; row];
        // Each stored row is a filter byte and then the row; a short last row
        // is what a writer that padded nothing leaves, and is taken as far as
        // it goes.
        for stored in data.chunks(row + 1) {
            let (Some(&filter), line) = (stored.first(), &stored[1.min(stored.len())..]) else {
                break;
            };
            let mut current = line.to_vec();
            for i in 0..current.len() {
                let left = if i >= bpp { current[i - bpp] } else { 0 };
                let up = previous.get(i).copied().unwrap_or(0);
                let upper_left = if i >= bpp { previous.get(i - bpp).copied().unwrap_or(0) } else { 0 };
                let add = match filter {
                    0 => 0,
                    1 => left,
                    2 => up,
                    3 => ((left as u16 + up as u16) / 2) as u8,
                    4 => paeth(left, up, upper_left),
                    // A filter type PNG does not define: the row is not one
                    // this can read, and guessing at it is not reading it.
                    _ => return None,
                };
                current[i] = current[i].wrapping_add(add);
            }
            out.extend_from_slice(&current);
            if current.len() == row {
                previous = current;
            } else {
                previous.clear();
                previous.extend_from_slice(&current);
                previous.resize(row, 0);
            }
        }
        Some(out)
    }
}

/// PNG's Paeth predictor: whichever of left, up and upper-left is nearest the
/// linear estimate `left + up - upper_left`.
fn paeth(left: u8, up: u8, upper_left: u8) -> u8 {
    let p = left as i16 + up as i16 - upper_left as i16;
    let (pa, pb, pc) = ((p - left as i16).abs(), (p - up as i16).abs(), (p - upper_left as i16).abs());
    if pa <= pb && pa <= pc {
        left
    } else if pb <= pc {
        up
    } else {
        upper_left
    }
}

/// The most a single stream is allowed to inflate to.
///
/// No real content stream, font or CMap comes near it; a hostile one is
/// stopped a long way short of the memory it was aiming for.
pub const INFLATED_LIMIT: u64 = 128 * 1024 * 1024;

/// Deflate a stream back, at the default level.
pub fn encode(data: &[u8]) -> Result<Vec<u8>> {
    use std::io::Write;

    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(data)
        .and_then(|()| encoder.finish())
        .map_err(|e| PdfError::Pdfium(format!("the content stream could not be written: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every PNG filter type, and the TIFF one, undone — against what a
    /// writer following the specification produces.
    #[test]
    fn predictors_are_undone_row_by_row() {
        use std::io::Write;
        // Four rows of six bytes, two samples of three components each.
        let plain: Vec<u8> = (0u8..24).map(|n| n.wrapping_mul(37).wrapping_add(11)).collect();
        let (columns, colors) = (2u32, 3u32);
        let row = 6usize;
        let bpp = 3usize;

        // PNG, each row filtered with its own type (0, 1, 2, 3 then 4).
        let mut stored = Vec::new();
        let mut previous = vec![0u8; row];
        for (r, line) in plain.chunks(row).enumerate() {
            let filter = (r % 5) as u8;
            stored.push(filter);
            for i in 0..row {
                let left = if i >= bpp { line[i - bpp] } else { 0 };
                let up = previous[i];
                let upper_left = if i >= bpp { previous[i - bpp] } else { 0 };
                let predicted = match filter {
                    0 => 0,
                    1 => left,
                    2 => up,
                    3 => ((left as u16 + up as u16) / 2) as u8,
                    _ => paeth(left, up, upper_left),
                };
                stored.push(line[i].wrapping_sub(predicted));
            }
            previous = line.to_vec();
        }
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&stored).expect("deflate");
        let packed = encoder.finish().expect("deflate");

        let mut parms = super::super::Dict(Vec::new());
        parms.set(b"Predictor", Object::Number(b"15".to_vec()));
        parms.set(b"Colors", Object::Number(colors.to_string().into_bytes()));
        parms.set(b"Columns", Object::Number(columns.to_string().into_bytes()));
        let mut dict = super::super::Dict(Vec::new());
        dict.set(b"Filter", Object::Name(b"FlateDecode".to_vec()));
        dict.set(b"DecodeParms", Object::Dict(parms.clone()));
        assert_eq!(decode(&dict, &packed).as_deref(), Some(plain.as_slice()), "PNG predictors");

        // TIFF: each sample stored as the difference from the one before it.
        let mut differenced = plain.clone();
        for line in differenced.chunks_mut(row) {
            for i in (colors as usize..row).rev() {
                line[i] = line[i].wrapping_sub(line[i - colors as usize]);
            }
        }
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&differenced).expect("deflate");
        let packed = encoder.finish().expect("deflate");
        parms.set(b"Predictor", Object::Number(b"2".to_vec()));
        dict.set(b"DecodeParms", Object::Dict(parms.clone()));
        assert_eq!(decode(&dict, &packed).as_deref(), Some(plain.as_slice()), "TIFF predictor");

        // And what is not read is refused, not guessed: a predictor number
        // PDF does not define, and TIFF at a depth this does not do.
        parms.set(b"Predictor", Object::Number(b"7".to_vec()));
        dict.set(b"DecodeParms", Object::Dict(parms.clone()));
        assert!(decode(&dict, &packed).is_none());
        parms.set(b"Predictor", Object::Number(b"2".to_vec()));
        parms.set(b"BitsPerComponent", Object::Number(b"16".to_vec()));
        dict.set(b"DecodeParms", Object::Dict(parms));
        assert!(decode(&dict, &packed).is_none());
    }

    /// **A stream that inflates past the limit is not decoded.** Found by
    /// audit: a megabyte of zeros deflates to about a kilobyte and came back
    /// as a gigabyte on the first edit.
    #[test]
    fn a_stream_that_inflates_past_the_limit_is_refused() {
        use std::io::Write;
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        let block = vec![0u8; 1024 * 1024];
        let over = INFLATED_LIMIT as usize / block.len() + 1;
        for _ in 0..over {
            encoder.write_all(&block).expect("deflate");
        }
        let bomb = encoder.finish().expect("deflate");
        assert!(bomb.len() < 1024 * 1024, "the bomb did not compress: {} bytes", bomb.len());

        let mut dict = super::super::Dict(Vec::new());
        dict.set(b"Filter", Object::Name(b"FlateDecode".to_vec()));
        assert!(decode(&dict, &bomb).is_none(), "the bomb was inflated");

        // Under the limit, the same stream decodes.
        let mut small = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        small.write_all(&block).expect("deflate");
        let small = small.finish().expect("deflate");
        assert_eq!(decode(&dict, &small).map(|d| d.len()), Some(block.len()));
    }

    fn ops(bytes: &[u8]) -> Vec<Operation> {
        parse(bytes).expect("parse")
    }

    #[test]
    fn operands_belong_to_the_operator_that_follows_them() {
        let found = ops(b"1 0 0 1 72 720 cm");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].operator, b"cm");
        assert_eq!(found[0].operands.len(), 6);
    }

    #[test]
    fn the_text_showing_operators_are_recognised() {
        let found = ops(b"BT /F1 12 Tf (Hello) Tj [(A) -250 (B)] TJ ET");
        let showing: Vec<&[u8]> =
            found.iter().filter(|o| o.shows_text()).map(|o| o.operator.as_slice()).collect();
        assert_eq!(showing, vec![&b"Tj"[..], b"TJ"]);
    }

    /// The span has to cover the operands too, or cutting an operation leaves
    /// its arguments behind as gibberish.
    #[test]
    fn an_operations_span_covers_its_operands() {
        let stream = b"BT (Hello) Tj ET";
        let found = ops(stream);
        let tj = found.iter().find(|o| o.operator == b"Tj").expect("Tj");
        assert_eq!(&stream[tj.span.clone()], b"(Hello) Tj");
    }

    /// **The guarantee.** Everything not cut comes through byte for byte.
    #[test]
    fn removing_one_operation_leaves_every_other_byte_alone() {
        let stream = b"BT /F1 12 Tf 1 0 0 1 72 720 Tm (secret) Tj 0 -14 Td (public) Tj ET";
        let found = ops(stream);
        let secret = found
            .iter()
            .find(|o| o.operands.iter().any(|v| *v == Object::LiteralString(b"secret".to_vec())))
            .expect("the Tj");

        let edited = remove(stream, &[secret]);
        let text = String::from_utf8_lossy(&edited);
        assert!(!text.contains("secret"), "the words are still there: {text}");
        assert!(text.contains("(public) Tj"), "the surviving text was rewritten: {text}");
        assert!(text.contains("1 0 0 1 72 720 Tm"), "the state around it changed: {text}");
        assert!(text.contains("/F1 12 Tf"), "the font selection was lost: {text}");

        // And what is left is still a content stream.
        let again = ops(&edited);
        assert!(again.iter().any(|o| o.operator == b"ET"), "it no longer parses");
    }

    #[test]
    fn removing_nothing_returns_the_stream_unchanged() {
        let stream = b"BT (a) Tj ET";
        assert_eq!(remove(stream, &[]), stream.to_vec());
    }

    /// Two cuts must not run their neighbours together.
    #[test]
    fn cutting_two_operations_keeps_what_is_between_them_readable() {
        let stream = b"BT (one) Tj (two) Tj (three) Tj ET";
        let found = ops(stream);
        let first = &found[1];
        let third = &found[3];
        let edited = remove(stream, &[first, third]);
        let text = String::from_utf8_lossy(&edited);
        assert!(!text.contains("one") && !text.contains("three"), "{text}");
        assert!(text.contains("(two) Tj"), "{text}");
        assert!(ops(&edited).iter().any(|o| o.operator == b"Tj"), "it no longer parses");
    }

    /// **Inline image data is not scanned as operators.** Its bytes are as
    /// likely to spell `Tj` as anything else.
    #[test]
    fn an_inline_image_is_stepped_over_rather_than_read() {
        let mut stream = b"q BI /W 4 /H 4 ID ".to_vec();
        stream.extend_from_slice(b"(Tj noise ET [ )");
        stream.extend_from_slice(b" EI Q (after) Tj");
        let found = ops(&stream);

        let operators: Vec<&[u8]> = found.iter().map(|o| o.operator.as_slice()).collect();
        assert_eq!(operators, vec![&b"q"[..], b"Q", b"Tj"], "the image data was read as operators");
        assert!(found.last().expect("Tj").shows_text());
    }

    #[test]
    fn a_stream_that_stops_making_sense_is_refused() {
        assert!(parse(b"BT (unclosed").is_err());
        assert!(parse(b"] Tj").is_err());
    }

    // -- where the text is -------------------------------------------------

    #[test]
    fn a_text_matrix_places_the_string_it_precedes() {
        let found = ops(b"BT 1 0 0 1 72 720 Tm (here) Tj ET");
        let placed = origins(&found);
        assert_eq!(placed.len(), 1);
        assert_eq!((placed[0].x, placed[0].y), (72.0, 720.0));
    }

    /// `Td` moves relative to where the last line began, not to where the last
    /// string ended — the distinction that puts every wrapped paragraph in the
    /// wrong place if it is got wrong.
    #[test]
    fn each_line_moves_from_the_line_before_it() {
        let found = ops(b"BT 1 0 0 1 72 720 Tm (one) Tj 0 -14 Td (two) Tj 0 -14 Td (three) Tj ET");
        let placed = origins(&found);
        assert_eq!(placed.len(), 3);
        assert_eq!((placed[0].x, placed[0].y), (72.0, 720.0));
        assert_eq!((placed[1].x, placed[1].y), (72.0, 706.0));
        assert_eq!((placed[2].x, placed[2].y), (72.0, 692.0));
    }

    /// `T*` uses the leading, and `TD` sets it on the way past.
    #[test]
    fn the_leading_carries_the_next_line_down() {
        let found = ops(b"BT 1 0 0 1 0 100 Tm 20 TL (a) Tj T* (b) Tj ET");
        let placed = origins(&found);
        assert_eq!(placed[1].y, 80.0);

        let found = ops(b"BT 1 0 0 1 0 100 Tm 0 -30 TD (a) Tj T* (b) Tj ET");
        let placed = origins(&found);
        assert_eq!(placed[0].y, 70.0, "TD did not move the first line");
        assert_eq!(placed[1].y, 40.0, "TD did not set the leading");
    }

    /// The graphics state moves the text with it, and `Q` puts it back.
    #[test]
    fn the_current_transform_moves_the_text_and_unwinds() {
        let found = ops(b"q 1 0 0 1 100 200 cm BT 1 0 0 1 10 20 Tm (in) Tj ET Q BT 1 0 0 1 10 20 Tm (out) Tj ET");
        let placed = origins(&found);
        assert_eq!((placed[0].x, placed[0].y), (110.0, 220.0), "the transform was not applied");
        assert_eq!((placed[1].x, placed[1].y), (10.0, 20.0), "Q did not unwind it");
    }

    /// A scaling transform has to scale the offset too, not just shift it.
    #[test]
    fn a_scaling_transform_scales_where_the_text_lands() {
        let found = ops(b"q 2 0 0 2 0 0 cm BT 1 0 0 1 50 60 Tm (x) Tj ET Q");
        let placed = origins(&found);
        assert_eq!((placed[0].x, placed[0].y), (100.0, 120.0));
    }

    /// `'` moves down a line before drawing — miss that and every such string
    /// is reported one line high.
    #[test]
    fn the_quote_operators_move_down_before_they_draw() {
        let found = ops(b"BT 1 0 0 1 0 100 Tm 12 TL (first) Tj (second) ' ET");
        let placed = origins(&found);
        assert_eq!(placed[0].y, 100.0);
        assert_eq!(placed[1].y, 88.0, "the quote operator drew on the same line");
    }

    // -- cutting codes out of one operator ---------------------------------

    #[test]
    fn a_literal_strings_escapes_are_resolved_into_real_bytes() {
        let found = ops(br"(a\(b\)c) Tj");
        assert_eq!(pieces(&found[0]), vec![Piece::Codes(b"a(b)c".to_vec())]);

        // Octal, and a backslash-newline that continues the line.
        let found = ops(b"(A\\101\\n) Tj");
        assert_eq!(pieces(&found[0]), vec![Piece::Codes(b"AA\n".to_vec())]);
    }

    #[test]
    fn a_hex_string_becomes_the_bytes_it_spells() {
        let found = ops(b"<48656C6C6F> Tj");
        assert_eq!(pieces(&found[0]), vec![Piece::Codes(b"Hello".to_vec())]);
        // An odd digit is padded with zero, as the specification says.
        let found = ops(b"<4A5> Tj");
        assert_eq!(pieces(&found[0]), vec![Piece::Codes(vec![0x4A, 0x50])]);
    }

    #[test]
    fn a_tj_array_keeps_its_spacing_numbers_between_the_strings() {
        let found = ops(b"[(A) -250 (B)] TJ");
        assert_eq!(
            pieces(&found[0]),
            vec![
                Piece::Codes(b"A".to_vec()),
                Piece::Kern(-250.0),
                Piece::Codes(b"B".to_vec())
            ]
        );
    }

    /// **The point of the whole thing.** Some codes go, the rest stay, and a
    /// spacing number holds them where they were.
    #[test]
    fn cutting_codes_keeps_the_rest_and_closes_the_gap() {
        let found = ops(b"(ABCDEF) Tj");
        // Drop C and D — codes 2 and 3 — which were 12pt wide on the page.
        let rebuilt = without_codes(&found[0], &[(2, 6.0), (3, 6.0)], b"F1", 24.0, 1.0, 1);
        let text = String::from_utf8_lossy(&rebuilt);

        assert!(text.contains("<4142>"), "the text before the gap went: {text}");
        assert!(text.contains("<4546>"), "the text after the gap went: {text}");
        assert!(!text.contains("4344"), "the cut codes are still there: {text}");
        // 12pt at size 24 is half an em, so -500 thousandths.
        assert!(text.contains("-500"), "the gap was not closed: {text}");
        assert!(text.starts_with("/F1 24 Tf"), "the font was not selected: {text}");
    }

    /// Cutting everything leaves nothing to draw, and nothing to draw it with.
    #[test]
    fn cutting_every_code_leaves_no_operator_at_all() {
        let found = ops(b"(AB) Tj");
        assert!(without_codes(&found[0], &[(0, 10.0), (1, 10.0)], b"F1", 10.0, 1.0, 1).is_empty());
    }

    /// A two-byte font counts glyphs, not bytes — cutting code 1 of a CID
    /// string must take the second *pair*, not the second byte.
    #[test]
    fn a_two_byte_font_is_cut_at_glyph_boundaries() {
        let found = ops(b"<00410042004300 44> Tj");
        let rebuilt = without_codes(&found[0], &[(1, 0.0)], b"F1", 10.0, 1.0, 2);
        let text = String::from_utf8_lossy(&rebuilt);
        assert!(text.contains("0041"), "the first glyph went: {text}");
        assert!(!text.contains("0042"), "the cut glyph is still there: {text}");
        assert!(text.contains("0043"), "the third glyph went: {text}");
    }

    /// The numbers already in a `TJ` are the file's own kerning and must
    /// survive, or every cut line re-spaces itself.
    #[test]
    fn the_files_own_kerning_is_kept() {
        let found = ops(b"[(AB) -35 (CD)] TJ");
        let rebuilt = without_codes(&found[0], &[(3, 0.0)], b"F1", 10.0, 1.0, 1);
        let text = String::from_utf8_lossy(&rebuilt);
        assert!(text.contains("-35"), "the original kerning was dropped: {text}");
        assert!(text.contains("<4142>") && text.contains("<43>"), "{text}");
        assert!(!text.contains("<4344>"), "D was not cut: {text}");
    }

    /// **The gap depends on the matrix as well as the font size.**
    ///
    /// Reported from use as the surviving text being shoved sideways: a
    /// producer that writes `1 Tf` and puts the real size in the text matrix
    /// made every gap an order of magnitude too wide.
    #[test]
    fn the_gap_accounts_for_the_text_matrix_not_only_the_font_size() {
        let found = ops(b"(ABCD) Tj");

        // 6pt of glyphs removed at size 24, matrix scale 1 — half an em.
        let plain = without_codes(&found[0], &[(1, 12.0)], b"F1", 24.0, 1.0, 1);
        assert!(String::from_utf8_lossy(&plain).contains("-500"), "{:?}", String::from_utf8_lossy(&plain));

        // The same text at `1 Tf` with the size in the matrix must give the
        // same displacement, not twenty-four times it.
        let scaled = without_codes(&found[0], &[(1, 12.0)], b"F1", 1.0, 24.0, 1);
        assert!(
            String::from_utf8_lossy(&scaled).contains("-500"),
            "the matrix scale was ignored: {:?}",
            String::from_utf8_lossy(&scaled)
        );
    }

    /// **Kerning inside the removed run goes with it.**
    ///
    /// The advance handed in is measured from one glyph's origin to the next,
    /// so it already crosses whatever spacing lay between them. Keeping those
    /// numbers as well applies the displacement twice, and the surviving text
    /// drifts — measured on real pages at one to four points.
    #[test]
    fn spacing_numbers_inside_the_gap_are_dropped_with_it() {
        // Remove B and C, which have a kern between them.
        let found = ops(b"[(AB) -40 (CD)] TJ");
        let out = without_codes(&found[0], &[(1, 5.0), (2, 5.0)], b"F1", 10.0, 1.0, 1);
        let text = String::from_utf8_lossy(&out);

        assert!(!text.contains("-40"), "the kerning inside the gap was kept: {text}");
        assert!(text.contains("<41>") && text.contains("<44>"), "{text}");
    }

    /// And kerning outside it stays, or the line re-spaces itself.
    #[test]
    fn spacing_numbers_outside_the_gap_are_kept() {
        let found = ops(b"[(AB) -40 (CD)] TJ");
        let out = without_codes(&found[0], &[(3, 5.0)], b"F1", 10.0, 1.0, 1);
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("-40"), "the file's own kerning was dropped: {text}");
    }

    /// **What makes a line one line.** Two strings drawn without a move down
    /// between them continue one another — which is how a line's hyphen,
    /// written as a `Tj` of its own, is known to belong to the line before it.
    #[test]
    fn strings_with_no_move_between_them_are_one_line() {
        let found = ops(b"BT 1 0 0 1 0 100 Tm (word) Tj (-) Tj 0 -12 Td (next) Tj ET");
        let placed = placed(&found);
        assert_eq!(placed.len(), 3);
        assert_eq!(placed[0].line, placed[1].line, "the hyphen was put on its own line");
        assert_ne!(placed[1].line, placed[2].line, "the next line was not separated");
    }

    /// A `Tm` that only slides along the line does not start a new one; one
    /// that changes the vertical does.
    #[test]
    fn a_horizontal_reposition_stays_on_the_same_line() {
        let found = ops(b"BT 1 0 0 1 0 100 Tm (a) Tj 1 0 0 1 50 100 Tm (b) Tj 1 0 0 1 0 88 Tm (c) Tj ET");
        let placed = placed(&found);
        assert_eq!(placed[0].line, placed[1].line, "a sideways move split the line");
        assert_ne!(placed[1].line, placed[2].line, "a move down did not split it");
    }

    /// Each `T*` and each text object starts a line of its own.
    #[test]
    fn the_line_operators_each_begin_a_new_line() {
        let found = ops(b"BT 12 TL (a) Tj T* (b) Tj ET BT (c) Tj ET");
        let placed = placed(&found);
        assert_ne!(placed[0].line, placed[1].line, "T* did not begin a line");
        assert_ne!(placed[1].line, placed[2].line, "BT did not begin a line");
    }

    /// **Editing words changes only those words.**
    #[test]
    fn replacing_codes_keeps_what_is_either_side() {
        let found = ops(b"(ABCDEF) Tj");
        // Swap C and D for a single new code.
        let out = replacing_codes(&found[0], 2..4, &[0x5A], b"F1", 12.0, 1);
        let text = String::from_utf8_lossy(&out);

        assert!(text.contains("<4142>"), "the text before it went: {text}");
        assert!(text.contains("<5A>"), "the new text is not there: {text}");
        assert!(text.contains("<4546>"), "the text after it went: {text}");
        assert!(!text.contains("4344"), "the old text is still there: {text}");
    }

    /// The file's own kerning outside the edit survives it.
    #[test]
    fn replacing_codes_keeps_kerning_outside_the_edit() {
        let found = ops(b"[(AB) -35 (CD)] TJ");
        let out = replacing_codes(&found[0], 0..1, &[0x5A], b"F1", 12.0, 1);
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("-35"), "the kerning was dropped: {text}");
        assert!(text.contains("<42>") && text.contains("<4344>"), "{text}");
    }

    /// A degenerate scale must not produce an infinite displacement.
    #[test]
    fn a_zero_scale_closes_no_gap_rather_than_dividing_by_it() {
        let found = ops(b"(ABCD) Tj");
        let out = without_codes(&found[0], &[(1, 12.0)], b"F1", 0.0, 0.0, 1);
        let text = String::from_utf8_lossy(&out);
        assert!(!text.contains("inf") && !text.contains("NaN"), "{text}");
    }

    // -- filters -----------------------------------------------------------

    #[test]
    fn a_deflated_stream_round_trips() {
        let original = b"BT /F1 12 Tf (round trip) Tj ET";
        let packed = encode(original).expect("encode");
        let mut dict = super::super::Dict::default();
        dict.set(b"Filter", Object::Name(b"FlateDecode".to_vec()));
        assert_eq!(decode(&dict, &packed).expect("decode"), original.to_vec());
    }

    #[test]
    fn an_unfiltered_stream_needs_no_decoding() {
        let dict = super::super::Dict::default();
        assert_eq!(decode(&dict, b"BT ET").expect("decode"), b"BT ET".to_vec());
    }

    /// A filter this does not understand is `None`, not a guess — the caller
    /// leaves that page alone rather than writing nonsense into it.
    #[test]
    fn a_filter_this_does_not_know_is_refused() {
        let mut dict = super::super::Dict::default();
        dict.set(b"Filter", Object::Name(b"LZWDecode".to_vec()));
        assert!(decode(&dict, b"anything").is_none());

        // A chain, even one that includes Flate.
        dict.set(
            b"Filter",
            Object::Array(vec![
                Object::Name(b"ASCII85Decode".to_vec()),
                Object::Name(b"FlateDecode".to_vec()),
            ]),
        );
        assert!(decode(&dict, b"anything").is_none());
    }

    /// An empty `/DecodeParms` names no predictor, which is predictor 1: the
    /// bytes as inflated. A predictor this does not read is refused rather
    /// than ignored — ignoring one would hand back something that is not
    /// the stream.
    #[test]
    fn a_predictor_this_does_not_read_is_refused_rather_than_ignored() {
        let mut dict = super::super::Dict::default();
        dict.set(b"Filter", Object::Name(b"FlateDecode".to_vec()));
        dict.set(b"DecodeParms", Object::Dict(super::super::Dict::default()));
        assert_eq!(decode(&dict, &encode(b"x").expect("encode")).as_deref(), Some(&b"x"[..]));

        let mut parms = super::super::Dict::default();
        parms.set(b"Predictor", Object::Number(b"3".to_vec()));
        dict.set(b"DecodeParms", Object::Dict(parms));
        assert!(decode(&dict, &encode(b"x").expect("encode")).is_none());
    }
}
