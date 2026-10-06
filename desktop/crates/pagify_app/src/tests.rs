use super::*;

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn the_conversion_to_egui_keeps_the_page_the_right_way_up() {
    let session = Session::open(fixture("quadrants.pdf")).expect("open");
    let raster = session.render_page(0, 1.0).expect("render");
    let image = page_to_image(&raster);

    assert_eq!(image.width(), 400);
    assert_eq!(image.height(), 400);

    let (near, far) = (100, 300);
    let top_left = image[(near, near)];
    let top_right = image[(far, near)];
    let bottom_left = image[(near, far)];

    assert!(top_left.r() > top_left.b(), "top-left should be red, got {top_left:?}");
    assert!(top_right.g() > top_right.r(), "top-right should be green");
    assert!(bottom_left.b() > bottom_left.r(), "bottom-left should be blue");
}

/// **Reported from use: undoing an added text box turned a whole real
/// page grey.** `remove_text` — what undoing `Annotation::Text` calls —
/// used to take the marked object off with `FPDFPage_RemoveObject` and
/// then call `FPDFPage_GenerateContent` to rebuild the stream around
/// the gap. That rebuild does not carry every colour space back through
/// unchanged: the real page it was reported on draws most of its own
/// diagram in vector shapes with a colour space `FPDFPage_
/// GenerateContent` does not, confirmed only against that real file —
/// **`quadrants.pdf`'s own plain `rg`-filled squares survive the old,
/// broken implementation just as well as the fix**, so this test alone
/// cannot catch a regression back to it; it only guards the ordinary
/// case (a simple fill surviving a text removal at all) while the real
/// defect stays unverified by anything portable. See the fix's own doc,
/// on `remove_text`, for the honest state of this.
#[test]
fn undoing_an_added_text_box_does_not_disturb_a_simple_coloured_fill() {
    use pdf_core::document::{Annotation, Glyph};

    let session = Session::open(fixture("quadrants.pdf")).expect("open");
    session
        .execute(pdf_core::command::Command::AddAnnotation {
            page_index: 0,
            annotation: Annotation::Text {
                text: "hello".to_string(),
                font: "Helvetica".to_string(),
                font_asset: None,
                size: 12.0,
                color: pdf_core::document::Color { r: 0, g: 0, b: 0, a: 255 },
                glyphs: vec![Glyph { ch: "hello".to_string(), id: 0, x: 10.0, y: 20.0, radians: 0.0 }],
                id: 1,
                restore: String::new(),
                frame: Vec::new(),
                frame_width: 0.0,
            },
        })
        .expect("add text");

    let (undone, _) = session.undo().expect("undo");
    assert!(undone, "there should have been something to undo");

    let raster = session.render_page(0, 1.0).expect("render");
    let image = page_to_image(&raster);
    let (near, far) = (100, 300);
    let top_left = image[(near, near)];
    let top_right = image[(far, near)];
    let bottom_left = image[(near, far)];
    assert!(top_left.r() > top_left.b(), "top-left should still be red, got {top_left:?}");
    assert!(top_right.g() > top_right.r(), "top-right should still be green, got {top_right:?}");
    assert!(bottom_left.b() > bottom_left.r(), "bottom-left should still be blue, got {bottom_left:?}");
}

#[test]
fn a_page_uploads_as_a_texture() {
    let session = Session::open(fixture("text-lines.pdf")).expect("open");
    let raster = session.render_page(0, 2.0).expect("render");
    let ctx = egui::Context::default();
    let handle = ctx.load_texture("page", page_to_image(&raster), egui::TextureOptions::LINEAR);
    assert_eq!(handle.size(), [raster.width as usize, raster.height as usize]);
}

#[test]
fn a_rotated_page_renders_at_the_swapped_size() {
    let session = Session::open(fixture("mixed-sizes.pdf")).expect("open");
    let upright = session.render_page(0, 1.0).expect("render");
    let turned = session
        .render_page_rotated(0, 1.0, Rotation::Clockwise90)
        .expect("render rotated");
    assert_eq!((turned.width, turned.height), (upright.height, upright.width));
    let _ = page_to_image(&turned);
}

