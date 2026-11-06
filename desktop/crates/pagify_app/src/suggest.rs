//! Tool suggestions over the command box, as AutoCAD's AutoComplete: while the
//! first word is typed, the ribbon tools it almost matches, best first, to be
//! picked with the arrows and Enter or a click.

use pagify_shell::spelling::edit_distance;

use crate::ribbon::Tab;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Suggestion {
    pub(crate) glyph: &'static str,
    pub(crate) label: &'static str,
    pub(crate) command: &'static str,
    pub(crate) tab: &'static str,
}

/// At most this many rows.
pub(crate) const SHOWN: usize = 8;

/// The ribbon tools `typed` almost matches, best first, then by name: the
/// command or the name starts with it; then a later word of the name does;
/// then either contains it; then, for 3 letters or more, a near miss — one
/// letter wrong up to 5 letters, two for longer — against the command or a
/// word of the name, whole or its start. Nothing once a space is typed: the
/// rest is arguments.
pub(crate) fn suggest(typed: &str) -> Vec<Suggestion> {
    let typed = typed.trim().to_lowercase();
    if typed.is_empty() || typed.contains(char::is_whitespace) {
        return Vec::new();
    }
    let t: Vec<char> = typed.chars().collect();
    let allowed = if t.len() <= 5 { 1 } else { 2 };
    let near = |word: &str| {
        let w: Vec<char> = word.chars().collect();
        edit_distance(&t, &w) <= allowed || edit_distance(&t, &w[..w.len().min(t.len())]) <= allowed
    };

    let mut seen: Vec<&str> = Vec::new();
    let mut ranked: Vec<(u8, Suggestion)> = Vec::new();
    for tab in Tab::ALL {
        for (glyph, label, command) in tab.leading().iter().chain(tab.buttons()) {
            let command = command.text().trim();
            if seen.contains(&command) {
                continue;
            }
            seen.push(command);
            let name = label.to_lowercase();
            let words: Vec<&str> =
                name.split(|c: char| c.is_whitespace() || "…&/()".contains(c)).filter(|w| !w.is_empty()).collect();
            let rank = if command.starts_with(&typed) || name.starts_with(&typed) {
                0
            } else if words.iter().any(|w| w.starts_with(&typed)) {
                1
            } else if command.contains(&typed) || name.contains(&typed) {
                2
            } else if t.len() >= 3 && (near(command) || words.iter().any(|w| near(w))) {
                3
            } else {
                continue;
            };
            ranked.push((rank, Suggestion { glyph, label, command, tab: tab.label() }));
        }
    }
    ranked.sort_by(|a, b| (a.0, a.1.label).cmp(&(b.0, b.1.label)));
    ranked.into_iter().take(SHOWN).map(|(_, s)| s).collect()
}

impl crate::PagifyApp {
    /// What the list over the command box shows this frame: nothing unless the
    /// box is `focused`, no tool waits for points, the box is not asking for a
    /// password or passcode, and Escape has not closed it.
    ///
    /// `focused` includes losing focus this frame: the box loses it to Enter
    /// or to a click on a row as it is drawn, and those have to see the list
    /// they act on.
    pub(crate) fn suggestions_showing(&self, focused: bool) -> Vec<Suggestion> {
        let open = focused
            && self.tab().tool.is_none()
            && self.tab().secure_state.awaiting_password.is_none()
            && !self.ui_state.suggestions_closed;
        if open { suggest(self.cmd.input()) } else { Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(typed: &str) -> Vec<&'static str> {
        suggest(typed).into_iter().map(|s| s.label).collect()
    }

    #[test]
    fn hig_lists_highlight_first() {
        assert_eq!(labels("hig").first(), Some(&"Highlight"), "{:?}", labels("hig"));
    }

    #[test]
    fn a_word_inside_the_name_matches() {
        let found = labels("light");
        assert!(found.contains(&"Highlight") && found.contains(&"Light Theme"), "{found:?}");
    }

    #[test]
    fn a_near_miss_still_finds_it() {
        assert!(labels("higlight").contains(&"Highlight"), "{:?}", labels("higlight"));
    }

    #[test]
    fn nothing_like_it_lists_nothing() {
        assert!(suggest("zzzz").is_empty(), "{:?}", labels("zzzz"));
    }

    #[test]
    fn a_start_outranks_a_middle() {
        let found = labels("line");
        let line = found.iter().position(|l| *l == "Line").expect("no Line");
        let polyline = found.iter().position(|l| *l == "Polyline").expect("no Polyline");
        assert!(line < polyline, "{found:?}");
    }

    #[test]
    fn at_most_eight_and_no_command_twice() {
        let found = suggest("e");
        assert!(!found.is_empty() && found.len() <= SHOWN, "{}", found.len());
        let mut commands: Vec<&str> = found.iter().map(|s| s.command).collect();
        commands.sort();
        commands.dedup();
        assert_eq!(commands.len(), found.len(), "a command listed twice");
    }

    #[test]
    fn arguments_turn_the_list_off() {
        assert!(suggest("find two").is_empty());
    }
}
