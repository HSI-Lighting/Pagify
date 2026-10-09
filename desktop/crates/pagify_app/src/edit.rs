//! Picking the text run or paragraph a click names, building the editor over
//! it, and applying what was typed back to the page — the engine-facing side
//! of `doc_tab/edit.rs` from the review. The planning and page-geometry
//! helpers these call (`paragraph_lines`, `fix_extracted_text`,
//! `join_paragraph_lines`, and the rest) are not moved here; they stayed
//! useful to more than this file and are not this split's concern.

use crate::paragraph_lines::{asks_for_width, paragraph_applied_message, removed_pieces};
use crate::{
    explain, fix_extracted_text, hyphens_before_drawn_lines, join_paragraph_lines, majority_look, page_is_heavy,
    union_rect, wrap_hyphen_marks, EditingRun, LineEnd, ParagraphCommands, ParagraphLine, RunBox,
};
use pagify_shell::block_input::{self, looks_rotated, PageBlocks, PickPath, PickTrace};
use pagify_shell::markup::HIT_TOLERANCE_PT;
use pagify_shell::page_space::AppPoint;
use pagify_shell::PageRaster;
use pdf_core::document::Color;

impl crate::PagifyApp {
    /// The body of [`Self::pick_text_run`], filling `trace` as it goes.
    ///
    /// The run is the unit because it is what the file holds — one may be a
    /// whole paragraph, or a single letter that needed different spacing.
    /// Offering "the word you clicked" would mean rewriting the content stream
    /// around it, which is a different and much larger feature. What a click
    /// opens is decided in this order:
    ///
    /// 0. **how heavy the page is**, counted before any of it is read
    ///    ([`Self::page_weight`]): past [`HEAVY_PAGE_TEXT_OBJECTS`] text objects or
    ///    [`HEAVY_PAGE_OBJECTS`] objects the page is not read for paragraphs at all
    ///    — step 1 is made on its text objects read once ([`Self::light_page`]),
    ///    steps 3 and 4 are skipped, and the click opens the word alone, without
    ///    the picture or the font the other clicks fetch;
    /// 1. the text object under the point ([`block_input::pick_seed`], over
    ///    every text object with area on the page, with a few points of slack
    ///    because the target is a few pixels tall) — none: a word drawn as
    ///    outlines, else why there is nothing;
    /// 2. rotated text is refused;
    /// 3. a paragraph the person **joined by hand** overrides everything
    ///    below — after it has been checked against the page
    ///    ([`Self::forget_stale_groups`]);
    /// 4. the seed's **paragraph** as the detector found it, when it holds
    ///    more than one text object, passes the editor's invariants
    ///    ([`block_input::check_editor_invariants`] — a block that fails them
    ///    degrades to the next line, with the reason logged and said, never to
    ///    damage) **and the clicked words are not on a line the page draws part
    ///    of as shapes** (such a line is never written, so a box over it could not
    ///    change what was clicked);
    /// 5. the one run, as the tool always did — with a sentence at the head of the
    ///    message saying why, when it is a paragraph that did not open.
    pub(crate) fn pick_text_run_traced(
        &mut self,
        page: usize,
        at: AppPoint,
        trace: &mut PickTrace,
    ) -> Result<String, String> {
        if self.tab().doc.is_none() {
            return Err("nothing open.".into());
        }
        let (x, y) = (at.x as f32, at.y as f32);

        // **How big the page is, before any of it is read** — one count, kept
        // for as long as the page is as it is. A page too heavy to read in one
        // pass (see `page_is_heavy`) gets no paragraphs: its words are the single
        // runs, found from their rectangles and read one at a time — the click is
        // not made to wait for a read of tens of thousands of objects, which
        // froze the whole window for seconds (and, at 877 000 shapes, for tens of
        // seconds) with nothing on screen to say why.
        let heavy = self.page_weight(page).filter(page_is_heavy);

        // **One read of the page for the whole click**, cached — see
        // `page_blocks`. If it cannot be made, the tool still works on the
        // run alone.
        let (pb, run): (Option<std::rc::Rc<PageBlocks>>, pdf_core::document::TextRun) =
            if let Some(weight) = heavy {
                self.session_log.record(
                    "pick-note",
                    &format!(
                        "page {} holds {} objects, {} of them text: too many to read for paragraphs; picking the run alone",
                        page + 1,
                        weight.page_objects,
                        weight.text_objects
                    ),
                );
                match self.heavy_run_under(page, x, y)? {
                    Some((run, rule)) => {
                        trace.seed = Some(run.object);
                        trace.rule = Some(rule);
                        (None, run)
                    }
                    None => return self.no_text_here(page, at, trace),
                }
            } else {
            match self.page_blocks(page) {
                Ok((pb, hit)) => {
                    trace.page_facts(&pb);
                    trace.cache_hit = hit;
                    if hit {
                        // The cost of the reading the click found waiting is not
                        // a cost of this click (`page_facts` carries the cost of
                        // the one that made it): a hit's line says it cost nothing.
                        trace.build_ms = 0.0;
                        trace.detect_ms = 0.0;
                    }
                    // **How far outside its own box a run will still answer
                    // to a click** (`HIT_TOLERANCE_PT`): the target is a few
                    // pixels tall. Measured at the zoom these documents open
                    // at: the median run is 5.1 pixels high in one and 6.7 in
                    // the other. Asking somebody to land inside a five-pixel
                    // band is not a reasonable thing to ask, and missing it
                    // looks exactly like the tool being broken.
                    let Some((seed, rule)) = block_input::pick_seed(&pb, x, y, HIT_TOLERANCE_PT as f32)
                    else {
                        return self.no_text_here(page, at, trace);
                    };
                    trace.seed = Some(seed);
                    trace.rule = Some(rule);
                    let Some(run) = pb.runs.get(&seed).cloned() else {
                        return Err("no text there — click on some words.".into());
                    };
                    (Some(pb), run)
                }
                Err(why) => {
                    self.session_log.record(
                        "pick-note",
                        &format!("the page's text could not be read in one pass ({why}); picking the run alone"),
                    );
                    match self.legacy_run_under(page, x, y)? {
                        Some((run, rule)) => {
                            trace.seed = Some(run.object);
                            trace.rule = Some(rule);
                            (None, run)
                        }
                        None => return self.no_text_here(page, at, trace),
                    }
                }
            }
            };
        let unreadable = run.text.trim().is_empty();

        // **Reported from use, with a screenshot: clicking near a rotated
        // dimension label ("54mm", turned on its side next to a technical
        // drawing) opened an editor that made no sense — a box a few points
        // wide claiming to hold a whole word, one letter to a line.**
        // `pdf_core::document::TextRun` carries no rotation angle at all, so
        // this cannot be read directly; `looks_rotated` infers it from the
        // box's own shape instead. Refused outright rather than opened
        // wrong: every box, wrap and font-size computation from here down
        // assumes the text reads left to right along the rect's own width,
        // and a run turned on its side breaks that assumption at the root,
        // not at any one place worth patching around.
        if looks_rotated(&run.rect, run.text.trim().chars().count()) {
            trace.path = PickPath::RefusedRotated;
            return Err(
                "that text is rotated on the page — editing rotated text is not \
                 supported yet."
                    .into(),
            );
        }

        // **A person's own join overrides the geometry.** Checked before the
        // detector's paragraph so a manually joined block stays joined even
        // where its members would never be one on their own — a different
        // face, a gap wider than the detector allows, an unrelated column of
        // a table in between.
        //
        // **Checked against the page first, and only with the page in hand.** A
        // group names runs by number and by how they read when it was made; one
        // whose runs no longer read that way is forgotten (`forget_stale_groups`)
        // before it can open an editor over other words. When the page could not be
        // read at all there is nothing to check it against, and the group is
        // neither used nor forgotten: it is the page that is unreadable, not the
        // join that is wrong, and it comes back with the next click that can read.
        if !unreadable && run.color.a != 0 {
            if let Some(pb) = pb.as_ref() {
                self.forget_stale_groups(pb);
                if let Some(group_index) = self.group_containing(page, run.object) {
                    let objects = self.tab_mut().joined_groups[group_index].objects.clone();
                    match self.open_joined_editor(page, &objects, pb) {
                        Ok(message) => {
                            trace.path = PickPath::Joined;
                            return Ok(message);
                        }
                        // A group that matches the page and still cannot be
                        // opened is no use to anybody: forgotten rather than
                        // left to fail the same way every time, and the click
                        // falls through to the ordinary geometry below.
                        Err(_) => {
                            self.tab_mut().joined_groups.remove(group_index);
                        }
                    }
                }
            }
        }

        // **Edit Text retypes a whole paragraph at once, not one line of
        // it.** Readable, written (not drawn) text only, and only when the
        // paragraph holds more than one text object: a block of one object
        // is the run itself, and opens as it always did. A block that fails
        // the editor's invariants is not opened, and says why in the log.
        //
        // **Except when the clicked words are on a line the page draws part of as
        // shapes** (a currency symbol drawn as a path beside an amount, an
        // outlined word in the middle of a sentence): such a line is never
        // written, so a box over it could not change what was clicked — it said
        // "left as it is" and dropped the new amount, where the single run of that
        // piece can be retyped. The paragraph is still one click away on any of its
        // written lines. Chosen over keeping the box for large blocks because the
        // click names the words the person wants to change, and this is the way
        // they can be.
        //
        // **Why a click that could have opened a paragraph opens the word alone**,
        // said to the person as well as to the log — one short sentence at the head
        // of the pick's message — so that a box that is smaller than expected is
        // never a mystery.
        let mut alone: Option<&str> = heavy.map(|_| "this page is very large, so its paragraphs are not read");
        let mut seed_is_drawn_lettering = false;
        if let Some(pb) = pb.as_ref().filter(|_| !unreadable && run.color.a != 0) {
            if let Some(&(block, line)) = pb.by_object.get(&run.object) {
                trace.block_facts(pb, block);
                if trace.objects > 1 {
                    if pb.blocks[block].lines[line].outlined.is_empty() {
                        match self.open_block(page, pb, block) {
                            Ok(message) => {
                                trace.path = PickPath::Block;
                                return Ok(message);
                            }
                            Err(why) => {
                                self.session_log.record(
                                    "pick-note",
                                    &format!("block {block} not opened ({why}); picking the run alone"),
                                );
                                alone = Some("the block could not be read safely");
                            }
                        }
                    } else {
                        seed_is_drawn_lettering = true;
                        alone = Some("its line is partly drawn as shapes, which cannot be retyped here");
                        self.session_log.record(
                            "pick-note",
                            &format!(
                                "block {block} not opened (the clicked words are on line {line}, which has drawn \
                                 lettering in it); picking the run alone"
                            ),
                        );
                    }
                }
            }
        }

        trace.path = PickPath::Single;
        let readable_text = fix_extracted_text(&run.text);
        let picked_char_count = readable_text.chars().count();
        // A mark on a line of drawn lettering that has no words of its own — the
        // hyphen glyph the page ends such a line with — is nothing to retype.
        if seed_is_drawn_lettering && readable_text.trim().is_empty() {
            trace.path = PickPath::None;
            return Err(
                "that mark belongs to a line the page draws as shapes, and has no words of its own to retype — \
                 click the words beside it."
                    .into(),
            );
        }
        self.tab_mut().editing_run = Some(EditingRun {
            page,
            object: run.object,
            look_object: run.object,
            original: readable_text.clone(),
            rect: run.rect,
            lines: vec![(vec![run.object], run.rect)],
            frozen: vec![false],
            twins: Vec::new(),
            doc_generation: self.doc_generation(),
            render_epoch: self.render_epoch(),
            refusal: None,
            buffer: readable_text,
            // Seeded from the run, so leaving the controls alone changes
            // nothing about how it looks.
            //
            // **From the baseline, not the top of the box.** Those differ by
            // the font's ascent, and `at` means the baseline — seeding it from
            // `rect.top` moved a restyled run up by most of its own height,
            // onto the line above. See `TextRun::origin`.
            style: pdf_core::document::TextStyle {
                size: Some(run.size),
                color: Some(run.color),
                at: Some((run.origin.x, run.origin.y)),
                face: None,
            },
            was: pdf_core::document::TextStyle {
                size: Some(run.size),
                color: Some(run.color),
                at: Some((run.origin.x, run.origin.y)),
                face: None,
            },
            // From the page's own read when there is one: no further call.
            // **Three things a click on a very large page does without**: each is a
            // further open of the page (or, for the background, a render of all of
            // it), and on such a page that is what a click costs. The box is
            // drawn on white, in the program's own face, with the face's name
            // left off its font field; what is typed is written the same.
            current_face: match &pb {
                Some(pb) => pb.faces.get(&run.object).cloned(),
                None if heavy.is_some() => None,
                None => self.tab()
                    .doc
                    .as_ref()
                    .and_then(|d| d.session.run_font_name(page, run.object).ok())
                    .flatten(),
            },
            background: if heavy.is_some() {
                Color { r: 255, g: 255, b: 255, a: 255 }
            } else {
                self.page_behind(page, run.rect, run.color)
            },
            drawn: run.color.a == 0,
            focused: false,
            box_resize: RunBox::default(),
        });
        // The face these words are already in, for the field to be set in. Asked
        // for here and used a frame or two later — see `want_document_face`.
        if heavy.is_some() {
            self.no_document_face();
        } else {
            self.want_document_face_for(
                page,
                run.object,
                pb.as_ref().and_then(|pb| pb.styles.get(&run.object)).map(|style| style.font),
            );
        }

        // **Invisible words are an extracted layer, not the printed page.**
        //
        // `extracttext` writes what it recognises as fully transparent text
        // over the artwork, so the page can be searched and copied from. The
        // words a reader *sees* on such a page are vector outlines — type
        // converted to paths when the file was made — and nothing done to the
        // layer touches them.
        //
        // Reported from use, and it is the worst kind of failure: the edit
        // appears to work, the reported text changes, and the page looks
        // exactly as it did. Said at the moment of picking, before anything is
        // typed.
        if run.color.a == 0 {
            return Ok(
                "these words are drawn, not written — type converted to outlines. \
                 Changing them takes the drawn shapes off the page and writes real \
                 text in their place, in a face that will not match. Escape to \
                 leave them."
                    .into(),
            );
        }

        if unreadable {
            // Visible on the page, and the file cannot say what it says: the
            // font carries no `/ToUnicode`, so there is nothing to read out.
            // Offered anyway — it can still be replaced — but not pretending
            // the box is showing what is there.
            return Ok("these words are on the page but the file does not say what they \
                       are. Type a replacement, or Escape to leave them."
                .into());
        }
        // **The character count is the diagnostic, not decoration.** A
        // report of text that "still shows up but disappears when I try to
        // edit it" is otherwise impossible to place: it could mean the
        // document's own text is already blank at the moment it's picked
        // (a real hidden/corrupted run — the screen is stale, not wrong),
        // or that it reads normally here and something else blanks it later
        // (a different bug entirely). Logging the length at pick time lets
        // the next report be matched against this line instead of guessed at.
        Ok(match alone {
            None => format!(
                "edit the words on the page ({picked_char_count} characters) — Apply to keep, Escape to leave them."
            ),
            Some(why) => format!(
                "opened this word alone: {why} ({picked_char_count} characters) — Apply to keep, Escape to leave them."
            ),
        })
    }

