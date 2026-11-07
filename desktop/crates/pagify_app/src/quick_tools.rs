//! The quick tools: icon buttons at the left of the title bar, after the logo,
//! as Foxit has them. Which are shown is the program's, not a window's — lent
//! to each window through the hub's `Shared` — and kept in `quick-tools.json`.

use std::io;
use std::path::{Path, PathBuf};

use crate::ribbon::Tab;

pub(crate) struct QuickTool {
    pub(crate) glyph: &'static str,
    pub(crate) label: &'static str,
    pub(crate) command: &'static str,
}

const fn tool(glyph: &'static str, label: &'static str, command: &'static str) -> QuickTool {
    QuickTool { glyph, label, command }
}

/// Foxit's list, in its order, less Email (Pagify's is not built yet), plus
/// Save for sharing (asked for). Each runs a command string, like every ribbon
/// button.
pub(crate) const STANDARD: [QuickTool; 10] = [
    tool("\u{E2C8}", "Open", "open"),
    tool("\u{E161}", "Save", "save"),
    tool("\u{EB60}", "Save As", "saveas"),
    tool("\u{E80D}", "Save for sharing", "sharecopy"),
    tool("\u{E8AD}", "Print", "print"),
    tool("\u{E66D}", "Blank", "blankdoc"),
    tool("\u{E166}", "Undo", "undo"),
    tool("\u{E15A}", "Redo", "redo"),
    tool("\u{E925}", "Hand", "hand"),
    tool("\u{EF52}", "Select", "selecttool"),
];

/// Ticked until someone changes it: Foxit's defaults, less Email.
pub(crate) const DEFAULTS: [&str; 7] = ["open", "save", "print", "blankdoc", "undo", "redo", "hand"];

/// The `v` that opens "Customize Quick Access Toolbar".
pub(crate) const MENU_GLYPH: &str = "\u{E5C5}";

pub(crate) fn defaults() -> Vec<String> {
    DEFAULTS.map(String::from).to_vec()
}

/// Beside `exact-pages.json`, in Pagify's state folder.
pub(crate) fn path() -> Option<PathBuf> {
    pagify_shell::state::state_dir().map(|dir| dir.join("quick-tools.json"))
}

/// A missing or unreadable file, or one that is not a list of commands, is the
/// defaults — never an error.
pub(crate) fn load_from(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .unwrap_or_else(defaults)
}

pub(crate) fn save_to(path: &Path, chosen: &[String]) -> io::Result<()> {
    pagify_shell::state::write_own(path, serde_json::to_string(chosen)?.as_bytes())
}

/// Untick if ticked, else tick — a new one goes to the end.
pub(crate) fn toggle(chosen: &mut Vec<String>, command: &str) {
    match chosen.iter().position(|c| c == command) {
        Some(i) => {
            chosen.remove(i);
        }
        None => chosen.push(command.to_owned()),
    }
}

/// What the bar draws, as (glyph, label, command): the ticked [`STANDARD`]
/// ones in their own order, then any other ribbon button in the order it was
/// ticked. A command no ribbon button runs any more is dropped.
pub(crate) fn shown(chosen: &[String]) -> Vec<(&'static str, &'static str, &'static str)> {
    let mut out: Vec<_> = STANDARD
        .iter()
        .filter(|t| chosen.iter().any(|c| c == t.command))
        .map(|t| (t.glyph, t.label, t.command))
        .collect();
    for c in chosen.iter().filter(|c| !STANDARD.iter().any(|t| t.command == c.as_str())) {
        let button = Tab::ALL
            .iter()
            .flat_map(|tab| tab.leading().iter().chain(tab.buttons()))
            .find(|(_, _, command)| command.text().trim() == c.as_str());
        if let Some((glyph, label, command)) = button {
            out.push((*glyph, *label, command.text().trim()));
        }
    }
    out
}

/// One quick tool: a 28 px icon button named `label` for the screen reader
/// and on hover, greyed when `!enabled`. An empty button for the frame and
/// the hover state, with the glyph drawn over it, as `ribbon::history_toggle`.
pub(crate) fn quick_tool_button(ui: &mut egui::Ui, glyph: &str, label: &str, enabled: bool) -> egui::Response {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("").frame_when_inactive(false).min_size(egui::vec2(28.0, 28.0)),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    if ui.is_rect_visible(response.rect) {
        let colour = if enabled { ui.style().interact(&response).fg_stroke.color } else { crate::theme::ink_faint() };
        ui.painter().text(response.rect.center(), egui::Align2::CENTER_CENTER, glyph, crate::icon_font(18.0), colour);
    }
    response.on_hover_text(label).on_disabled_hover_text(label)
}

impl crate::PagifyApp {
    /// Tick or untick a quick tool, and keep the choice.
    pub(crate) fn set_quick_tool(&mut self, command: &str) {
        toggle(&mut self.ui_state.quick_tools, command);
        if !cfg!(test) {
            if let Some(path) = path() {
                let _ = save_to(&path, &self.ui_state.quick_tools);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pagify-quick-tools-{}", std::process::id()));
        dir.join(name)
    }

    fn labels(chosen: &[&str]) -> Vec<&'static str> {
        let chosen: Vec<String> = chosen.iter().map(|c| c.to_string()).collect();
        shown(&chosen).into_iter().map(|(_, label, _)| label).collect()
    }

    #[test]
    fn the_defaults_are_foxits_less_email() {
        assert_eq!(load_from(&temp("missing.json")), DEFAULTS.map(String::from).to_vec());
    }

    #[test]
    fn a_choice_survives_a_restart() {
        let path = temp("kept.json");
        let chosen = vec!["redo".to_string(), "highlight".to_string()];
        save_to(&path, &chosen).expect("write");
        assert_eq!(load_from(&path), chosen);
    }

    #[test]
    fn a_broken_file_gives_the_defaults() {
        let path = temp("broken.json");
        pagify_shell::state::write_own(&path, b"{not json").expect("write");
        assert_eq!(load_from(&path), DEFAULTS.map(String::from).to_vec());
    }

    #[test]
    fn the_menu_list_comes_first_in_its_own_order() {
        assert_eq!(labels(&["redo", "highlight", "open"]), ["Open", "Redo", "Highlight"]);
    }

    /// **Asked for: "add save for sharing".** In the menu, after Save As.
    #[test]
    fn save_for_sharing_is_in_the_menu_after_save_as() {
        let commands: Vec<&str> = STANDARD.iter().map(|t| t.command).collect();
        let at = commands.iter().position(|c| *c == "saveas").expect("Save As");
        assert_eq!(commands.get(at + 1), Some(&"sharecopy"), "{commands:?}");
    }

    #[test]
    fn an_unknown_command_is_dropped() {
        assert_eq!(labels(&["open", "nosuchcommand"]), ["Open"]);
    }

    #[test]
    fn toggle_adds_at_the_end_and_removes() {
        let mut chosen = vec!["open".to_string(), "save".to_string()];
        toggle(&mut chosen, "highlight");
        assert_eq!(chosen, ["open", "save", "highlight"]);
        toggle(&mut chosen, "open");
        assert_eq!(chosen, ["save", "highlight"]);
    }
}
