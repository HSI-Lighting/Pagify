//! The command box: dispatch across two namespaces, and the state behind the
//! three-part bar.
//!
//! Build plan §7 — this is not a convenience wrapper over the buttons, it *is*
//! the interface. Every ribbon button will run a command string, so anything
//! clickable is typeable and anything typeable is scriptable. That is also why
//! all of it lives in `pagify_shell` rather than in the app: an interface whose
//! whole surface is a text box should be testable without a window, and every
//! rule below is asserted in `tests/command_box.rs`.

use crate::verbs::{self, Verb};

/// What a submitted line turned out to mean.
///
/// Not `Clone`: `cad_kernel::parser::Command` is not, and a dispatch is meant to
/// be consumed once by whoever acts on it rather than stored.
#[derive(Debug)]
pub enum Dispatch {
    /// Pagify's own verb — acts on the document.
    Pagify(Verb),
    /// The kernel's — acts on geometry.
    Kernel(Box<cad_kernel::parser::Command>),
    /// Understood, and declined: it means something only a CAD program has.
    Refused { token: String, why: &'static str },
    /// Neither namespace claims this word.
    Unknown(String),
    /// A verb one of them owns, with arguments it could not use.
    Bad(String),
}

/// Route one line through both namespaces.
///
/// Order is Pagify, then the refusal list, then the kernel — Pagify's table is
/// authoritative, and a word it claims is never also refused.
///
/// Returns `None` for a blank line, which is not an error and must not be
/// echoed as one.
pub fn dispatch(line: &str) -> Option<Dispatch> {
    let line = line.trim();
    let head = line.split_whitespace().next()?.to_ascii_lowercase();

    match verbs::parse(line) {
        Some(Ok(verb)) => return Some(Dispatch::Pagify(verb)),
        Some(Err(problem)) => return Some(Dispatch::Bad(problem)),
        None => {}
    }

    if let Some(refusal) = verbs::REFUSED.iter().find(|r| r.token == head) {
        return Some(Dispatch::Refused { token: head, why: refusal.why });
    }

    match cad_kernel::parser::parse(line) {
        // Pagify opens PDFs and files related to them. It does not open
        // drawings — `.dxf` and `.rsm` are not formats this program handles,
        // and §10 rules out a drawing document entirely.
        //
        // Today these are unreachable anyway, because `open` / `save` /
        // `saveas` are claimed by Pagify's own table first. This arm is here so
        // that stays true if the table is ever reordered or an override is
        // removed: the guarantee should not rest on the order two lookups
        // happen in. `no_route_through_the_box_can_reach_a_drawing_file` in
        // tests/command_box.rs holds it.
        Ok(cad_kernel::parser::Command::Open(_))
        | Ok(cad_kernel::parser::Command::SaveAs(_)) => Some(Dispatch::Refused {
            token: head,
            why: "Pagify opens PDFs, not drawings. There is no .dxf or .rsm path here.",
        }),

        Ok(command) => Some(Dispatch::Kernel(Box::new(command))),

        // The one place this crate reads a *string* from the kernel to decide
        // control flow, and it is worth naming as a seam. `parse` reports an
        // unrecognised verb and a badly-argued one through the same `Err(String)`,
        // and the difference matters: "unknown command" is ours to report in our
        // own words, while "fillet radius must be >= 0" is the kernel's answer
        // and must reach the user verbatim.
        //
        // If SIMLUX ever rewords that message the failure is cosmetic rather
        // than behavioural — an unknown verb would be reported as an argument
        // fault — but `unknown_verbs_are_still_reported_as_unknown` in
        // tests/command_box.rs pins the assumption so the change surfaces as a
        // failing test instead of as a slightly worse error message.
        Err(problem) if problem.starts_with("unknown command") => {
            Some(Dispatch::Unknown(head))
        }
        Err(problem) => Some(Dispatch::Bad(problem)),
    }
}

/// What the box has told you. The top third of the bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// A line the user submitted, echoed back.
    Echo,
    Info,
    Error,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub kind: Kind,
    pub text: String,
}

/// Whether a space is a separator or a character.
///
/// §7: "Space is suppressed while typing text content, where a space is a
/// space." Getting this wrong makes it impossible to type a caption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Command,
    TextContent,
}

/// How a submission was triggered. Kept distinct because they are not
/// interchangeable: space submits only in [`Mode::Command`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submit {
    Enter,
    Space,
    Button,
}

/// What Escape did. §7: it cancels; the plan's phase 1 adds that it returns to
/// the pointer from anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Escaped {
    /// There was typing in progress, and it was discarded.
    ClearedInput,
    /// The box was already empty: cancel the running command, drop the
    /// selection, and go back to the pointer.
    ReturnedToPointer,
}

/// The middle third of the bar: what the box wants right now, always visible,
/// naming the document it will act on.
#[derive(Debug, Clone)]
pub struct Prompt {
    /// The document this will act on. `None` before anything is open — and
    /// saying so is the point, because a command box that does not name its
    /// target is one you cannot safely type into.
    pub document: Option<String>,
    /// What is wanted right now: "type a command", or mid-command, "second
    /// point".
    pub wants: String,
}

impl Default for Prompt {
    fn default() -> Self {
        Prompt { document: None, wants: "type a command".into() }
    }
}

impl Prompt {
    pub fn render(&self) -> String {
        match &self.document {
            Some(name) => format!("{name} › {}", self.wants),
            None => format!("pagify › {}", self.wants),
        }
    }
}

/// How many history lines to keep. Enough to scroll back through a working
/// session; bounded so a replay of ten thousand commands cannot grow without
/// limit.
const HISTORY_LIMIT: usize = 500;