    /// The editor for lines that are already decided — **including frozen
    /// ones**: lines that carry words the page draws as shapes, which applying
    /// an edit never touches (see [`EditingRun::frozen`]).
    ///
    /// A frozen line may have no text object at all (a whole line drawn as
    /// shapes). Its place in the buffer is [`block_input::OUTLINED_PLACEHOLDER`]
    /// — one entry for every line, so the buffer has exactly one
    /// `\n`-separated entry per line, which is what
    /// [`Self::paragraph_edit_commands`] checks before it writes anything, and
    /// the person can see that something is there that this box cannot retype.
    /// [`EditingRun::object`] is the first object of the first line that has
    /// one, and where *no* line has one there is nothing to put an editor on:
    /// this says so instead of inventing an object.
    ///
    /// `runs`, `face_names` and `styles` need cover only the objects of
    /// `lines`; `styles` (font identity, from [`Session::run_styles`]) is what
    /// the dominant look is decided on. Returns the object whose look won the
    /// majority vote, which is also [`EditingRun::look_object`].
    pub(crate) fn build_editor_from_lines(
        &mut self,
        page: usize,
        lines: Vec<ParagraphLine>,
        runs: &[pdf_core::document::TextRun],
        face_names: &std::collections::HashMap<usize, String>,
        styles: &std::collections::HashMap<usize, pdf_core::document::RunStyle>,
        shapes: &[pdf_core::document::DrawnObject],
        page_raster: Option<&PageRaster>,
    ) -> Result<(EditingRun, usize), String> {
        let Some(seed_object) = lines.iter().flat_map(|line| line.objects.iter()).copied().next() else {
            return Err("nothing there can be edited: every line of it is drawn as shapes.".into());
        };
        // Not a second read of the page — the caller already paid for one to
        // find the seed line, and that extraction is real work on a page of
        // any size. Asking again here is exactly the "a second or two to
        // open" this was reported as.
        let text_by_object: std::collections::HashMap<usize, &pdf_core::document::TextRun> =
            runs.iter().map(|r| (r.object, r)).collect();

        // A line's text is its objects' own, left to right, **on one line**: a
        // newline inside a text object would otherwise split the paragraph's
        // text into more entries than it has lines, and every later line
        // would be matched against the wrong object on the way back out. (No
        // real page measured so far has one; a producer is free to.) A line
        // with no text object stands for words drawn as shapes.
        let mut line_texts: Vec<String> = lines
            .iter()
            .map(|line| {
                if line.objects.is_empty() {
                    return block_input::OUTLINED_PLACEHOLDER.to_string();
                }
                line.objects
                    .iter()
                    .map(|o| text_by_object.get(o).map(|r| r.text.as_str()).unwrap_or(""))
                    .collect::<String>()
                    .replace(['\r', '\n'], " ")
            })
            .collect();
        // A hyphen the page's text carries as a control code, at the end of a
        // line that the page continues with a line it draws as shapes: the letter
        // that should follow it cannot be looked at, so it is settled here.
        let drawn: Vec<bool> = lines.iter().map(|line| line.frozen).collect();
        hyphens_before_drawn_lines(&mut line_texts, &drawn);
        // **Where each line ends, to find a hyphen the page draws there.**
        // The row's rightmost run is the line's last glyph, and its own
        // baseline and size are the line's — see `wrap_hyphen_marks`. A line
        // with no text object has no last glyph and so no mark to find.
        let ends: Vec<Option<LineEnd>> = lines
            .iter()
            .map(|line| {
                let right_of = |r: &pdf_core::document::TextRun| r.rect.left.max(r.rect.right);
                line.objects
                    .iter()
                    .filter_map(|o| text_by_object.get(o).copied())
                    .max_by(|a, b| right_of(a).total_cmp(&right_of(b)))
                    .map(|last| LineEnd { right: right_of(last), baseline: last.origin.y, em: last.size })
            })
            .collect();
        let hyphen_after = wrap_hyphen_marks(&ends, shapes);
        // Fixed up as one whole block, after joining — not line by line
        // before it. A hyphen that happens to fall exactly on a line wrap
        // (see `fix_extracted_text`'s own doc) has a letter on one side and
        // a `\n` on the other until the lines are joined; sanitising each
        // line in isolation would see no letter following it and drop it,
        // exactly where a real hyphen is most likely to occur.
        let combined = fix_extracted_text(&join_paragraph_lines(&line_texts, &hyphen_after));

        // Every line's own box, frozen ones included: the editor is drawn
        // over the whole block.
        let union = lines.iter().skip(1).fold(lines[0].rect, |acc, line| union_rect(acc, &line.rect));

        // The *look* is taken from whichever face+size most of the
        // paragraph's text actually uses — not always the first line, and
        // not just one vote per line: a line that is itself more than one
        // run (see above) counts every run, the same as it would if those
        // runs happened to fall on separate lines.
        //
        // **Reported from use, with a screenshot**: a paragraph whose first
        // line was a bold "Description:" heading opened with its entire
        // multi-line body rendered in that same bold, oversized face, even
        // though every line beneath it was ordinary body text. `TextEdit`
        // draws in one font for the whole box, so *something* has to be
        // outvoted — and the line that started the paragraph is not
        // entitled to override the five lines under it just by coming
        // first.
        //
        // **Weighed by how much text each fragment holds**, not one vote a
        // piece — see `majority_look`'s own doc. The look is `(font, size)`,
        // the font as the page's own identity for it (`RunStyle::font`: one
        // id per font resource), **not its name**: the real datasheet names
        // five different weights "Montserrat-Thin", so a name vote cannot
        // tell its headings from its body.
        let lines_by_look: Vec<(usize, (u32, u32), usize)> = lines
            .iter()
            .flat_map(|line| line.objects.iter())
            .map(|object| {
                let font = styles.get(object).map_or(u32::MAX, |style| style.font);
                let run = text_by_object.get(object);
                let size_bits = run.map(|r| r.size.to_bits()).unwrap_or(0);
                let ink = run.map(|r| r.text.chars().filter(|c| !c.is_whitespace()).count()).unwrap_or(0);
                (*object, (font, size_bits), ink)
            })
            .collect();
        let look_object = majority_look(&lines_by_look).unwrap_or(seed_object);

        // Position stays anchored to the first line that has text — where the
        // paragraph starts, not what it mostly looks like.
        let at = match text_by_object.get(&seed_object) {
            Some(r) => (r.origin.x, r.origin.y),
            None => (union.left, union.bottom),
        };
        let (size, color) = match text_by_object.get(&look_object) {
            Some(r) => (r.size, r.color),
            None => (12.0, pdf_core::document::Color { r: 20, g: 20, b: 20, a: 255 }),
        };
        let style = pdf_core::document::TextStyle {
            size: Some(size),
            color: Some(color),
            at: Some(at),
            face: None,
        };

        let edit = EditingRun {
            page,
            object: seed_object,
            look_object,
            original: combined.clone(),
            rect: union,
            frozen: lines.iter().map(|line| line.frozen).collect(),
            // Set by the caller that knows them — see `pick_paragraph`.
            twins: Vec::new(),
            doc_generation: self.doc_generation(),
            render_epoch: self.render_epoch(),
            refusal: None,
            lines: lines.into_iter().map(|line| (line.objects, line.rect)).collect(),
            buffer: combined,
            style: style.clone(),
            was: style,
            current_face: face_names.get(&look_object).cloned(),
            background: Self::background_at(page_raster, union, color),
            drawn: false,
            focused: false,
            box_resize: RunBox::default(),
        };
        Ok((edit, look_object))
    }