/// The whole tool chain, driven the way a script would drive it.
///
/// These exist because the tools were broken in a way no shell test could
/// see: the geometry functions were all correct and individually tested,
/// and the *app* was choosing the wrong one — deciding the operation from
/// the prompt text, so Fillet performed a Move. The defect lived entirely
/// in the glue between command and operation, which is exactly what these
/// exercise.
mod tools_chain {
    use super::*;
    use cad_kernel::Geom;

    fn app() -> PagifyApp {
        let app = PagifyApp::new(Some(&fixture("pages-ladder.pdf")));
        assert!(app.tab().doc.is_some(), "the fixture did not open");
        app
    }

    fn marks(app: &PagifyApp) -> &[cad_kernel::DObject] {
        app.tab().markup.existing(0).map(|l| l.objects()).unwrap_or(&[])
    }

    fn errors(app: &PagifyApp) -> Vec<String> {
        app.cmd
            .history()
            .iter()
            .filter(|e| e.kind == Kind::Error)
            .map(|e| e.text.clone())
            .collect()
    }

    fn infos(app: &PagifyApp) -> Vec<String> {
        app.cmd
            .history()
            .iter()
            .filter(|e| e.kind == Kind::Info)
            .map(|e| e.text.clone())
            .collect()
    }

    /// **The acceptance test for the whole thing: nothing of the ink is
    /// left to draw.**
    ///
    /// The two tests below check the mechanism — the layer stops painting,
    /// and new marks are refused. This checks the *outcome* the person
    /// actually sees: draw, lock, then rasterise the page at the size the
    /// thumbnail strip draws it and count pixels of the ink's own colour.
    ///
    /// Asserted on the colour rather than on how much of the page is dark,
    /// because a locked page is blank either way — "less ink" would pass
    /// whether or not the stroke went.
    #[test]
    fn a_locked_page_has_no_trace_of_the_ink_drawn_on_it() {
        /// Pixels of the markup ink's own colour, at thumbnail size.
        fn ink_pixels(app: &PagifyApp) -> usize {
            let doc = app.tab().doc.as_ref().expect("doc");
            let (width_pt, _) = doc.strip.size_of(0).expect("size");
            // 140px wide, which is what the strip draws.
            let raster = doc
                .session
                .render_page(0, 140.0 / width_pt)
                .expect("render the page");
            let (r, g, b) = (MARKUP_INK.r, MARKUP_INK.g, MARKUP_INK.b);
            raster
                .pixels
                .chunks_exact(4)
                .filter(|p| {
                    // Near the ink colour, allowing for antialiasing.
                    (p[0] as i16 - r as i16).abs() < 60
                        && (p[1] as i16 - g as i16).abs() < 60
                        && (p[2] as i16 - b as i16).abs() < 60
                })
                .count()
        }

        let mut app = app();
        app.submit("l 100,100 300,300");
        // Into the document, but **not** to disk: `save(None)` writes over
        // the fixture, which every other test in this module then reads.
        app.commit_marks_on(&[0]).expect("commit the ink");
        let drawn = ink_pixels(&app);
        assert!(drawn > 0, "the ink never reached the page, so this proves nothing");

        app.lock_pages(&[0], b"a good passcode").expect("lock");
        let left = ink_pixels(&app);
        println!("thumbnail-sized render: {drawn} ink pixels drawn, {left} after locking");
        assert_eq!(left, 0, "the locked page still draws {drawn} pixels of ink");
    }

