//! Deciding whether a line names a person, an organisation or a job.
//!
//! # Why this is a linear model and not trees
//!
//! Part 9 starts at gradient-boosted trees, which is right for a feature vector
//! that is mostly dense geometry. This vector is not: it is five dense values
//! and four thousand sparse n-gram buckets, and on that shape a linear model
//! over log-probabilities is both the standard answer and the one that stays
//! interpretable — every weight is "how much more often this n-gram appears in
//! a person than in a company", which can be read and argued with.
//!
//! It also trains in Rust, in milliseconds, with no hyperparameters to tune.
//! That matters more here than accuracy at the margin: Part 13 worries about
//! features drifting between a Python trainer and a Rust runtime, and the
//! surest way to prevent that is to not have a Python trainer.
//!
//! # What it is deliberately not
//!
//! It is not the arbiter. It labels a line and says how strongly; the caller
//! decides what to do with that. The heuristics still run, and where the two
//! disagree the plan (§11) says take the label and mark the field uncertain —
//! but **not** turn agreement into a confidence number, because with twenty
//! cards §11.3 cannot be measured and an unvalidated confidence signal that
//! looks principled is worse than a visibly crude one.

use super::features::{self, LEXICAL_WIDTH};
use super::labels::{Dataset, LineLabel};
use serde::{Deserialize, Serialize};

/// A trained model: one weight per class per feature, plus a class prior.
///
/// Small on purpose — four classes of 4101 weights is 64 KB as `f32`, which is
/// well inside the megabyte Part 9 budgets and small enough to sit in the
/// binary with `include_bytes!` rather than be downloaded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    /// The feature layout this was trained against. Refuse a mismatch.
    pub feature_version: u32,
    /// `[class][feature]`, as log-probabilities.
    pub weights: Vec<Vec<f32>>,
    /// Log of each class's share of the training set.
    pub priors: Vec<f32>,
}

/// What the model thinks a line is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Verdict {
    pub label: LineLabel,
    /// How far ahead the winner was, in log-odds against the runner-up.
    ///
    /// **Not a probability and deliberately not called one.** Turning this into
    /// a calibrated confidence needs the bucketing §11.3 describes, and twenty
    /// cards cannot bucket anything. A margin is honest about being a margin.
    pub margin: f32,
}

/// Why a model would not load.
#[derive(Debug, PartialEq)]
pub enum LoadError {
    /// Trained against a different feature layout. The numbers would still be
    /// numbers and inference would still succeed — with nonsense.
    WrongFeatureVersion { model: u32, engine: u32 },
    Malformed(&'static str),
}

impl Model {
    /// Check a model over before trusting it.
    pub fn validate(&self) -> Result<(), LoadError> {
        if self.feature_version != features::VERSION {
            return Err(LoadError::WrongFeatureVersion {
                model: self.feature_version,
                engine: features::VERSION,
            });
        }
        if self.weights.len() != LineLabel::ALL.len() || self.priors.len() != LineLabel::ALL.len() {
            return Err(LoadError::Malformed("a class is missing its weights"));
        }
        if self.weights.iter().any(|row| row.len() != LEXICAL_WIDTH) {
            return Err(LoadError::Malformed("a class has the wrong number of weights"));
        }
        Ok(())
    }

    /// What this line is, from its text alone.
    pub fn classify(&self, text: &str) -> Verdict {
        self.score(&features::extract_lexical(text))
    }

    /// The same, when the features are already to hand.
    pub fn score(&self, feature_row: &[f32]) -> Verdict {
        let mut scores: Vec<f32> = self
            .weights
            .iter()
            .zip(self.priors.iter())
            .map(|(row, prior)| {
                prior + row.iter().zip(feature_row).map(|(w, f)| w * f).sum::<f32>()
            })
            .collect();

        let mut best = 0usize;
        for index in 1..scores.len() {
            if scores[index] > scores[best] {
                best = index;
            }
        }
        let winner = scores[best];
        scores[best] = f32::NEG_INFINITY;
        let runner_up = scores.iter().copied().fold(f32::NEG_INFINITY, f32::max);

        Verdict { label: LineLabel::ALL[best], margin: winner - runner_up }
    }