    /// The next line submitted replaces the run that was picked.
    ///
    /// Intercepted before the command box sees it, the same way a password is:
    /// the words being typed are text, not a command, and dispatching them
    /// would answer `unknown command` to somebody's sentence.
    pub(crate) fn apply_one_edit(&mut self, edit: EditingRun) {
        let typed = edit.buffer.trim().to_string();

        // **Everything typed away deletes the text.** This used to say "left as it
        // was" and do nothing, so a paragraph could be cut down to one character
        // but never to none (reported from use). The paragraph path already reads
        // an empty buffer as "every line removed" — twins and all, frozen lines
        // left alone — and a single run is the one-line case of it, so it is
        // one undo step like any other edit.
        if typed.is_empty() {
            if edit.original.trim().is_empty() {
                self.say_info("left as it was.");
            } else if edit.drawn {
                self.say_error(
                    "those words are drawn as shapes, not text, so they cannot be deleted here — \
                     pick them with Edit Object instead.",
                );
            } else {
                self.apply_paragraph_edit(&edit, &edit.buffer);
            }
            return;
        }
        // **A font pick is judged on its own, not lumped in with size, colour
        // and position.** Those three go through PDFium's own text-object
        // rewrite when any of them changes (see the `style:` sent below); a
        // font pick instead travels the byte-safe path `set_run_in_stream`
        // already uses for the words themselves — see
        // `set_text_run_styled`'s fast-path gate in pdf_core, which checks
        // for exactly this shape (`face` set, everything else left alone).
        // Comparing the *whole* style here would send a font-only pick down
        // the slow path instead, where nothing reads `face` at all.
        let changed_look = pdf_core::document::TextStyle { face: None, ..edit.style.clone() }
            != pdf_core::document::TextStyle { face: None, ..edit.was.clone() };
        let picked_font = edit.style.face.is_some();
        if typed == edit.original.trim() && !changed_look && !picked_font {
            self.say_info("unchanged.");
            return;
        }

        // **Words that are artwork are replaced, not edited.**
        //
        // There is no text on the page to change — see `replace_outlined_word`,
        // which takes the drawn shapes off and writes real words in their place.
        if edit.drawn {
            let at = edit.style.at.unwrap_or((edit.rect.left, edit.rect.bottom));
            let size = edit.style.size.unwrap_or(12.0);
            match self
                .replace_outlined_word(edit.page, edit.rect, at, size, &edit.original, &typed)
            {
                Ok(said) => self.say_info(said),
                Err(e) => self.say_error(e),
            }
            return;
        }

        // **More than one line, or more than one object, or a frozen line —
        // any one of them is a paragraph.** A producer routinely splits one
        // visual *line* across more than one run — see `pagify_shell::blocks`'
        // own doc — so a single-row pick can still carry several objects in
        // `edit.lines[0].0`. `apply_paragraph_edit` (via
        // `paragraph_edit_commands`) already handles that correctly: it
        // writes the new text into the first object on a line and removes the
        // rest from the page, so no stale sibling is left drawing its own old
        // text. The row-count check this used to be routed a multi-object single row
        // into the plain path instead, which rewrites only `edit.object` — the
        // row's first object — and never touches its siblings at all, which
        // then kept showing their own untouched original text right next to
        // the new words. **Reported from use, with a screenshot**: "CCT:3500K,"
        // retyped as "CCT:3500K, 5000" instead of replacing it, each further
        // edit compounding more of the same stale text onto the end.
        //
        // **And the object count alone is not enough either.** A block of one
        // text object and a line the page draws as shapes has one object and
        // two lines: down the plain path the buffer's later lines would be
        // written as *new* lines below the first, extra text the page never
        // had. A single frozen line with one text piece next to its outlined
        // word is the same trap in one line — the plain path would rewrite it.
        // Either goes to the paragraph path, which knows to leave a frozen
        // line alone.
        let objects_on_page: usize = edit.lines.iter().map(|(objects, _)| objects.len()).sum();
        if edit.lines.len() > 1 || objects_on_page > 1 || edit.frozen.iter().any(|frozen| *frozen) {
            // The buffer as it was typed, **not** `typed`: that is trimmed as
            // a whole, which takes a leading or trailing empty line with it —
            // and an empty first or last line is exactly what a frozen line
            // with no text object is. Every other line would then be matched
            // against the wrong original. Each line is compared trimmed
            // inside, so nothing is lost by leaving the ends alone.
            self.apply_paragraph_edit(&edit, &edit.buffer);
            return;
        }

        // **A single run can grow a second line of its own, the same way a
        // paragraph already could.** Enter adds a line here now instead of
        // submitting (see `draw_run_editor`), so the buffer this reads may
        // hold more than the one line the run itself is. Only the first
        // replaces the run in place; anything after it is new content,
        // written just below in the run's own size, colour and font — not
        // sent to `SetTextRun`, which is one run's own text, not several.
        let mut typed_lines = typed.split('\n');
        let first_line = typed_lines.next().unwrap_or("").to_string();
        let extra_lines: Vec<&str> = typed_lines.collect();

        let Some(doc) = &self.tab_mut().doc else { return };
        match doc.session.execute(pdf_core::command::Command::SetTextRun {
            page_index: edit.page,
            object: edit.object,
            text: first_line,
            // **Nothing asked for, when nothing was changed.**
            //
            // The style here is *seeded* from the run so the controls open
            // showing what is there — a UI convenience. Sending it as an
            // instruction made every edit an instruction to restyle, and the
            // engine takes its safe path (swap the characters in the stream,
            // touch nothing else) only when the style asks for nothing. So the
            // safe path was unreachable, and every word-only edit went through
            // the one that re-emits the page.
            //
            // Reported from use twice, with a screenshot: one line of a
            // paragraph drawn over the line above it in a different font, its
            // own place left empty. This is why. A requested font rides
            // along regardless — it is not one of the three properties that
            // gate the slow path, see `changed_look` above.
            //
            // **Sent field by field against `edit.was`, not as the whole
            // struct.** `edit.style` is seeded with the run's *current*
            // size, colour and position from the moment the editor opens, so
            // all three already read `Some` before anyone touches anything —
            // sending it whole made changing only the size also assert an
            // unchanged colour and position right back at their own values,
            // which is indistinguishable from asking to change all three and
            // forces PDFium's page-wide rewrite for a plain size increase.
            // Reported from use as "i can't increase the font sizes": the
            // size itself has its own byte-safe path in `pdf_core` (see
            // `set_run_in_stream`'s `new_size`) that this was never reaching.
            style: {
                let mut style = if changed_look {
                    pdf_core::document::TextStyle {
                        size: (edit.style.size != edit.was.size).then_some(edit.style.size).flatten(),
                        color: (edit.style.color != edit.was.color).then_some(edit.style.color).flatten(),
                        at: (edit.style.at != edit.was.at).then_some(edit.style.at).flatten(),
                        face: None,
                    }
                } else {
                    pdf_core::document::TextStyle::default()
                };
                style.face = edit.style.face.clone();
                style
            },
        }) {
            Ok(_) => {
                // Said when it happened, not discovered on a printed page: the
                // document's own font could not spell these words, so they are
                // in a face that is not their neighbours'.
                let face = self.tab_mut()
                    .doc
                    .as_ref()
                    .and_then(|d| d.session.substituted_face());
                if let Some(doc) = &mut self.tab_mut().doc {
                    doc.rendered_is_stale();
                }
                self.tab_mut().text_selection = None;
                self.tab_mut().find_hits.clear();

                // A pick made through the font button is not a surprise
                // fallback — say it as the choice it was, not as the
                // "does not match its neighbours" warning an *automatic*
                // substitution earns. Checked first: `substituted_face()`
                // reports a face either way, since both go through the same
                // swap machinery underneath.
                match (&edit.style.face, face) {
                    (Some(picked), _) => self.say_info(format!(
                        "changed to \"{typed}\", written in {picked}. `undo` puts it back."
                    )),
                    (None, Some(face)) => self.say_info(format!(
                        "changed to \"{typed}\", written in {face} — the document's own \
                         font here has only the letters it already uses, so this will \
                         not match its neighbours. `undo` puts it back."
                    )),
                    (None, None) => {
                        self.say_info(format!("changed to \"{typed}\" — `undo` puts it back."))
                    }
                }

                if !extra_lines.is_empty() {
                    let gap = (edit.rect.bottom - edit.rect.top).max(12.0);
                    let base_x = edit.style.at.map(|(x, _)| x).unwrap_or(edit.rect.left);
                    let face = self.registered_face_of(&edit);
                    if let Err(e) = self.write_extra_styled_lines(
                        edit.page,
                        base_x,
                        edit.rect.bottom,
                        gap,
                        &edit.style,
                        face.as_deref(),
                        &extra_lines,
                    ) {
                        self.say_error(format!("the new line could not be added: {e}"));
                    }
                }
            }
            Err(e) => self.say_error(explain(&e)),
        }
    }

