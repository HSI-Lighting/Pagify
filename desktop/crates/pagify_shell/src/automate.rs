//! Automate — build plan phase 11.
//!
//! "Record a sequence of commands, save it, replay it against another document
//! or a folder. Cheap because the engine's commands were built serialisable for
//! exactly this."
//!
//! What gets recorded here is **command lines**, not engine commands, and the
//! difference matters. §7's whole claim is that the command box *is* the
//! interface — every ribbon button runs a command string, so anything clickable
//! is typeable and anything typeable is scriptable. A script of lines is
//! therefore a script of everything the program can do, readable and editable
//! by hand. A script of `pdf_core::command::Command` would cover only the
//! document mutations and none of the drawing, and could not be written by a
//! person in a text editor.

use serde::{Deserialize, Serialize};

pub const SCRIPT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Script {
    pub version: u32,
    #[serde(default)]
    pub name: String,
    pub steps: Vec<String>,
}

impl Script {
    pub fn new(name: impl Into<String>) -> Self {
        Script { version: SCRIPT_VERSION, name: name.into(), steps: Vec::new() }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a Script always serialises")
    }

    pub fn from_json(text: &str) -> Result<Script, String> {
        let script: Script =
            serde_json::from_str(text).map_err(|e| format!("not a Pagify script: {e}"))?;
        if script.version > SCRIPT_VERSION {
            return Err(format!(
                "this script was written by a newer Pagify (version {}, this build runs {})",
                script.version, SCRIPT_VERSION
            ));
        }
        Ok(script)
    }
}

/// Records command lines as they are submitted.
#[derive(Debug, Default)]
pub struct Recorder {
    script: Option<Script>,
}

/// Lines that must never be recorded.
///
/// A script that opens the document it was recorded against, and then closes
/// it, is a script that does nothing to the document you actually pointed it
/// at — and `quit` in the middle of a folder run stops the run. These are
/// dropped with the recorder still running rather than refused, because
/// stopping to argue mid-recording is worse than quietly doing the sane thing.
fn is_recordable(line: &str) -> bool {
    let head = line
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    !matches!(head.as_str(), "open" | "close" | "quit" | "exit" | "record" | "replay" | "help")
}

impl Recorder {
    pub fn is_recording(&self) -> bool {
        self.script.is_some()
    }

    pub fn start(&mut self, name: impl Into<String>) {
        self.script = Some(Script::new(name));
    }

    /// Offer a submitted line. Returns whether it was kept.
    pub fn observe(&mut self, line: &str) -> bool {
        let line = line.trim();
        let Some(script) = self.script.as_mut() else { return false };
        if line.is_empty() || !is_recordable(line) {
            return false;
        }
        script.steps.push(line.to_string());
        true
    }

    pub fn steps(&self) -> usize {
        self.script.as_ref().map(|s| s.steps.len()).unwrap_or(0)
    }

    /// Stop, and hand back what was recorded.
    pub fn finish(&mut self) -> Option<Script> {
        self.script.take()
    }

    pub fn cancel(&mut self) {
        self.script = None;
    }
}

/// What a replay did, step by step, so a failure names the line it failed on.
#[derive(Debug, Clone, PartialEq)]
pub struct Replayed {
    pub ran: usize,
    /// The first step that failed, as (one-based step number, line, why).
    pub failed: Option<(usize, String, String)>,
}

impl Replayed {
    pub fn ok(&self) -> bool {
        self.failed.is_none()
    }

    pub fn render(&self) -> String {
        match &self.failed {
            None => format!("replayed {} step{}.", self.ran, if self.ran == 1 { "" } else { "s" }),
            Some((step, line, why)) => format!(
                "stopped at step {step} (`{line}`): {why}. {} step{} ran before it.",
                self.ran,
                if self.ran == 1 { "" } else { "s" }
            ),
        }
    }
}

