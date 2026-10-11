//! The Accounting Calculator's tape — the Comment tab's little adding
//! machine.
//!
//! Arithmetic only: the panel that draws the buttons is `panels.rs`, and it
//! presses these keys. **Immediate execution**, like every desk calculator:
//! `2 + 3 × 4 =` gives 20, not 14 — each operator collapses what came
//! before it, and the tape keeps every collapsed step so the arithmetic can
//! be read back afterwards.

/// One keypress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Digit(u8),
    Dot,
    Add,
    Subtract,
    Multiply,
    Divide,
    Equals,
    /// Clear the entry being typed.
    ClearEntry,
    /// Clear everything, tape and all.
    ClearAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Add,
    Subtract,
    Multiply,
    Divide,
}

impl Op {
    fn sign(self) -> &'static str {
        match self {
            Op::Add => "+",
            Op::Subtract => "−",
            Op::Multiply => "×",
            Op::Divide => "÷",
        }
    }
}

/// The tape calculator itself.
#[derive(Default)]
pub struct Calculator {
    /// What is being typed.
    entry: String,
    /// The operand and operator waiting for the next number.
    pending: Option<(f64, Op)>,
    /// The last answer — shown when nothing is being typed.
    shown: f64,
    tape: Vec<String>,
    total: f64,
    /// Set by a division by zero; the next keypress starts over.
    error: Option<String>,
}

impl Calculator {
    /// The display: what is being typed, or the last answer, or the error.
    pub fn display(&self) -> String {
        if let Some(error) = &self.error {
            return error.clone();
        }
        if !self.entry.is_empty() {
            return self.entry.clone();
        }
        format_number(self.shown)
    }

    /// The tape, oldest first.
    pub fn tape(&self) -> &[String] {
        &self.tape
    }

    /// The grand total: the sum of everything the equals key produced, which
    /// is the number an accountant's tape is read for.
    pub fn total(&self) -> f64 {
        self.total
    }

    pub fn press(&mut self, key: Key) {
        // Any key after an error starts over, the way a calculator's clear
        // does — the error is on the display, not in the state.
        if self.error.is_some() {
            self.error = None;
            self.entry.clear();
            self.pending = None;
            if key == Key::ClearAll {
                *self = Self::default();
                return;
            }
        }
        match key {
            Key::Digit(digit) => {
                if self.entry.len() < 16 {
                    self.entry.push((b'0' + digit.min(9)) as char);
                }
            }
            Key::Dot => {
                if !self.entry.contains('.') {
                    if self.entry.is_empty() {
                        self.entry.push('0');
                    }
                    self.entry.push('.');
                }
            }
            Key::ClearEntry => self.entry.clear(),
            Key::ClearAll => *self = Self::default(),
            Key::Add | Key::Subtract | Key::Multiply | Key::Divide => {
                let op = match key {
                    Key::Add => Op::Add,
                    Key::Subtract => Op::Subtract,
                    Key::Multiply => Op::Multiply,
                    _ => Op::Divide,
                };
                let typed = !self.entry.is_empty();
                let value = self.take_entry().unwrap_or(self.shown);
                match self.pending.take() {
                    Some((left, previous)) if typed => {
                        // A second operator collapses what came before it and
                        // keeps the result, so the tape reads left to right.
                        let run = self.compute(left, previous, value);
                        if self.error.is_none() {
                            self.tape.push(line(previous, left, value, run));
                            self.pending = Some((run, op));
                        }
                    }
                    Some((left, _)) => self.pending = Some((left, op)),
                    None => self.pending = Some((value, op)),
                }
                self.entry.clear();
            }
            Key::Equals => {
                match (self.pending.take(), self.take_entry()) {
                    (Some((left, op)), Some(value)) => {
                        let answer = self.compute(left, op, value);
                        if self.error.is_none() {
                            self.tape.push(line(op, left, value, answer));
                            self.total += answer;
                        }
                    }
                    (None, Some(value)) => self.shown = value,
                    // An operator with no number after it: nothing to do.
                    (Some(pair), None) => self.pending = Some(pair),
                    (None, None) => {}
                }
                self.entry.clear();
            }
        }
    }