    /// Apply a paragraph's typed text to the lines it came from: **a retyped
    /// line replaces its pieces.** The line's first object takes the new
    /// words and its other pieces are *removed from the page*; a line the
    /// person deleted has every piece removed; a line that still says what it
    /// said is left alone, every piece of it where it is. See the
    /// `paragraph_lines` module's own doc for why this used to hide the other
    /// pieces by colour and why that was wrong (white glyphs drawn after the new
    /// words erased part of them, the old words stayed in the file, a later pick
    /// found them again, and undo re-typed the old words instead of restoring
    /// them).
    ///
    /// What is executed is [`Self::paragraph_edit_commands`]' own output —
    /// one `Command::ReplaceTextLines`, with the paragraph's new colour first if
    /// one was asked for — as a single command, or a `Command::Batch` of the two,
    /// so **one `undo` puts the whole paragraph back, exactly**: the replace undoes
    /// by a page snapshot, then the colour. Before the batch each line was its own
    /// `execute()` and undoing one paragraph took several presses.
    ///
    /// **The replace renumbers the page's objects.** Afterwards every object id
    /// kept from before is stale, so what holds one is dealt with here: the
    /// page's caches (`rendered_is_stale`, `layers`, the extracted text) and the
    /// joins a person declared on this page, which name objects by id and are
    /// forgotten when pieces came off — a join left pointing at other objects
    /// would open the wrong paragraph.
    ///
    /// Lines typed past the paragraph's own end go below it by position, **after**
    /// the replace has run and only if it was not refused.
    ///
    /// **What it says afterwards** is [`paragraph_applied_message`]:
    /// "paragraph changed.", or "unchanged." when every line says what it said,
    /// with "; N line(s) drawn as shapes were left as they are" added when
    /// words were typed over frozen lines ([`EditingRun::frozen`]), which are
    /// never written or removed, and a note when a new position was asked for
    /// (not applied). A paragraph that no longer lines up with its own text is
    /// refused with an error before anything is built; an engine refusal (a
    /// piece that cannot be removed without moving its neighbour, say) leaves
    /// the page as it was and is said as it came.
    ///
    /// `typed` is the buffer **as typed**, not trimmed as a whole — see
    /// `apply_one_edit`'s own comment at the call.
    pub(crate) fn apply_paragraph_edit(&mut self, edit: &EditingRun, typed: &str) {
        // **Timed and logged in two pieces, not one.** The app's own call
        // into pdf_core (`doc.session.execute`) is one opaque function from
        // here — if the overall apply time reported by `apply_editing_page`
        // keeps not matching what a fresh, single-edit test measures,
        // knowing whether the time is spent *building* the command list
        // (this file: `paragraph_edit_commands`) or *executing* it (pdf_core:
        // the actual PDFium write) is the next thing needed to tell which
        // side of that boundary to keep looking on, without having to
        // round-trip a build for every guess.
        let t_build = std::time::Instant::now();
        let planned = match self.paragraph_edit_commands(edit, typed) {
            Ok(planned) => planned,
            Err(e) => {
                // Refused before a single command was built: the page is as it was.
                self.say_error(e);
                return;
            }
        };
        let build_time = t_build.elapsed();
        let ParagraphCommands { commands, frozen_changed, position_ignored, surplus } = planned;

        let line_count = edit.lines.len();
        let nothing_to_execute = commands.is_empty();
        let renumbers = commands.iter().any(|command| match command {
            pdf_core::command::Command::ReplaceTextLines { edits, .. } => removed_pieces(edits) > 0,
            _ => false,
        });
        let asked_for_width = asks_for_width(&commands);
        // One command is executed as itself; a colour and then the replace as
        // one batch, which undoes in reverse as one step.
        let one_command = |mut commands: Vec<pdf_core::command::Command>| match commands.len() {
            0 => None,
            1 => commands.pop(),
            _ => Some(pdf_core::command::Command::Batch { commands }),
        };
        let command = one_command(commands);
        let t_exec = std::time::Instant::now();
        let mut failed = match (command, &self.tab_mut().doc) {
            (Some(command), Some(doc)) => doc.session.execute(command).err().map(|e| explain(&e)),
            _ => None,
        };
        // **A line the engine could not stretch to its old width.** It refuses
        // the whole replace, atomically, and does not say which line asked too
        // much (a one-letter line, a gap that would have to close or open too far,
        // codes it cannot count): the only thing to try is the same edit without
        // asking for the width, and to say that the lines came out ragged.
        let mut ragged = false;
        if failed.is_some() && asked_for_width {
            if let Ok(again) = self.paragraph_edit_commands_with(edit, typed, false) {
                if let (Some(retry), Some(doc)) = (one_command(again.commands), &self.tab_mut().doc) {
                    failed = doc.session.execute(retry).err().map(|e| explain(&e));
                    ragged = failed.is_none();
                }
            }
        }
        let exec_time = t_exec.elapsed();
        let inner = self.tab_mut().doc.as_ref().map(|doc| doc.session.take_last_batch_timing());
        self.session_log.record(
            "info",
            &format!(
                "paragraph apply breakdown: {line_count} lines, build {build_time:?}, execute {exec_time:?}"
            ),
        );
        if let Some(inner) = inner {
            for (name, d) in inner {
                self.session_log.record("info", &format!("  pdf_core: {name} {d:?}"));
            }
        }

        // New lines go below the paragraph by position — geometric, so the
        // renumbering does not matter to them — and not at all after a refusal.
        let mut surplus_written = 0;
        if let (None, Some(surplus)) = (&failed, &surplus) {
            surplus_written = surplus.written();
            let lines: Vec<&str> = surplus.lines.iter().map(String::as_str).collect();
            if let Err(e) = self.write_extra_styled_lines(
                edit.page,
                surplus.base_x,
                surplus.below,
                surplus.gap,
                &edit.style,
                surplus.face.as_deref(),
                &lines,
            ) {
                self.say_error(format!("a new line could not be added: {e}"));
            }
        }

        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        self.tab_mut().text_selection = None;
        self.tab_mut().find_hits.clear();
        if failed.is_none() && renumbers {
            // Every object number of the page moved. `layers` is a list read
            // before it; a join names objects by number and would now name
            // other ones.
            let page = edit.page;
            self.tab_mut().joined_groups.retain(|group| group.page != page);
        }

        match failed {
            // Said as the engine said it, and what it leaves unsaid: the replace
            // is atomic — and a colour written ahead of it is taken back with it —
            // so a refusal changes nothing.
            Some(e) => self.say_error(format!("{e} — nothing was changed.")),
            None => {
                let mut said = paragraph_applied_message(
                    nothing_to_execute && surplus_written == 0,
                    frozen_changed,
                    position_ignored,
                );
                if typed.trim().is_empty() {
                    said = said.replacen("paragraph changed", "deleted — `undo` puts it back", 1);
                }
                // Said when it happened, as a single run's edit already does: a
                // font that had no outline for what was typed is not the
                // words' neighbours' font, and would otherwise be found on a
                // printed page.
                let swapped = if nothing_to_execute {
                    None
                } else {
                    self.tab_mut().doc.as_ref().and_then(|d| d.session.substituted_face())
                };
                if let Some(face) = swapped {
                    said = format!(
                        "{said} Written in {face} — the document's own font here has only the \
                         letters it already uses, so this will not match its neighbours."
                    );
                }
                self.say_info(if ragged {
                    format!("{said} The retyped lines could not be stretched to the width they had, so they end where the new words end.")
                } else {
                    said
                });
            }
        }
    }

