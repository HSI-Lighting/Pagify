//! The print window: settings on the left, the sheet on the right, **Print**.
//!
//! Windows-only (gated in `main.rs` like `print_windows`, which it drives).
//!
//! **Why this is Pagify's own window and not the OS dialog.** On Windows 11
//! the OS Print dialog's preview pane is only filled by an app that supplies a
//! WinRT print source, which an egui window cannot — so it said "This app
//! doesn't support print preview". The preview here is drawn from
//! [`pagify_shell::print_layout::place`], the very function the printer job
//! places each page with, so what the sheet shows is what comes out.
//!
//! The state is `PanelsState::print_ask`, taken and put back each frame like
//! `extract_ask`'s.

use crate::print_windows::{self, Orientation, PrinterInfo, PrinterSetup};
use crate::{page_to_image, theme, PagifyApp};
use pagify_shell::print_layout::{self, Fit, Options, Paper, Placement};
use pdf_core::Rotation;

/// One page's picture on the sheet, rendered for the size it is shown at.
struct Preview {
    /// What this picture is *of*; any change re-renders it.
    key: (usize, u8, u32),
    texture: egui::TextureHandle,
}

/// Where a frame drew the sheet and the page on it, in screen points — what a
/// test measures, so the preview is checked against the picture and not
/// against the function that drew it.
// `sheet` and `page` are read by the tests, which is what they are for.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Shown {
    pub(crate) sheet: egui::Rect,
    pub(crate) page: egui::Rect,
    pub(crate) rotation: Rotation,
    pub(crate) clipped: bool,
}

pub(crate) struct PrintAsk {
    printers: Vec<PrinterInfo>,
    chosen: usize,
    setup: Option<PrinterSetup>,
    paper: Option<Paper>,
    /// The sheet must be asked for again (printer, orientation or driver
    /// settings changed). A network printer can be slow, so only then.
    paper_stale: bool,
    orientation: Orientation,
    copies: u32,
    collate: bool,
    pages: String,
    options: Options,
    /// Which of the pages to be printed the sheet on show is.
    sheet: usize,
    /// `(page, size in points)` for the page on show — asked of the engine once
    /// per page, not once per frame.
    page_size: Option<(usize, (f32, f32))>,
    preview: Option<Preview>,
    problem: Option<String>,
    /// The window to parent the driver's dialog to. An integer, not an `HWND`,
    /// so this state stays `Send` like the rest of the tab's.
    owner: isize,
    pub(crate) shown: Option<Shown>,
}

const SHEET_W: f32 = 540.0;

/// The most a preview is rendered across: the sheet on show is never bigger
/// than a screen, and a big drawing at its own size would be tens of megabytes
/// for a picture a few hundred points wide.
const PREVIEW_MAX_PX: u32 = 2400;

impl PrintAsk {
    fn new(printers: Vec<PrinterInfo>, owner: isize) -> Self {
        let mut ask = PrintAsk {
            printers,
            chosen: 0,
            setup: None,
            paper: None,
            paper_stale: false,
            orientation: Orientation::Portrait,
            copies: 1,
            collate: true,
            pages: "all".into(),
            options: Options::default(),
            sheet: 0,
            page_size: None,
            preview: None,
            problem: None,
            owner,
            shown: None,
        };
        ask.choose_printer();
        ask
    }

    /// A window that is not tied to any printer, for a test to hand a sheet.
    #[cfg(test)]
    pub(crate) fn for_test(paper: Paper) -> Self {
        let mut ask = PrintAsk::new(Vec::new(), 0);
        ask.paper = Some(paper);
        ask.paper_stale = false;
        ask
    }

    #[cfg(test)]
    pub(crate) fn set_options(&mut self, options: Options) {
        self.options = options;
    }

    #[cfg(test)]
    pub(crate) fn set_pages(&mut self, pages: &str) {
        self.pages = pages.into();
    }