    /// **Ink drawn before locking is sealed with the page, not left on top
    /// of it.**
    ///
    /// Reported from use: a locked page still showed pen strokes. Drawn
    /// marks live in this layer until the document is saved, so locking —
    /// which seals the page *as the document has it* — sealed a page that
    /// did not include them, and the layer went on painting them over the
    /// blank result.
    #[test]
    fn locking_a_page_takes_the_ink_drawn_on_it() {
        let mut app = app();
        app.submit("l 100,100 300,300");
        assert!(!marks(&app).is_empty(), "nothing was drawn, so this proves nothing");

        app.lock_pages(&[0], b"a good passcode").expect("lock");

        assert!(
            marks(&app).is_empty(),
            "the drawn ink is still being painted over the locked page"
        );
    }

    /// And nothing new can be drawn on it afterwards — said at the moment
    /// of drawing, not when the document is eventually saved.
    #[test]
    fn a_locked_page_refuses_to_be_drawn_on() {
        let mut app = app();
        app.lock_pages(&[0], b"a good passcode").expect("lock");

        app.submit("l 100,100 300,300");
        assert!(
            errors(&app).iter().any(|e| e.contains("locked")),
            "it did not say why: {:?}",
            errors(&app)
        );
        assert!(marks(&app).is_empty(), "it drew on a locked page anyway");
    }

    #[test]
    fn typed_coordinates_draw() {
        let mut app = app();
        app.submit("l 30,250 170,250");
        app.submit("ci 100,400 40");
        assert_eq!(marks(&app).len(), 2, "errors: {:?}", errors(&app));
    }

    #[test]
    fn a_tool_plus_typed_picks_draws() {
        // `line` arms the tool; the picks are the clicks.
        let mut app = app();
        app.submit("line");
        app.submit("pick 20,20");
        app.submit("pick 120,120");
        assert_eq!(marks(&app).len(), 1, "errors: {:?}", errors(&app));
        assert!(matches!(marks(&app)[0].geom, Geom::Line(_)));
    }

    /// `fill` says which state it left, so the ribbon button and the box
    /// agree about what just happened.
    #[test]
    fn the_fill_command_toggles_and_says_so() {
        let mut app = app();
        app.submit("fill");
        assert!(app.draw_fill);
        assert!(infos(&app).iter().any(|s| s.contains("fill: on")), "{:?}", infos(&app));

        app.submit("fill");
        assert!(!app.draw_fill);
        assert!(infos(&app).iter().any(|s| s.contains("fill: off")), "{:?}", infos(&app));
    }

    /// A rectangle drawn with fill off — the default — is hollow: its
    /// outline, and nothing else.
    #[test]
    fn a_rectangle_drawn_without_fill_is_hollow() {
        let mut app = app();
        app.submit("rectangle");
        app.submit("pick 100,100");
        app.submit("pick 200,180");
        assert_eq!(marks(&app).len(), 1, "errors: {:?}", errors(&app));
        assert!(matches!(marks(&app)[0].geom, Geom::Polyline(_)));
    }

    /// Turning `fill` on before drawing a rectangle pairs it with a solid
    /// `Geom::Hatch` pointed at its own handle — the live representation a
    /// filled shape carries per [`fill_boundary`].
    #[test]
    fn a_rectangle_drawn_with_fill_on_carries_a_hatch() {
        let mut app = app();
        app.submit("fill");
        app.submit("rectangle");
        app.submit("pick 100,100");
        app.submit("pick 200,180");
        assert_eq!(marks(&app).len(), 2, "errors: {:?}", errors(&app));

        let boundary = marks(&app).iter().find(|o| matches!(o.geom, Geom::Polyline(_)))
            .expect("the rectangle itself");
        let hatch = marks(&app).iter().find_map(|o| match &o.geom {
            Geom::Hatch(h) => Some(h),
            _ => None,
        }).expect("a hatch pairing the fill");
        assert_eq!(hatch.boundary_handles, vec![boundary.handle]);
    }