    /// Replace a word that is *drawn* rather than written.
    ///
    /// # The page this exists for
    ///
    /// Type converted to outlines: the words are Bézier paths, indistinguishable
    /// from any other artwork, and there is no text on the page to edit.
    /// `extracttext` recognises them and lays a transparent text layer over the
    /// top so the page can be searched and copied from — but that layer is not
    /// the page. Editing it changed what could be found and left what could be
    /// *seen* exactly as it was, which is what "I deleted it and the text below
    /// still shows" means.
    ///
    /// So this does the two things that actually change the page: it takes the
    /// paths off, and it writes words in their place.
    ///
    /// **Removal first, and it must be complete.** `allow_incomplete` is false
    /// on purpose — a word half taken off, with new text written over the
    /// remains, is worse than a refusal, and a refusal costs nothing because
    /// the removal is what would have happened first anyway.
    pub(crate) fn replace_outlined_word(
        &mut self,
        page: usize,
        area: pdf_core::document::Rect,
        origin: (f32, f32),
        size: f32,
        was: &str,
        text: &str,
    ) -> Result<String, String> {
        use pdf_core::document::{Annotation, Color, Glyph};

        let faces = self.outlined_font_bytes();
        if faces.is_empty() {
            return Err(
                "these words are artwork, and matching them needs a font — \
                 `outlinedfont add <file.ttf>` supplies one."
                    .into(),
            );
        }
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
        let borrowed: Vec<&[u8]> = faces.iter().map(Vec::as_slice).collect();

        // **A drawn word is often part of something bigger.**
        //
        // A heading converted to outlines is frequently one path holding every
        // letter of it — reported from use as "object 18 is text drawn as
        // curves, which this pass cannot remove". A rectangle around one word
        // merely *crosses* that path, and a crossed path stays; a contained one
        // comes off whatever its shape. So: ask what is in the way, and widen
        // to contain it.
        let mut area = area;
        let mut widened = false;
        if let Ok(report) = doc.session.preview_redaction(page, area, &borrowed) {
            for blocker in &report.uncleared {
                let pdf_core::document::Uncleared::OutlinedText { object } = blocker else {
                    continue;
                };
                let Ok(bounds) = doc.session.object_bounds(page, *object) else { continue };
                area = pdf_core::document::Rect {
                    left: area.left.min(bounds.left),
                    top: area.top.min(bounds.top),
                    right: area.right.max(bounds.right),
                    bottom: area.bottom.max(bounds.bottom),
                };
                widened = true;
            }
        }

        // But not without limit. Some pages draw everything on them as one
        // path, and widening to contain *that* would take the page with it.
        if widened {
            let size = doc.session.page_size(page).map_err(|e| e.to_string())?;
            let share = ((area.right - area.left) * (area.bottom - area.top)).abs()
                / (size.width_pt * size.height_pt).max(1.0);
            if share > 0.4 {
                return Err(
                    "these words are part of one drawn shape covering most of the page, \
                     so replacing them would mean replacing all of it. Nothing was changed."
                        .into(),
                );
            }
        }

        // Off the page, leaving the space blank rather than the black bar a
        // redaction paints: this is an edit, not a redaction, and a mark would
        // be a second answer to a question nobody asked.
        doc.session
            .execute(pdf_core::command::Command::Redact {
                page_index: page,
                area,
                fill: None,
                allow_incomplete: false,
                outlined_fonts: faces,
            })
            .map_err(|e| {
                format!("the drawn words could not be taken off the page, so nothing was changed — {e}")
            })?;

        // **The document's own face, where it can draw these letters.**
        //
        // The words being replaced were set in it — that is how they were
        // recognised at all — so writing the new ones in anything else leaves a
        // patch that reads as a repair. Falls back to Helvetica only where no
        // face offered can spell what was typed, and says which was used.
        let mut size = size.max(1.0);
        let mut baseline = origin.1;
        let mut pen_start = origin.0;
        let (face, font_asset, glyphs) = match self
            .writing_faces()
            .into_iter()
            .find(|name| pdf_core::text::covers(name, text))
        {
            Some(name) => {
                // **The size the box says, not the size the recogniser guessed.**
                //
                // Measured: replacing a word with *itself* came back a quarter
                // too small and sitting left of where it had been. The word
                // that was there occupies a known width, and the same word set
                // in the same face has a known width per point — so the ratio
                // is the point size, and it is a measurement rather than an
                // estimate. Only used where it is sane: a recogniser that read
                // the word wrongly must not resize the replacement to match its
                // mistake.
                let drawn = area.right - area.left;
                if let Ok(original) = pdf_core::text::shape(&name, was.trim()) {
                    let per_point = original.width();
                    if per_point > 0.01 && drawn > 0.5 {
                        let measured = drawn / per_point;
                        if measured > size * 0.4 && measured < size * 3.0 {
                            size = measured;
                        }
                    }
                }

                // **And the baseline the box says, for the same reason.**
                //
                // Measured: the recogniser's baseline put the replacement three
                // points above the line the word had been sitting on. The
                // bottom of a word's ink *is* its baseline unless something in
                // it descends, and how far the original word descended is a
                // fact about its letters — see `text::ink_depth`.
                if let Some(depth) = pdf_core::text::ink_depth(&name, was.trim()) {
                    baseline = area.bottom - depth * size;
                }

                // And the same horizontally: the box's left is where the ink
                // starts, not where the pen was. Every glyph has a left side
                // bearing, and starting the pen at the edge pushes the word
                // right by it — measured at two points on a real page.
                if let Some(bearing) = pdf_core::text::ink_start(&name, text) {
                    pen_start = area.left - bearing * size;
                }
                let shaped = pdf_core::text::shape(&name, text)
                    .map_err(|e| format!("{name} could not set those words — {e}"))?;
                // Where each glyph goes: the pen starts on the baseline and
                // moves by the font's own advances, which is the whole reason
                // for shaping rather than spacing by hand.
                let mut boundaries: Vec<usize> =
                    shaped.glyphs.iter().map(|g| g.cluster as usize).collect();
                boundaries.sort_unstable();
                boundaries.dedup();

                let mut pen = pen_start;
                let mut placed = Vec::with_capacity(shaped.glyphs.len());
                for glyph in &shaped.glyphs {
                    let from = (glyph.cluster as usize).min(text.len());
                    let to = boundaries
                        .iter()
                        .find(|&&b| b > from)
                        .copied()
                        .unwrap_or(text.len())
                        .min(text.len());
                    placed.push(Glyph {
                        // What this glyph stands for, so the words are
                        // searchable afterwards — a joined form has no
                        // character of its own to fall back on.
                        ch: text.get(from..to).unwrap_or_default().to_string(),
                        id: glyph.id,
                        x: pen + glyph.offset_x * size,
                        // Page space counts downwards; a glyph that hangs above
                        // the pen has a positive offset.
                        y: baseline - glyph.offset_y * size,
                        radians: 0.0,
                    });
                    pen += glyph.advance * size;
                }
                (name.clone(), Some(name), placed)
            }
            None => (
                "Helvetica".to_string(),
                None,
                vec![Glyph {
                    ch: text.to_string(),
                    id: 0,
                    x: origin.0,
                    y: baseline,
                    radians: 0.0,
                }],
            ),
        };

        let id = self.tab_mut().next_text_id;
        self.tab_mut().next_text_id += 1;
        let Some(doc) = &self.tab_mut().doc else { return Err("nothing open.".into()) };
        doc.session
            .execute(pdf_core::command::Command::AddAnnotation {
                page_index: page,
                annotation: Annotation::Text {
                    text: text.to_string(),
                    font: if font_asset.is_some() { face.clone() } else { "Helvetica".into() },
                    font_asset: font_asset.clone(),
                    // The size and baseline the recognised word had, so the new
                    // words sit on the line the old ones sat on.
                    size,
                    color: Color { r: 20, g: 20, b: 20, a: 255 },
                    glyphs,
                    id,
                    restore: String::new(),
                    frame: Vec::new(),
                    frame_width: 0.0,
                },
            })
            .map_err(|e| format!("the words came off but the new ones would not go on — {e}"))?;

        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
        }
        self.tab_mut().text_selection = None;
        self.tab_mut().find_hits.clear();
        Ok(format!(
            "replaced the drawn word with \"{text}\" on page {}, set in {face}.{} It is \
             real text now — and this took two steps, so `undo` twice puts the \
             artwork back.",
            page + 1,
            if widened {
                " The letters around it were part of the same drawn shape and came off \
                 with it."
            } else {
                ""
            }
        ))
    }
}