/// The command box.
pub struct CommandBox {
    input: String,
    history: Vec<Entry>,
    /// Lines the user actually submitted, for recall and for repeat.
    submitted: Vec<String>,
    /// Where up/down arrow recall is sitting. `None` means "at the live input".
    recall: Option<usize>,
    prompt: Prompt,
    mode: Mode,
}

impl Default for CommandBox {
    fn default() -> Self {
        CommandBox {
            input: String::new(),
            history: Vec::new(),
            submitted: Vec::new(),
            recall: None,
            prompt: Prompt::default(),
            mode: Mode::Command,
        }
    }
}

impl CommandBox {
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Mutable access for the text widget to write into.
    ///
    /// Deliberately free of side effects. An immediate-mode text widget borrows
    /// its buffer every frame whether or not anything was typed, so resetting
    /// recall here would clear it on the very next frame and make up-arrow
    /// useless. The app calls [`Self::note_edited`] when the widget reports an
    /// actual change instead.
    pub fn input_mut(&mut self) -> &mut String {
        &mut self.input
    }

    /// Tell the box the user actually typed something.
    ///
    /// Typing invalidates where recall was sitting, or the next up-arrow would
    /// jump from a half-edited line to somewhere unrelated.
    pub fn note_edited(&mut self) {
        self.recall = None;
    }

    pub fn history(&self) -> &[Entry] {
        &self.history
    }

    pub fn prompt(&self) -> &Prompt {
        &self.prompt
    }

    pub fn prompt_mut(&mut self) -> &mut Prompt {
        &mut self.prompt
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    /// Whether a space keypress should submit rather than insert.
    pub fn space_submits(&self) -> bool {
        self.mode == Mode::Command
    }

    /// The last line submitted, which Enter on an empty box repeats.
    pub fn last_command(&self) -> Option<&str> {
        self.submitted.last().map(String::as_str)
    }

    /// Append a line to the history. How the app reports back into the box.
    pub fn say(&mut self, kind: Kind, text: impl Into<String>) {
        self.history.push(Entry { kind, text: text.into() });
        if self.history.len() > HISTORY_LIMIT {
            let excess = self.history.len() - HISTORY_LIMIT;
            self.history.drain(..excess);
        }
    }

    /// Report a dispatch the app could not or would not carry out. Kept here so
    /// the wording of a refusal is the same wherever it is triggered from.
    pub fn report(&mut self, dispatch: &Dispatch) {
        match dispatch {
            Dispatch::Refused { token, why } => {
                self.say(Kind::Error, format!("{token} — {why}"));
            }
            Dispatch::Unknown(token) => {
                self.say(Kind::Error, format!("unknown command '{token}'. `help` lists what there is."));
            }
            Dispatch::Bad(problem) => {
                self.say(Kind::Error, problem.clone());
            }
            Dispatch::Pagify(Verb::Planned { verb, phase }) => {
                self.say(Kind::Info, format!("`{verb}` is planned for {phase}, and not built yet."));
            }
            Dispatch::Pagify(Verb::Help(topic)) => {
                for line in verbs::help_text(topic.as_deref()) {
                    self.say(Kind::Info, line);
                }
            }
            _ => {}
        }
    }

    /// Submit the current line.
    ///
    /// Returns what it meant, or `None` when there was nothing to do — a blank
    /// box with no previous command, or a space in text-content mode, where a
    /// space is a space.
    pub fn submit(&mut self, how: Submit) -> Option<Dispatch> {
        if how == Submit::Space && !self.space_submits() {
            return None;
        }

        let line = if self.input.trim().is_empty() {
            // §7: Enter on an empty box repeats the last command. Only Enter —
            // a stray space in an empty box should do nothing at all.
            if how != Submit::Enter {
                return None;
            }
            self.submitted.last()?.clone()
        } else {
            self.input.trim().to_string()
        };

        self.input.clear();
        self.recall = None;
        self.say(Kind::Echo, line.clone());

        let dispatch = dispatch(&line);
        if dispatch.is_some() {
            // Repeated lines collapse: pressing Enter four times to place four
            // circles should leave one entry to scroll past, not four.
            if self.submitted.last().map(String::as_str) != Some(line.as_str()) {
                self.submitted.push(line);
                if self.submitted.len() > HISTORY_LIMIT {
                    let excess = self.submitted.len() - HISTORY_LIMIT;
                    self.submitted.drain(..excess);
                }
            }
        }

        dispatch
    }

    /// Escape. Cancels typing if there is any; otherwise returns to the pointer.
    ///
    /// Two steps rather than one on purpose: a half-typed command and a running
    /// command are different things to be rid of, and collapsing them means
    /// Escape either cannot clear a typo without also dropping your selection,
    /// or cannot drop the selection while anything is typed.
    pub fn escape(&mut self) -> Escaped {
        self.recall = None;
        self.mode = Mode::Command;

        if self.input.is_empty() {
            Escaped::ReturnedToPointer
        } else {
            self.input.clear();
            Escaped::ClearedInput
        }
    }

    /// Walk back through submitted lines. Up arrow.
    pub fn recall_previous(&mut self) {
        if self.submitted.is_empty() {
            return;
        }
        let next = match self.recall {
            None => self.submitted.len() - 1,
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.recall = Some(next);
        self.input = self.submitted[next].clone();
    }

    /// Walk forward again, ending at the empty live input. Down arrow.
    pub fn recall_next(&mut self) {
        let Some(current) = self.recall else { return };

        if current + 1 >= self.submitted.len() {
            self.recall = None;
            self.input.clear();
        } else {
            self.recall = Some(current + 1);
            self.input = self.submitted[current + 1].clone();
        }
    }
}