    /// The same, for a filled circle — the other shape the user chose.
    #[test]
    fn a_circle_drawn_with_fill_on_carries_a_hatch() {
        let mut app = app();
        app.submit("fill");
        app.submit("circle");
        app.submit("pick 100,400");
        app.submit("pick 140,400");
        assert_eq!(marks(&app).len(), 2, "errors: {:?}", errors(&app));
        assert!(marks(&app).iter().any(|o| matches!(o.geom, Geom::Hatch(_))));
    }

    /// **The end of the chain: a filled rectangle actually reaches the
    /// document as a filled mark, not only as a paired object in the live
    /// layer.** Every layer above this one — `to_blob`, `commit_page`,
    /// PDFium's path API — could each be individually right and the
    /// feature still not work; this is the one test that would catch that.
    #[test]
    fn a_filled_rectangle_commits_as_a_real_fill_annotation() {
        let mut app = app();
        app.submit("fill");
        app.submit("rectangle");
        app.submit("pick 100,100");
        app.submit("pick 200,180");
        assert_eq!(marks(&app).len(), 2, "errors: {:?}", errors(&app));

        app.commit_marks_on(&[0]).expect("commit the markup");

        let annotations = app.tab_mut().doc.as_ref().expect("doc").session.annotations(0).expect("read them back");
        let fills = annotations
            .iter()
            .filter(|a| matches!(a.annotation, pdf_core::document::Annotation::Fill { .. }))
            .count();
        let inks = annotations
            .iter()
            .filter(|a| matches!(a.annotation, pdf_core::document::Annotation::Ink { .. }))
            .count();
        assert_eq!(fills, 1, "the fill did not reach the document: {annotations:?}");
        assert!(inks >= 1, "the rectangle's own outline should still be there too: {annotations:?}");
    }

    /// **The same end-to-end proof, for a shape's own colour**: setting
    /// one through `Layer::set_color` has to survive `commit_page` and
    /// come back as the ink's own `/C`, not the page's shared pen.
    #[test]
    fn a_shape_given_its_own_colour_commits_with_it_not_the_pages_pen() {
        let mut app = app();
        app.submit("circle");
        app.submit("pick 100,400");
        app.submit("pick 140,400");
        assert_eq!(marks(&app).len(), 1, "errors: {:?}", errors(&app));

        assert!(app.tab_mut().markup.page(0, 792.0).set_color(0, (10, 200, 90)));

        app.commit_marks_on(&[0]).expect("commit the markup");

        let annotations = app.tab_mut().doc.as_ref().expect("doc").session.annotations(0).expect("read them back");
        let colour = annotations
            .iter()
            .find_map(|a| match &a.annotation {
                pdf_core::document::Annotation::Ink { color, .. } => Some(*color),
                _ => None,
            })
            .expect("the circle's ink annotation");
        assert_eq!(
            (colour.r, colour.g, colour.b),
            (10, 200, 90),
            "it committed with the page's pen instead of its own colour: {colour:?}"
        );
    }

    #[test]
    fn a_polyline_collects_until_it_is_resolved() {
        let mut app = app();
        app.submit("pline");
        for p in ["pick 10,10", "pick 60,10", "pick 60,60"] {
            app.submit(p);
        }
        assert!(marks(&app).is_empty(), "a polyline must not commit early");
        app.resolve();
        match &marks(&app)[0].geom {
            Geom::Polyline(pl) => assert_eq!(pl.vertices.len(), 3),
            other => panic!("{other:?}"),
        }
    }

    /// The reported bug, end to end.
    #[test]
    fn fillet_fillets_rather_than_moving() {
        let mut app = app();
        app.submit("l 30,250 170,250");
        app.submit("l 170,250 170,350");
        let before: Vec<_> = marks(&app).iter().map(|o| o.bbox()).collect();

        app.submit("fillet 25");
        app.submit("pick 100,250");
        app.submit("pick 170,320");

        assert!(
            marks(&app).iter().any(|o| matches!(o.geom, Geom::Arc(_))),
            "no arc — fillet did not fillet. errors: {:?}",
            errors(&app)
        );
        assert_ne!(
            marks(&app).len(),
            before.len(),
            "nothing was added; a move would have left the count alone"
        );
    }