    /// Open the chosen printer's own settings. A printer that will not answer
    /// leaves the window up with the reason, rather than closing on the person.
    fn choose_printer(&mut self) {
        self.setup = None;
        self.paper = None;
        self.preview = None;
        let Some(printer) = self.printers.get(self.chosen) else { return };
        match PrinterSetup::new(&printer.name) {
            Ok(setup) => {
                self.orientation = setup.orientation();
                self.copies = setup.copies();
                self.setup = Some(setup);
                self.paper_stale = true;
                self.problem = None;
            }
            Err(e) => self.problem = Some(e),
        }
    }

    /// Send the orientation to the driver and read the sheet back.
    fn refresh_paper(&mut self) {
        self.paper_stale = false;
        let Some(setup) = self.setup.as_mut() else { return };
        let outcome = setup.apply(self.orientation, self.copies, self.collate).and_then(|()| print_windows::paper(setup));
        match outcome {
            Ok(paper) => {
                self.paper = Some(paper);
                self.problem = None;
            }
            Err(e) => {
                self.paper = None;
                self.problem = Some(e);
            }
        }
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

impl PagifyApp {
    /// Ctrl+P: open the print window on this tab's document.
    pub(crate) fn open_print_window(&mut self, frame: &mut eframe::Frame) {
        use raw_window_handle::HasWindowHandle as _;
        if self.tab().doc.is_none() {
            self.say_error("nothing open.");
            return;
        }
        let hwnd = match frame.window_handle().map(|h| h.as_raw()) {
            Ok(raw_window_handle::RawWindowHandle::Win32(win32)) => {
                windows::Win32::Foundation::HWND(win32.hwnd.get() as *mut std::ffi::c_void)
            }
            Ok(_) => {
                self.say_error("this window has no HWND to print from.");
                return;
            }
            Err(e) => {
                self.say_error(format!("couldn't find this window to print from: {e}"));
                return;
            }
        };
        // `frame` is the first window's. With several windows the one to
        // print from is the one the person is using — see `hub`.
        let hwnd = crate::hub::active_window().unwrap_or(hwnd);

        let printers = match print_windows::list_printers() {
            Ok(p) if !p.is_empty() => p,
            Ok(_) => {
                self.say_error("there is no printer installed to print to.");
                return;
            }
            Err(e) => {
                self.say_error(e);
                return;
            }
        };
        self.tab_mut().panels.print_ask = Some(PrintAsk::new(printers, hwnd.0 as isize));
    }

    pub(crate) fn draw_print_window(&mut self, ctx: &egui::Context) {
        let Some(mut ask) = self.tab_mut().panels.print_ask.take() else { return };
        let Some((count, session)) = self.tab().doc.as_ref().map(|d| (d.page_count, d.session.clone())) else { return };
        let current = self.tab().view_state.page;
        let unsaved = self.unsaved();

        if ask.paper_stale {
            ask.refresh_paper();
        }
        let wanted = pagify_shell::organize::parse_range(&ask.pages, count);
        let pages = wanted.clone().unwrap_or_default();
        ask.sheet = ask.sheet.min(pages.len().saturating_sub(1));
        let on_show = pages.get(ask.sheet).copied();
        // One engine answer per page; the same `page_size` the job places by.
        if let Some(page) = on_show {
            if ask.page_size.map(|(p, _)| p) != Some(page) {
                ask.page_size = session.page_size(page).ok().map(|s| (page, (s.width_pt, s.height_pt)));
            }
        }
        let placement = match (&ask.paper, ask.page_size) {
            (Some(paper), Some((_, size))) if on_show.is_some() => Some(print_layout::place(paper, size, &ask.options)),
            _ => None,
        };

        let sheet_h = (ctx.content_rect().height() - 330.0).clamp(300.0, 560.0);
        let (mut cancel, mut go, mut properties, mut printer_changed, mut paper_changed) = (false, false, false, false, false);
        let can_print = ask.setup.is_some() && ask.paper.is_some() && !pages.is_empty();

        egui::Modal::new(egui::Id::new("print-window")).show(ctx, |ui| {
            ui.set_width(SHEET_W + 360.0);
            ui.heading("Print");
            ui.add_space(6.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(310.0);

                    ui.label(egui::RichText::new("Printer").strong());
                    let before = ask.chosen;
                    let name = ask.printers.get(ask.chosen).map(|p| p.name.clone()).unwrap_or_default();
                    egui::ComboBox::from_id_salt("print-printer").width(300.0).selected_text(name).show_ui(ui, |ui| {
                        for (i, p) in ask.printers.iter().enumerate() {
                            let label = if p.is_default { format!("{} (default)", p.name) } else { p.name.clone() };
                            ui.selectable_value(&mut ask.chosen, i, label);
                        }
                    });
                    printer_changed = ask.chosen != before;
                    if ui.add_enabled(ask.setup.is_some(), egui::Button::new("Printer properties\u{2026}")).clicked() {
                        properties = true;
                    }

                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("Orientation").strong());
                    ui.horizontal(|ui| {
                        let before = ask.orientation;
                        ui.radio_value(&mut ask.orientation, Orientation::Portrait, "Portrait");
                        ui.radio_value(&mut ask.orientation, Orientation::Landscape, "Landscape");
                        paper_changed |= ask.orientation != before;
                    });

                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("Pages").strong());
                    ui.add(egui::TextEdit::singleline(&mut ask.pages).desired_width(300.0).hint_text("e.g. 1-3,7 or all"));
                    ui.horizontal(|ui| {
                        if ui.small_button("This page").clicked() {
                            ask.pages = (current + 1).to_string();
                        }
                        if ui.small_button("All pages").clicked() {
                            ask.pages = "all".into();
                        }
                    });
                    if let Err(e) = &wanted {
                        ui.colored_label(theme::danger(), e);
                    }

                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Copies").strong());
                        ui.add(egui::DragValue::new(&mut ask.copies).range(1..=99));
                        if ask.copies > 1 {
                            ui.checkbox(&mut ask.collate, "Collate");
                        }
                    });

                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("Scale").strong());
                    ui.radio_value(&mut ask.options.fit, Fit::Page, "Fit to page");
                    ui.radio_value(&mut ask.options.fit, Fit::ActualSize, "Actual size");
                    ui.checkbox(&mut ask.options.rotate_to_fit, "Rotate pages to fit");

                    if let Some((marks, on_pages)) = unsaved {
                        ui.add_space(10.0);
                        ui.colored_label(
                            theme::warn(),
                            format!(
                                "{marks} unsaved mark{} on {on_pages} page{} aren't included \u{2014} save first.",
                                plural(marks),
                                plural(on_pages)
                            ),
                        );
                    }
                    if let Some(problem) = &ask.problem {
                        ui.add_space(10.0);
                        ui.colored_label(theme::danger(), problem);
                    }
                });

                ui.add_space(16.0); // not `ui.separator()`: in a horizontal row it takes the full available height, so the modal grew every frame

                ui.vertical(|ui| {
                    ui.set_width(SHEET_W);
                    let (pane, _) = ui.allocate_exact_size(egui::vec2(SHEET_W, sheet_h), egui::Sense::hover());
                    let painter = ui.painter_at(pane);
                    painter.rect_filled(pane, 4.0, theme::void());
                    ask.shown = None;
                    if let (Some(paper), Some(placement), Some(page)) = (ask.paper, placement, on_show) {
                        let shown = draw_sheet(ctx, &painter, pane, &paper, &placement, page, &session, &mut ask.preview);
                        ask.shown = Some(shown);
                    } else {
                        painter.text(
                            pane.center(),
                            egui::Align2::CENTER_CENTER,
                            "Nothing to preview",
                            egui::FontId::proportional(14.0),
                            theme::ink_dim(),
                        );
                    }

                    ui.horizontal(|ui| {
                        if ui.add_enabled(ask.sheet > 0, egui::Button::new("\u{25C0}")).clicked() {
                            ask.sheet -= 1;
                        }
                        if ui.add_enabled(ask.sheet + 1 < pages.len(), egui::Button::new("\u{25B6}")).clicked() {
                            ask.sheet += 1;
                        }
                        match on_show {
                            Some(page) => ui.label(format!(
                                "Page {} \u{2014} sheet {} of {}",
                                page + 1,
                                ask.sheet + 1,
                                pages.len()
                            )),
                            None => ui.label("No pages chosen"),
                        };
                    });
                    if let Some(shown) = ask.shown {
                        if shown.clipped {
                            ui.colored_label(theme::warn(), "Clipped \u{2014} this page is larger than the printable area.");
                        } else if shown.rotation == Rotation::Clockwise90 {
                            ui.label(egui::RichText::new("Turned a quarter to fill the sheet.").color(theme::ink_dim()));
                        }
                    }
                });
            });

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let label = format!("Print {} page{}", pages.len(), plural(pages.len()));
                if ui.add_enabled(can_print, egui::Button::new(label)).clicked() {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });

        if cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.say_info("nothing was printed.");
            return;
        }
        if printer_changed {
            ask.choose_printer();
        }
        if paper_changed {
            ask.paper_stale = true;
        }
        if properties {
            let owner = windows::Win32::Foundation::HWND(ask.owner as *mut std::ffi::c_void);
            if let Some(setup) = ask.setup.as_mut() {
                match setup.show_properties(owner) {
                    Ok(true) => {
                        ask.orientation = setup.orientation();
                        ask.copies = setup.copies();
                        ask.paper_stale = true;
                    }
                    Ok(false) => {}
                    Err(e) => ask.problem = Some(e),
                }
            }
        }
        if go && can_print && wanted.is_ok() {
            let title = session
                .path()
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Pagify document".to_string());
            let printed = match ask.setup.as_mut() {
                Some(setup) => setup
                    .apply(ask.orientation, ask.copies, ask.collate)
                    .and_then(|()| print_windows::print(setup, &session, &title, &pages, &ask.options, None))
                    .map(|()| setup.name.clone()),
                None => Err("no printer is chosen.".to_string()),
            };
            match printed {
                Ok(printer) => {
                    self.say_info(format!("sent {} page{} to {printer}.", pages.len(), plural(pages.len())));
                    return;
                }
                Err(e) => {
                    self.say_error(e.clone());
                    ask.problem = Some(e);
                }
            }
        }
        self.tab_mut().panels.print_ask = Some(ask);
    }
}