/// Replay a script, stopping at the first step that fails.
///
/// Stopping rather than continuing is deliberate. A script is a sequence, and
/// step 7 usually assumes steps 1 to 6 happened; carrying on after a failure
/// applies the rest of the script to a document in a state it was never written
/// for, which is how a batch run quietly ruins a folder full of files.
pub fn replay<F>(script: &Script, mut run: F) -> Replayed
where
    F: FnMut(&str) -> Result<(), String>,
{
    let mut ran = 0;
    for (index, line) in script.steps.iter().enumerate() {
        match run(line) {
            Ok(()) => ran += 1,
            Err(why) => {
                return Replayed { ran, failed: Some((index + 1, line.clone(), why)) };
            }
        }
    }
    Replayed { ran, failed: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recorder_keeps_what_was_typed_in_order() {
        let mut recorder = Recorder::default();
        assert!(!recorder.is_recording());
        assert!(!recorder.observe("zoom fit"), "nothing is kept before recording starts");

        recorder.start("stamp every drawing");
        recorder.observe("zoom fit");
        recorder.observe("l 0,0 100,100");

        let script = recorder.finish().expect("a script");
        assert_eq!(script.steps, vec!["zoom fit", "l 0,0 100,100"]);
        assert_eq!(script.name, "stamp every drawing");
        assert!(!recorder.is_recording());
    }

    #[test]
    fn lines_that_would_wreck_a_replay_are_not_recorded() {
        // A script that opens the file it was recorded against does nothing to
        // the file you pointed it at. `quit` mid-folder stops the run.
        let mut recorder = Recorder::default();
        recorder.start("test");

        for line in ["open a.pdf", "close", "quit", "exit", "record x", "replay y", "help"] {
            assert!(!recorder.observe(line), "`{line}` should not be recorded");
        }
        recorder.observe("rotate 90");

        assert_eq!(recorder.finish().unwrap().steps, vec!["rotate 90"]);
    }

    #[test]
    fn blank_lines_are_not_steps() {
        let mut recorder = Recorder::default();
        recorder.start("test");
        assert!(!recorder.observe("   "));
        assert_eq!(recorder.steps(), 0);
    }

    #[test]
    fn a_script_survives_being_written_down() {
        let mut script = Script::new("nightly");
        script.steps = vec!["rotate 90".into(), "zoom fit".into()];

        let json = script.to_json();
        assert_eq!(Script::from_json(&json).unwrap(), script);
    }

    #[test]
    fn a_newer_script_is_refused_rather_than_half_understood() {
        let json = r#"{"version":99,"name":"x","steps":[]}"#;
        assert!(Script::from_json(json).unwrap_err().contains("newer"));
        assert!(Script::from_json("nonsense").is_err());
    }

    #[test]
    fn replay_runs_every_step_in_order() {
        let mut script = Script::new("t");
        script.steps = vec!["a".into(), "b".into(), "c".into()];

        let mut seen = Vec::new();
        let result = replay(&script, |line| {
            seen.push(line.to_string());
            Ok(())
        });

        assert!(result.ok());
        assert_eq!(result.ran, 3);
        assert_eq!(seen, vec!["a", "b", "c"]);
    }

    #[test]
    fn replay_stops_at_the_first_failure_and_names_it() {
        // Carrying on would apply the rest of the script to a document in a
        // state it was never written for.
        let mut script = Script::new("t");
        script.steps = vec!["a".into(), "boom".into(), "c".into()];

        let mut seen = Vec::new();
        let result = replay(&script, |line| {
            seen.push(line.to_string());
            if line == "boom" { Err("no such thing".into()) } else { Ok(()) }
        });

        assert!(!result.ok());
        assert_eq!(result.ran, 1);
        assert_eq!(seen, vec!["a", "boom"], "step 3 must not have run");

        assert!(result.render().contains("step 2"));
        let (step, line, why) = result.failed.clone().unwrap();
        assert_eq!(step, 2);
        assert_eq!(line, "boom");
        assert_eq!(why, "no such thing");
    }
}