    #[test]
    fn trim_cuts_and_stays_armed_for_the_next_piece() {
        let mut app = app();
        app.submit("l 0,250 200,250");
        app.submit("l 100,200 100,300");
        app.submit("trim");
        app.submit("pick 180,250");

        assert!(errors(&app).is_empty(), "errors: {:?}", errors(&app));
        assert!(
            app.tab_mut().pending.is_some(),
            "trim is a repeating tool and must still be armed"
        );
    }

    #[test]
    fn offset_takes_an_object_and_then_a_side() {
        let mut app = app();
        app.submit("l 30,250 170,250");
        app.submit("offset 15");
        app.submit("pick 100,250"); // the object
        app.submit("pick 100,300"); // which side

        assert_eq!(marks(&app).len(), 2, "errors: {:?}", errors(&app));
    }

    #[test]
    fn move_needs_a_selection_and_says_so() {
        let mut app = app();
        app.submit("l 30,250 170,250");
        app.submit("move");
        assert!(app.tab_mut().pending.is_none(), "move armed with nothing selected");
        assert!(
            errors(&app).iter().any(|e| e.contains("selected")),
            "no explanation: {:?}",
            errors(&app)
        );
    }

    #[test]
    fn move_moves_once_something_is_selected() {
        let mut app = app();
        app.submit("l 30,250 170,250");
        app.submit("all");
        app.submit("move");
        app.submit("pick 0,0");
        app.submit("pick 0,100");

        match &marks(&app)[0].geom {
            Geom::Line(l) => assert!((l.a.y - 350.0).abs() < 1e-6, "moved to {}", l.a.y),
            other => panic!("{other:?}"),
        }
    }

    /// **Reported from use: a drawn shape could only be moved by typing
    /// `move` and clicking twice — dragging it did nothing.**
    /// `finish_markup_grab` is what a real drag now commits through
    /// instead; this exercises it the same direct way
    /// `dragging_a_selected_signature_moves_it` exercises
    /// `finish_signature_grab`.
    #[test]
    fn dragging_a_selected_markup_shape_moves_it() {
        let mut app = app();
        app.submit("l 30,250 170,250");
        app.submit("all");

        let grab = Grab { handle: None, from: AppPoint { x: 0.0, y: 0.0 }, by: (40.0, -25.0) };
        app.finish_markup_grab(0, grab);

        match &marks(&app)[0].geom {
            Geom::Line(l) => {
                // App space counts downwards; kernel space upwards.
                assert!((l.a.x - 70.0).abs() < 1e-6, "x did not move: {}", l.a.x);
                assert!((l.a.y - 275.0).abs() < 1e-6, "y did not move: {}", l.a.y);
            }
            other => panic!("{other:?}"),
        }
    }

    /// A drag too small to mean anything commits nothing — the same
    /// noise floor [`PagifyApp::finish_grab`] and
    /// [`PagifyApp::finish_signature_grab`] apply.
    #[test]
    fn a_negligible_drag_on_a_markup_shape_changes_nothing() {
        let mut app = app();
        app.submit("l 30,250 170,250");
        app.submit("all");

        let grab = Grab { handle: None, from: AppPoint { x: 0.0, y: 0.0 }, by: (0.4, 0.2) };
        app.finish_markup_grab(0, grab);

        match &marks(&app)[0].geom {
            Geom::Line(l) => {
                assert!((l.a.x - 30.0).abs() < 1e-6, "a negligible drag moved it: {}", l.a.x);
                assert!((l.a.y - 250.0).abs() < 1e-6, "a negligible drag moved it: {}", l.a.y);
            }
            other => panic!("{other:?}"),
        }
    }