    /// Fit a model to a labelled dataset.
    ///
    /// Multinomial naive Bayes with add-one smoothing, which for a bag of
    /// hashed n-grams is the textbook choice and has the property that matters
    /// most here: it is deterministic. The same dataset gives the same model
    /// every time, so a change in the numbers always means a change in the data
    /// or the features, never a different random seed.
    pub fn train(dataset: &Dataset) -> Model {
        let classes = LineLabel::ALL.len();
        let mut totals = vec![0.0f64; classes];
        let mut sums = vec![vec![0.0f64; LEXICAL_WIDTH]; classes];
        let mut rows = vec![0usize; classes];

        for line in &dataset.lines {
            let class = line.label.column();
            rows[class] += 1;
            for (index, value) in features::extract_lexical(&line.text).iter().enumerate() {
                let value = *value as f64;
                sums[class][index] += value;
                totals[class] += value;
            }
        }

        let total_rows: usize = rows.iter().sum();
        let weights = sums
            .iter()
            .zip(totals.iter())
            .map(|(row, total)| {
                let denominator = total + LEXICAL_WIDTH as f64;
                row.iter().map(|v| (((v + 1.0) / denominator) as f32).ln()).collect()
            })
            .collect();
        let priors = rows
            .iter()
            .map(|count| (((*count as f64 + 1.0) / (total_rows as f64 + classes as f64)) as f32).ln())
            .collect();

        Model { feature_version: features::VERSION, weights, priors }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contacts::labels::LabelledLine;

    fn row(text: &str, label: LineLabel) -> LabelledLine {
        LabelledLine { text: text.into(), label, source: "test".into() }
    }

    fn tiny() -> Dataset {
        Dataset {
            note: "tiny".into(),
            lines: vec![
                row("James Whitfield", LineLabel::Person),
                row("Sarah Connolly", LineLabel::Person),
                row("Michael Brennan", LineLabel::Person),
                row("Northwind Traders Ltd", LineLabel::Organisation),
                row("Contoso Electronics", LineLabel::Organisation),
                row("Fabrikam Industries", LineLabel::Organisation),
                row("Marketing Manager", LineLabel::Role),
                row("Sales Director", LineLabel::Role),
                row("Head of Purchasing", LineLabel::Role),
                row("PO Box 1234 Dubai", LineLabel::Other),
                row("ISO9001:2015", LineLabel::Other),
                row("www.example.com", LineLabel::Other),
            ],
        }
    }

    /// A model trained twice on the same rows is the same model.
    ///
    /// The point of naive Bayes here rather than anything fitted iteratively:
    /// a change in the weights always means a change in the data or the
    /// features, never a different seed.
    #[test]
    fn training_is_deterministic() {
        let a = Model::train(&tiny());
        let b = Model::train(&tiny());
        assert_eq!(a.priors, b.priors);
        assert_eq!(a.weights, b.weights);
    }

    /// A model trained against another feature layout is refused, not run.
    ///
    /// This is the failure with no symptom: every weight still multiplies a
    /// number, inference still returns a label, and the label is nonsense.
    #[test]
    fn a_model_from_another_feature_version_will_not_load() {
        let mut model = Model::train(&tiny());
        assert_eq!(model.validate(), Ok(()));

        model.feature_version = features::VERSION + 1;
        assert_eq!(
            model.validate(),
            Err(LoadError::WrongFeatureVersion {
                model: features::VERSION + 1,
                engine: features::VERSION,
            }),
        );
    }

    /// A truncated weight row is refused too.
    #[test]
    fn a_model_missing_weights_will_not_load() {
        let mut model = Model::train(&tiny());
        model.weights[0].pop();
        assert!(matches!(model.validate(), Err(LoadError::Malformed(_))));
    }

    /// It learns something, on rows it has seen.
    ///
    /// A floor rather than a claim: a model that cannot label its own training
    /// data is broken, and this catches that without pretending to measure
    /// accuracy, which only held-out cards can do.
    #[test]
    fn the_model_labels_its_own_training_rows() {
        let model = Model::train(&tiny());
        assert_eq!(model.classify("James Whitfield").label, LineLabel::Person);
        assert_eq!(model.classify("Contoso Electronics").label, LineLabel::Organisation);
        assert_eq!(model.classify("Sales Director").label, LineLabel::Role);
    }

    /// The margin is a margin, not a probability.
    #[test]
    fn the_margin_is_never_negative() {
        let model = Model::train(&tiny());
        for text in ["James Whitfield", "Contoso Electronics", "ISO9001:2015", "qqqq"] {
            assert!(model.classify(text).margin >= 0.0, "{text} produced a negative margin");
        }
    }

    /// **The model is good on clean text and no use on what the pipeline
    /// actually produces.** Measured, and the reason to read the end-to-end
    /// number rather than the accuracy one.
    ///
    /// Trained on 319 hand-written rows it scores 38 of 43 on real strings
    /// lifted off the photographed cards — including `HSI LIGHTING`, the case
    /// the whole plan is built on. Wired into the parser and asked to pick the
    /// name from the lines the parser actually assembles, it takes the name
    /// field from 7 of 18 cards to 5 of 18. It makes things worse.
    ///
    /// The gap is the input. Every training row is a clean string, and on the
    /// noise a real card carries — a logo read as letters, two social glyphs,
    /// the colour words inside a graphic — the model is confidently wrong
    /// rather than unsure, which is the one failure mode Part 1 says costs more
    /// than being wrong.
    ///
    /// # It is not line assembly, and an earlier version of this note said it was
    ///
    /// That claim came from a corpus whose x coordinates had been silently
    /// zeroed by the script that built it. With every box at x=0 everything on
    /// a row merged, so a third of the lines did indeed arrive fused, and the
    /// conclusion followed. On the real coordinates **the right name is present
    /// as a clean assembled line on 14 of the 18 cards.** Line assembly is not
    /// the bottleneck and fixing it would not move this.
    ///
    /// # What the ceiling actually is
    ///
    /// | approach | name correct |
    /// |---|---|
    /// | available as a clean line | 14 / 18 |
    /// | the rules alone | 7 / 18 |
    /// | this model alone | 5 / 18 |
    /// | both, best arbitration found | 8 / 18 |
    ///
    /// The 8 is not a result. It moves 7, 7, 8, 7 as the confidence gate slides,
    /// which on eighteen cards is noise — and it was reached by trying policies
    /// against the same eighteen cards that measure them. Adding all-capitals
    /// names to the training set, an obvious repair for two specific failures,
    /// made the whole thing worse: the vetoes fell from seven to four while the
    /// losses only fell from two to one.
    ///
    /// **So this is parked rather than tuned further.** Every adjustment swings
    /// the result by three or four cards out of eighteen, using training data
    /// that was written by hand. That is fitting noise with invented data, and
    /// no amount of it reaches the six cards between 8 and 14. What would is
    /// real name and company corpora, which is a sourcing task rather than a
    /// modelling one.
    ///
    /// # The one thing it is good at
    ///
    /// Rejecting. Asked only whether the *rules'* answer is a person, it vetoes
    /// seven wrong names at the cost of two right ones. By Part 1's own value —
    /// a confidently wrong field is worse than a flagged one, because the wrong
    /// one gets accepted and the blank one gets filled in — that is a net gain
    /// of five cards. It is a real trade rather than a free win, and it has not
    /// been shipped.
    #[test]
    #[ignore = "records a measured negative: clean-text accuracy does not survive real lines"]
    fn the_model_is_not_fooled_by_what_the_recogniser_actually_returns() {
        let dataset: Dataset =
            serde_json::from_str(include_str!("../../data/lexical_training.json")).unwrap();
        let model = Model::train(&dataset);

        // Real strings off the cards, chosen because they carry nobody's
        // personal data: two social-media glyphs the recogniser rendered as
        // letters, and the colour words printed inside a logo graphic. None is
        // a person and none is a company.
        for noise in ["fO in Regular Member", "Hue", "Intensity"] {
            let verdict = model.classify(noise);
            assert_eq!(
                verdict.label,
                LineLabel::Other,
                "{noise:?} was read as {:?} at margin {:.2}",
                verdict.label,
                verdict.margin,
            );
        }
    }

    /// A model is a file, and the file has to survive the round trip.
    #[test]
    fn a_model_round_trips_through_json() {
        let model = Model::train(&tiny());
        let encoded = serde_json::to_string(&model).unwrap();
        let back: Model = serde_json::from_str(&encoded).unwrap();
        assert_eq!(back.validate(), Ok(()));
        assert_eq!(back.classify("Northwind Traders Ltd").label, LineLabel::Organisation);
    }
}
