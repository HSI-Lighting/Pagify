//! The paper a new blank document is made of: the window bare `blankdoc`
//! (the Blank button) opens before it asks where to save.

use pagify_shell::verbs::{PAGE_SIDE_PT, PAPERS};

use crate::PagifyApp;

/// What the window has chosen so far.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BlankPaper {
    /// An index into [`PAPERS`], or `None` for a size of the reader's own.
    pub paper: Option<usize>,
    /// That size of their own, in millimetres, as typed.
    pub custom_mm: (f32, f32),
    /// A named paper turned on its side.
    pub landscape: bool,
}

impl Default for BlankPaper {
    fn default() -> Self {
        BlankPaper { paper: Some(0), custom_mm: (210.0, 297.0), landscape: false }
    }
}

impl BlankPaper {
    /// The sheet, in points.
    pub fn size_pt(&self) -> (f32, f32) {
        match self.paper {
            Some(i) => {
                let (_, w, h) = PAPERS[i];
                if self.landscape { (h, w) } else { (w, h) }
            }
            None => (self.custom_mm.0 * 72.0 / 25.4, self.custom_mm.1 * 72.0 / 25.4),
        }
    }
}

const MM: f32 = 25.4 / 72.0;

impl PagifyApp {
    /// "New blank document": a paper and its way up; Create… then asks where.
    pub(crate) fn draw_blank_paper(&mut self, ctx: &egui::Context) {
        let Some(mut choice) = self.ui_state.blank_paper else { return };
        let mut open = true;
        let (mut create, mut cancel) = (false, false);
        egui::Window::new("New blank document").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            egui::Grid::new("blank-papers").num_columns(2).show(ui, |ui| {
                for (i, (name, w, h)) in PAPERS.iter().enumerate() {
                    ui.radio_value(&mut choice.paper, Some(i), *name);
                    ui.weak(format!("{:.0} × {:.0} mm", w * MM, h * MM));
                    ui.end_row();
                }
                ui.radio_value(&mut choice.paper, None, "Custom");
                ui.add_enabled_ui(choice.paper.is_none(), |ui| {
                    ui.horizontal(|ui| {
                        let sides = PAGE_SIDE_PT.start() * MM..=PAGE_SIDE_PT.end() * MM;
                        ui.add(egui::DragValue::new(&mut choice.custom_mm.0).range(sides.clone()).suffix(" mm"));
                        ui.label("×");
                        ui.add(egui::DragValue::new(&mut choice.custom_mm.1).range(sides).suffix(" mm"));
                    });
                });
                ui.end_row();
            });
            // A size of their own already says which way up it is.
            ui.add_enabled_ui(choice.paper.is_some(), |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut choice.landscape, false, "Portrait");
                    ui.selectable_value(&mut choice.landscape, true, "Landscape");
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                create = ui.button("Create…").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        self.ui_state.blank_paper = (open && !create && !cancel).then_some(choice);
        if create {
            self.new_blank_dialog(choice.size_pt());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn landscape_turns_a_named_paper_on_its_side() {
        let a3 = BlankPaper { paper: Some(1), landscape: true, ..Default::default() };
        assert_eq!(a3.size_pt(), (1191.0, 842.0));
    }

    /// A size of their own is width x height as typed: the way up is in the numbers.
    #[test]
    fn a_custom_size_is_millimetres_as_typed() {
        let own = BlankPaper { paper: None, custom_mm: (100.0, 50.0), landscape: false };
        let (w, h) = own.size_pt();
        assert!((w - 283.46).abs() < 0.01 && (h - 141.73).abs() < 0.01, "{w} x {h}");
    }

    /// Create… is not clicked here: it opens the system's Save dialog.
    #[test]
    fn a3_on_its_side_is_chosen_in_the_window_and_cancel_closes_it() {
        let mut h = crate::ui_tests::harness("two-column.pdf");
        h.state_mut().submit("blankdoc");
        h.run_steps(2);
        h.get_by_label("A3").click();
        h.run_steps(1);
        h.get_by_label("Landscape").click();
        h.run_steps(1);
        assert_eq!(h.state().ui_state.blank_paper.map(|p| p.size_pt()), Some((1191.0, 842.0)));

        h.get_by_label("Cancel").click();
        h.run_steps(1);
        assert!(h.state().ui_state.blank_paper.is_none());
    }
}