    /// **Companion ask, arriving mid-session: "once entered it can only
    /// be moved. the user should be able to rotate too."**
    /// `finish_markup_rotate` is the drag-driven counterpart to typing
    /// `rotate` twice; this checks the turn lands in the direction a
    /// real drag implies rather than merely that *something* moved —
    /// the sign is the part a rotation this easily gets backwards
    /// silently (see that function's own doc for the derivation this
    /// confirms).
    #[test]
    fn dragging_the_rotate_handle_turns_a_drawn_shape() {
        let mut app = app();
        app.submit("l 100,100 200,100");
        app.submit("all");

        let bounds = app.markup_selection_bounds(0).expect("a bounds to rotate about");
        let cx = (bounds.left + bounds.right) / 2.0;
        let cy = (bounds.top + bounds.bottom) / 2.0;

        // The drag: from "12 o'clock" of the selection's own centre to
        // "3 o'clock" — a quarter turn the way a clock's hands actually
        // move.
        let from = AppPoint { x: cx as f64, y: (cy - 50.0) as f64 };
        let by = (50.0, 50.0);
        let grab = Grab { handle: Some(Handle::Rotate), from, by };
        app.finish_markup_rotate(0, grab, bounds);

        match &marks(&app)[0].geom {
            Geom::Line(l) => {
                // Kernel space is y-up. A clockwise quarter turn about
                // the line's own midpoint carries its left end (9
                // o'clock) up to 12, and its right end (3 o'clock) down
                // to 6.
                assert!((l.a.x - 150.0).abs() < 1e-3, "a = {:?}", l.a);
                assert!((l.a.y - 150.0).abs() < 1e-3, "the left end should have swung up: a = {:?}", l.a);
                assert!((l.b.x - 150.0).abs() < 1e-3, "b = {:?}", l.b);
                assert!((l.b.y - 50.0).abs() < 1e-3, "the right end should have swung down: b = {:?}", l.b);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn measuring_reports_a_distance_and_calibration_changes_it() {
        let mut app = app();
        app.submit("measure distance");
        app.submit("pick 0,0");
        app.submit("pick 100,0");
        // The tool stays in hand, so the last line is its fresh prompt and
        // the answer is the one before it.
        let said = app.cmd.history()[app.cmd.history().len() - 2].text.clone();
        assert!(said.contains("100"), "uncalibrated distance wrong: {said}");
        assert!(said.contains("not calibrated"), "should warn: {said}");

        app.escape();
        app.submit("calibrate 5 m");
        app.submit("pick 0,0");
        app.submit("pick 100,0");
        app.submit("measure distance");
        app.submit("pick 0,0");
        app.submit("pick 200,0");
        let said = app.cmd.history()[app.cmd.history().len() - 2].text.clone();
        assert!(said.contains("10.000 m"), "calibrated distance wrong: {said}");
    }

    #[test]
    fn escape_style_cancellation_leaves_nothing_half_done() {
        let mut app = app();
        app.submit("fillet");
        app.submit("pick 100,250"); // misses — nothing there
        assert!(app.tab_mut().pending.is_some(), "a miss must not cancel the tool");
        assert!(marks(&app).is_empty());
    }
}

/// Not a test — a listing. Run with
/// `cargo test -p pagify_app font_coverage -- --nocapture --ignored`
/// to see which characters the bundled fonts can actually draw, which is
/// the only honest way to choose an icon set for them.
#[test]
#[ignore]
fn font_coverage() {
    let ctx = egui::Context::default();
    let mut frame = ctx.run_ui(Default::default(), |_| {});
    frame.textures_delta.clear();

    let font = egui::FontId::proportional(19.0);
    let ranges: &[(&str, u32, u32)] = &[
        ("latin symbols", 0x2000, 0x206F),
        ("letterlike", 0x2100, 0x214F),
        ("arrows", 0x2190, 0x21FF),
        ("maths", 0x2200, 0x22FF),
        ("technical", 0x2300, 0x23FF),
        ("enclosed", 0x2460, 0x24FF),
        ("box drawing", 0x2500, 0x257F),
        ("blocks", 0x2580, 0x259F),
        ("geometric", 0x25A0, 0x25FF),
        ("misc symbols", 0x2600, 0x26FF),
        ("dingbats", 0x2700, 0x27BF),
        ("supplemental arrows", 0x2900, 0x297F),
        ("misc symbols b", 0x2B00, 0x2BFF),
        ("emoji: misc", 0x1F300, 0x1F5FF),
        ("emoji: transport", 0x1F680, 0x1F6FF),
    ];

    ctx.fonts_mut(|fonts| {
        for (name, from, to) in ranges {
            let have: String = (*from..=*to)
                .filter_map(char::from_u32)
                .filter(|c| fonts.has_glyph(&font, *c))
                .collect();
            println!("\n== {name} ({} of {}) ==\n{have}", have.chars().count(), to - from + 1);
        }
    });
}

/// Every ribbon glyph must be a glyph the bundled fonts can draw.
///
/// The failure this catches is silent and total: a character egui has no
/// font for renders as an empty box, so a missing glyph does not look like
/// a missing glyph — it looks like a broken button, on a toolbar of two
/// hundred where nobody would notice which one. Emoji coverage is a
/// property of the fonts egui bundles, not of the platform, so it can be
/// asserted here rather than discovered by looking at a screen.
#[test]
fn every_ribbon_glyph_can_actually_be_drawn() {
    let ctx = egui::Context::default();
    // Fonts are built lazily on the first frame; without one there is
    // nothing to ask. The texture delta has to be taken rather than
    // dropped — epaint panics on an unhandled one, on the reasoning that a
    // real backend forgetting to upload it would be a bug.
    let mut frame = ctx.run_ui(Default::default(), |_| {});
    frame.textures_delta.clear();

    install_icons(&ctx);
    let mut frame = ctx.run_ui(Default::default(), |_| {});
    frame.textures_delta.clear();

    let font = icon_font(19.0);
    let mut missing: Vec<String> = Vec::new();

    ctx.fonts_mut(|fonts| {
        for tab in Tab::ALL {
            for (glyph, label, _) in tab.leading().iter().chain(tab.buttons()) {
                for ch in glyph.chars() {
                    if !fonts.has_glyph(&font, ch) {
                        missing.push(format!(
                            "{}/{label}: U+{:04X} {ch:?}",
                            tab.label(),
                            ch as u32
                        ));
                    }
                }
            }
        }
    });

    missing.sort();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "{} glyphs would render as empty boxes:\n  {}",
        missing.len(),
        missing.join("\n  ")
    );
}

/// `button_group_starts` names its dividers by index into `buttons()`,
/// not by the tool itself — so an edit to `Tab::Draw`'s own button list
/// that shifts anything could silently move the divider to in front of
/// the wrong tool instead of failing to compile. This pins the two
/// indices to the tools they're meant to mark.
#[test]
fn draws_divider_group_starts_point_at_the_tools_named_in_their_own_comment() {
    let buttons = Tab::Draw.buttons();
    let starts = Tab::Draw.button_group_starts();
    assert_eq!(starts, &[7, 11]);
    assert_eq!(buttons[7].1, "Trim");
    assert_eq!(buttons[11].1, "Calibrate");
}

/// No ribbon button's own glyph-plus-label content is taller than the
/// box `tool_button` draws it into, at today's `TOOL_WIDTH`/
/// `TOOL_HEIGHT` — using the exact layout calls `tool_button` itself
/// makes (`layout_no_wrap` for the glyph, `layout` wrapped to
/// `size.x - 8.0` for the label), so this is the actual risk a size
/// change runs, not a guess at it. `tool_button` paints centred rather
/// than clipped, so a label that doesn't fit overflows the button's own
/// rect into whatever is drawn next to it — silent, the same failure
/// mode `every_ribbon_glyph_can_actually_be_drawn` guards for icons.
#[test]
fn no_ribbon_label_overflows_its_own_button() {
    let ctx = egui::Context::default();
    let mut frame = ctx.run_ui(Default::default(), |_| {});
    frame.textures_delta.clear();
    install_icons(&ctx);
    let mut frame = ctx.run_ui(Default::default(), |_| {});
    frame.textures_delta.clear();

    let mut overflowing: Vec<String> = Vec::new();
    let mut frame = ctx.run_ui(Default::default(), |ui| {
        let painter = ui.painter();
        for tab in Tab::ALL {
            for (_, label, _) in tab.leading().iter().chain(tab.buttons()) {
                let glyph_h = painter.layout_no_wrap("x".to_owned(), icon_font(19.0), theme::ink()).size().y;
                let label_galley = painter.layout(
                    (*label).to_owned(),
                    egui::FontId::proportional(10.0),
                    theme::ink(),
                    TOOL_WIDTH - 8.0,
                );
                let total = glyph_h + 3.0 + label_galley.size().y;
                if total > TOOL_HEIGHT || label_galley.size().x > TOOL_WIDTH {
                    overflowing.push(format!(
                        "{}/{label}: needs {:.0}x{:.0}, box is {TOOL_WIDTH:.0}x{TOOL_HEIGHT:.0}",
                        tab.label(),
                        label_galley.size().x.max(TOOL_WIDTH),
                        total,
                    ));
                }
            }
        }
    });
    frame.textures_delta.clear();

    assert!(
        overflowing.is_empty(),
        "{} buttons overflow their own box:\n  {}",
        overflowing.len(),
        overflowing.join("\n  ")
    );
}

/// Every tab but `Draw` keeps a single, undivided group — asserted so
/// that adding a tool to one of them isn't silently read as "add a
/// divider" if a future edit near `button_group_starts` gets careless.
#[test]
fn only_draw_has_grouped_tools() {
    for tab in Tab::ALL {
        if tab == Tab::Draw {
            assert!(!tab.button_group_starts().is_empty());
        } else {
            assert!(tab.button_group_starts().is_empty(), "{:?} should have no dividers", tab.label());
        }
    }
}

/// Every ribbon button must be a command the box actually understands.
///
/// This is what keeps §7's claim true as buttons are added: a button whose
/// command string is a typo is a button that does nothing, and nothing else
/// would catch it.
#[test]
fn every_ribbon_button_runs_a_command_the_box_understands() {
    for tab in Tab::ALL {
        for (_glyph, label, command) in tab.leading().iter().chain(tab.buttons()) {
            let line = command.trim();
            // A trailing space means the button pre-fills the box for the
            // user to complete — those are checked as their bare verb.
            match pagify_shell::command::dispatch(line) {
                Some(Dispatch::Unknown(token)) => {
                    panic!("{}/{label} runs `{command}`, and `{token}` is not a command", tab.label())
                }
                Some(Dispatch::Refused { token, .. }) => {
                    panic!("{}/{label} runs `{command}`, which Pagify refuses ({token})", tab.label())
                }
                _ => {}
            }
        }
    }
}

/// **Reported from use, twice.** First: clicking "Add Text" failed
/// immediately with "the words to write, as in `addtext Draft`" — the
/// button ran bare `addtext`, which back then had no words yet to
/// place, so it was made to pre-fill the box with a trailing space
/// instead of submitting. Second: typing the words into the command box
/// *before* knowing where they would land turned out to be "totally
/// confusing and unintuitive" on its own — so bare `addtext` now drags
/// out a box to type into instead (see `begin_text_box`), and every Add
/// Text button should run it bare, submitting immediately, the same
/// click-and-place shape Add Images already has.
#[test]
fn add_text_buttons_submit_bare_and_arm_the_box_tool() {
    for tab in Tab::ALL {
        for (_glyph, label, command) in tab.leading().iter().chain(tab.buttons()) {
            if command.trim() == "addtext" {
                assert_eq!(
                    *command, "addtext",
                    "{}/{label} runs `{command}`, not bare `addtext`",
                    tab.label()
                );
            }
        }
    }
}