    /// The number just typed, consumed — `None` when nothing was typed.
    fn take_entry(&mut self) -> Option<f64> {
        if self.entry.is_empty() {
            return None;
        }
        match self.entry.parse::<f64>() {
            Ok(value) => {
                self.entry.clear();
                Some(value)
            }
            Err(_) => {
                self.error = Some("not a number".into());
                None
            }
        }
    }

    fn compute(&mut self, left: f64, op: Op, right: f64) -> f64 {
        let value = match op {
            Op::Add => left + right,
            Op::Subtract => left - right,
            Op::Multiply => left * right,
            Op::Divide => {
                if right == 0.0 {
                    self.error = Some("cannot divide by zero".into());
                    left
                } else {
                    left / right
                }
            }
        };
        if self.error.is_none() {
            self.shown = value;
        }
        value
    }
}

fn line(op: Op, left: f64, right: f64, answer: f64) -> String {
    format!("{} {} {} = {}", format_number(left), op.sign(), format_number(right), format_number(answer))
}

/// Whole numbers without a tail of zeros; everything else as the shortest
/// float that reads back.
fn format_number(value: f64) -> String {
    if value.is_finite() && (value - value.round()).abs() < f64::EPSILON && value.abs() < 1e15 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press_all(keys: &[Key]) -> Calculator {
        let mut calc = Calculator::default();
        for key in keys {
            calc.press(*key);
        }
        calc
    }

    #[test]
    fn an_addition_tapes_a_line_and_totals_it() {
        let calc = press_all(&[
            Key::Digit(2),
            Key::Add,
            Key::Digit(3),
            Key::Equals,
        ]);

        assert_eq!(calc.display(), "5");
        assert_eq!(calc.tape(), ["2 + 3 = 5"]);
        assert_eq!(calc.total(), 5.0);
    }

    #[test]
    fn operators_collapse_left_to_right_like_a_desk_calculator() {
        // 2 + 3 × 4 = is 20, not 14: the + collapses when × is pressed.
        let calc = press_all(&[
            Key::Digit(2),
            Key::Add,
            Key::Digit(3),
            Key::Multiply,
            Key::Digit(4),
            Key::Equals,
        ]);

        assert_eq!(calc.display(), "20");
        assert_eq!(calc.tape(), ["2 + 3 = 5", "5 × 4 = 20"]);
        assert_eq!(
            calc.total(),
            20.0,
            "the total sums every equals result; a collapse step is an intermediate, not a calculation"
        );
    }

    #[test]
    fn a_decimal_entry_is_one_number() {
        let calc = press_all(&[
            Key::Digit(1),
            Key::Dot,
            Key::Digit(5),
            Key::Add,
            Key::Dot,
            Key::Digit(5),
            Key::Equals,
        ]);

        assert_eq!(calc.display(), "2");
        assert_eq!(calc.tape(), ["1.5 + 0.5 = 2"]);
    }

    #[test]
    fn dividing_by_zero_says_so_and_the_next_key_starts_over() {
        let mut calc = press_all(&[Key::Digit(5), Key::Divide, Key::Digit(0), Key::Equals]);
        assert_eq!(calc.display(), "cannot divide by zero");
        assert!(calc.tape().is_empty(), "an error tapes nothing");

        calc.press(Key::Digit(7));
        assert_eq!(calc.display(), "7");
    }

    #[test]
    fn clearing_the_entry_keeps_the_tape_and_clear_all_does_not() {
        let mut calc = press_all(&[Key::Digit(2), Key::Add, Key::Digit(3), Key::Equals]);
        calc.press(Key::Digit(9));
        calc.press(Key::ClearEntry);
        assert_eq!(calc.display(), "5", "clear entry returns to the answer");
        assert_eq!(calc.tape().len(), 1, "clear entry keeps the tape");

        calc.press(Key::ClearAll);
        assert_eq!(calc.display(), "0");
        assert!(calc.tape().is_empty());
        assert_eq!(calc.total(), 0.0);
    }
}