/// Draw the sheet, its printable area and the page where `placement` puts it;
/// render the page's picture first if what is on show has changed.
#[allow(clippy::too_many_arguments)]
fn draw_sheet(
    ctx: &egui::Context,
    painter: &egui::Painter,
    pane: egui::Rect,
    paper: &Paper,
    placement: &Placement,
    page: usize,
    session: &pagify_shell::Session,
    preview: &mut Option<Preview>,
) -> Shown {
    let (pw, ph) = (paper.physical.0.max(1) as f32, paper.physical.1.max(1) as f32);
    let k = ((pane.width() - 24.0) / pw).min((pane.height() - 24.0) / ph);
    let sheet = egui::Rect::from_center_size(pane.center(), egui::vec2(pw * k, ph * k));
    let printable = egui::Rect::from_min_size(
        sheet.min + egui::vec2(paper.offset.0 as f32, paper.offset.1 as f32) * k,
        egui::vec2(paper.printable.0 as f32, paper.printable.1 as f32) * k,
    );
    let d = placement.dest;
    let page_rect = egui::Rect::from_min_size(
        printable.min + egui::vec2(d.x as f32, d.y as f32) * k,
        egui::vec2(d.w as f32, d.h as f32) * k,
    );

    painter.rect_filled(sheet, 0.0, egui::Color32::WHITE);
    painter.rect_stroke(sheet, 0.0, egui::Stroke::new(1.0, theme::line()), egui::StrokeKind::Outside);

    // The picture, at about the size it is shown on this screen.
    let across = (page_rect.width() * ctx.pixels_per_point()).ceil().max(1.0);
    let width_px = ((across / 128.0).ceil() as u32 * 128).clamp(128, PREVIEW_MAX_PX);
    let key = (page, placement.rotation as u8, width_px);
    if preview.as_ref().map(|p| p.key) != Some(key) {
        let turned = placement.rotation == Rotation::Clockwise90;
        *preview = session.page_size(page).ok().and_then(|size| {
            let across_pt = if turned { size.height_pt } else { size.width_pt };
            let raster = session.render_page_rotated(page, width_px as f32 / across_pt.max(1.0), placement.rotation).ok()?;
            let texture = ctx.load_texture(
                "print-preview",
                page_to_image(&raster),
                egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR },
            );
            Some(Preview { key, texture })
        });
    }
    // Only what is inside the printable area reaches the paper.
    let on_paper = painter.with_clip_rect(printable.intersect(painter.clip_rect()));
    if let Some(p) = preview.as_ref() {
        on_paper.image(
            p.texture.id(),
            page_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }
    let corners = [printable.left_top(), printable.right_top(), printable.right_bottom(), printable.left_bottom(), printable.left_top()];
    painter.extend(egui::Shape::dashed_line(&corners, egui::Stroke::new(1.0, theme::ink_faint()), 4.0, 3.0));

    Shown { sheet, page: page_rect, rotation: placement.rotation, clipped: placement.clipped }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_tests::harness_from;
    use egui_kittest::kittest::Queryable;
    use pagify_shell::print_layout::PxRect;

    /// A PDF whose pages are each one solid colour edge to edge, so what a
    /// test measures on a rendered sheet is exactly where the page landed.
    fn solid_pdf(sizes: &[(f32, f32)]) -> Vec<u8> {
        let n = sizes.len();
        let mut bodies: Vec<String> = Vec::new();
        bodies.push("<< /Type /Catalog /Pages 2 0 R >>".into());
        let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + 2 * i)).collect();
        bodies.push(format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")));
        for (i, (w, h)) in sizes.iter().enumerate() {
            bodies.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents {} 0 R >>",
                4 + 2 * i
            ));
            let content = format!("0 0 1 rg 0 0 {w} {h} re f");
            bodies.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
        }
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in bodies.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes());
        for o in offsets {
            out.extend(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).as_bytes());
        out
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("pagify-print-tests");
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir.join(name)
    }

    fn app_with(sizes: &[(f32, f32)], name: &str) -> PagifyApp {
        let path = scratch(name);
        std::fs::write(&path, solid_pdf(sizes)).expect("write the fixture");
        let app = PagifyApp::new(Some(&path.to_string_lossy()));
        assert!(app.tab().doc.is_some(), "{name} did not open");
        app
    }

    fn a4(landscape: bool) -> Paper {
        let (w, h) = if landscape { (3508, 2480) } else { (2480, 3508) };
        Paper { physical: (w, h), offset: (0, 0), printable: (w, h), dpi: (300.0, 300.0) }
    }

    fn open(app: PagifyApp, paper: Paper) -> egui_kittest::Harness<'static, PagifyApp> {
        let mut h = harness_from(app);
        h.state_mut().tab_mut().panels.print_ask = Some(PrintAsk::for_test(paper));
        h.run_steps(3);
        h
    }

    fn shown(h: &egui_kittest::Harness<'static, PagifyApp>) -> Option<Shown> {
        h.state().tab().panels.print_ask.as_ref().and_then(|a| a.shown)
    }

    #[test]
    fn a_portrait_page_on_landscape_paper_is_drawn_turned() {
        let h = open(app_with(&[(595.0, 842.0)], "turn.pdf"), a4(true));
        let s = shown(&h).expect("the sheet is drawn");
        assert_eq!(s.rotation, Rotation::Clockwise90);
        // Measured off what was painted, not off the layout function: the page
        // is wider than tall on the sheet, and sits in its middle.
        assert!(s.page.width() > s.page.height() * 1.3, "{s:?}");
        assert!((s.page.center() - s.sheet.center()).length() < 1.5, "{s:?}");
    }

    #[test]
    fn a_page_narrower_than_the_sheet_is_centred_across_it() {
        let h = open(app_with(&[(300.0, 842.0)], "thin.pdf"), a4(false));
        let s = shown(&h).expect("the sheet is drawn");
        assert_eq!(s.rotation, Rotation::None);
        assert!(s.page.width() < s.sheet.width() * 0.6, "{s:?}");
        assert!((s.page.center().x - s.sheet.center().x).abs() < 1.5, "{s:?}");
        assert!((s.page.height() - s.sheet.height()).abs() < 1.5, "{s:?}");
    }

    #[test]
    fn the_arrows_step_through_exactly_the_pages_asked_for() {
        let app = app_with(&[(595.0, 842.0), (612.0, 792.0), (420.0, 595.0)], "steps.pdf");
        let mut h = harness_from(app);
        let mut ask = PrintAsk::for_test(a4(false));
        ask.set_pages("1,2");
        h.state_mut().tab_mut().panels.print_ask = Some(ask);
        h.run_steps(3);
        let first = shown(&h).expect("drawn");
        let aspect = |s: &Shown| s.page.width() / s.page.height();
        assert!((aspect(&first) - 595.0 / 842.0).abs() < 0.01, "{first:?}");

        // A modal that grows or drifts between frames sends every click to
        // where its buttons *were*: the window must hold still. (It did not —
        // a `ui.separator()` in the row made it taller every frame.)
        let before = first.sheet;
        h.run_steps(5);
        assert_eq!(shown(&h).expect("drawn").sheet, before, "the window moved while nothing was happening");

        h.get_by_label("\u{25B6}").click();
        h.run_steps(3);
        let second = shown(&h).expect("drawn");
        assert!((aspect(&second) - 612.0 / 792.0).abs() < 0.01, "page 2 is Letter: {second:?}");

        // The last page asked for is the end: there is no third sheet, so a
        // further press changes nothing.
        h.get_by_label("\u{25B6}").click();
        h.run_steps(3);
        let third = shown(&h).expect("drawn");
        assert!((aspect(&third) - 612.0 / 792.0).abs() < 0.01, "stepped past the last page: {third:?}");
    }

    #[test]
    fn unsaved_marks_are_named_and_a_saved_document_says_nothing() {
        let mut app = app_with(&[(595.0, 842.0)], "banner.pdf");
        app.submit("l 30,30 200,200");
        let h = open(app, a4(false));
        assert!(h.query_by_label_contains("1 unsaved mark on 1 page").is_some(), "no banner for marks the printout would miss");

        let h = open(app_with(&[(595.0, 842.0)], "banner2.pdf"), a4(false));
        assert!(h.query_by_label_contains("unsaved mark").is_none(), "a banner with nothing to warn about");
    }

    #[test]
    fn escape_closes_the_window_without_printing() {
        let mut h = open(app_with(&[(595.0, 842.0)], "esc.pdf"), a4(false));
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert!(h.state().tab().panels.print_ask.is_none());
    }

    #[test]
    fn a_range_naming_no_page_previews_nothing() {
        let mut h = harness_from(app_with(&[(595.0, 842.0)], "none.pdf"));
        let mut ask = PrintAsk::for_test(a4(false));
        ask.set_pages("99");
        h.state_mut().tab_mut().panels.print_ask = Some(ask);
        h.run_steps(3);
        assert!(shown(&h).is_none());
    }

    /// End to end with no paper: three pages — upright, landscape on portrait
    /// paper, and a thin one — sent through the real GDI job to "Microsoft
    /// Print to PDF", then the *output* is rendered and measured. The expected
    /// position comes from `place`, the measured one from the pixels the
    /// printer produced, so neither is checking itself.
    #[test]
    fn a_job_to_pdf_lands_where_the_layout_says() {
        let Some(printer) =
            print_windows::list_printers().ok().and_then(|p| p.into_iter().find(|p| p.name == "Microsoft Print to PDF"))
        else {
            eprintln!("skipped: no Microsoft Print to PDF printer on this machine");
            return;
        };
        let sizes = [(595.0f32, 842.0f32), (842.0, 595.0), (300.0, 900.0)];
        let app = app_with(&sizes, "e2e-in.pdf");
        let session = app.tab().doc.as_ref().expect("open").session.clone();

        let mut setup = PrinterSetup::new(&printer.name).expect("settings");
        setup.apply(Orientation::Portrait, 1, false).expect("portrait");
        let paper = print_windows::paper(&setup).expect("paper");
        let options = Options::default();

        let out = scratch("e2e-out.pdf");
        let _ = std::fs::remove_file(&out);
        print_windows::print(&setup, &session, "e2e", &[0, 1, 2], &options, Some(&out)).expect("the job");

        // The spooler finishes the file after `EndDoc` returns.
        let mut last = 0u64;
        let mut still = 0;
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(300));
            let now = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
            still = if now > 0 && now == last { still + 1 } else { 0 };
            last = now;
            if still >= 3 {
                break;
            }
        }
        assert!(last > 0, "the PDF printer wrote nothing to {out:?}");

        let printed = PagifyApp::new(Some(&out.to_string_lossy()));
        let result = printed.tab().doc.as_ref().expect("the printed PDF opens");
        assert_eq!(result.page_count, 3, "one sheet per page asked for");

        for (i, &(w, h)) in sizes.iter().enumerate() {
            let sheet = result.session.page_size(i).expect("sheet size");
            // The PDF's sheet is the printer's physical sheet: one device pixel
            // is 1/dpi inch — the assumption the whole comparison rests on.
            let expect_w = paper.physical.0 as f32 * 72.0 / paper.dpi.0;
            let expect_h = paper.physical.1 as f32 * 72.0 / paper.dpi.1;
            assert!(
                (sheet.width_pt - expect_w).abs() < 1.5 && (sheet.height_pt - expect_h).abs() < 1.5,
                "sheet {i}: {sheet:?} vs {expect_w}x{expect_h}"
            );

            let raster = result.session.render_page(i, 1.0).expect("render the printed sheet");
            let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
            for (n, px) in raster.pixels.chunks_exact(4).enumerate() {
                if px[0] < 250 || px[1] < 250 || px[2] < 250 {
                    let (x, y) = (n as u32 % raster.width, n as u32 / raster.width);
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
            assert!(x1 > x0 && y1 > y0, "sheet {i} printed blank");
            let (mw, mh) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32);

            let p = print_layout::place(&paper, (w, h), &options);
            let PxRect { x, y, w: dw, h: dh } = p.dest;
            let to_pt = |px: i32, dpi: f32| px as f32 * 72.0 / dpi;
            let want = (
                to_pt(paper.offset.0 as i32 + x, paper.dpi.0),
                to_pt(paper.offset.1 as i32 + y, paper.dpi.1),
                to_pt(dw, paper.dpi.0),
                to_pt(dh, paper.dpi.1),
            );
            let near = |a: f32, b: f32| (a - b).abs() <= 3.0;
            assert!(
                near(x0 as f32, want.0) && near(y0 as f32, want.1) && near(mw, want.2) && near(mh, want.3),
                "sheet {i}: printed at ({x0},{y0}) {mw}x{mh}, layout said ({:.1},{:.1}) {:.1}x{:.1}",
                want.0,
                want.1,
                want.2,
                want.3
            );
        }

        // And the point of it, measured with no help from `place`: the
        // landscape page was turned, so it fills a portrait sheet's width
        // instead of leaving it half empty.
        let raster = result.session.render_page(1, 1.0).expect("render");
        let blue = raster.pixels.chunks_exact(4).filter(|px| px[2] > 200 && px[0] < 100).count() as f32;
        assert!(blue > 0.55 * (raster.width * raster.height) as f32, "a landscape page printed small on portrait paper");
    }
}
