//! What is in a STEP file, decided before any geometry is built.
//!
//! This runs on the raw bytes and needs no parser. That is the point: the two
//! gates have to answer "can this be shown at all" cheaply enough to run on
//! opening a file, and refusing a 14 MB assembly should not first spend a
//! second building a model of it.
//!
//! # What the numbers here are, and are not
//!
//! Every threshold and every judgement in this file came from auditing **41
//! real production STEP files** — supplier heatsinks, enclosures, frames and
//! front plates, not tutorial exports. What that audit found is recorded in the
//! tests, including the two places it contradicted the plan it was checking.

/// The most faces this will attempt.
///
/// **Provisional, and the audit could not settle it.** Of 41 real files only
/// two exceeded 4,000 faces — at 6,947 and 7,981 — and the largest below was
/// 1,689. Any threshold between 1,690 and 6,900 behaves identically on every
/// file measured, so this number is not evidence, it is a placeholder that
/// happens to sit in the gap. See [`tests::the_face_limit_is_not_settled_by_the_audit`].
pub const MAX_FACES: usize = 4000;

/// Above this share of freeform faces, a part is refused rather than shown with
/// holes in it.
///
/// **Measured, unlike [`MAX_FACES`].** Across the 21 audited files containing
/// freeform surfaces the share of faces they account for is a mean of 10% and a
/// maximum of 25%; eleven files are under 10% and two supplier heatsinks are
/// 0.9% — four fillets out of 465 faces. Refusing every one of those, which is
/// what "any occurrence" would do, discards parts that are 90–99% renderable.
///
/// A third missing is a different matter: at that point the holes stop reading
/// as fillets and start reading as the shape. A quarter is the most any real
/// file needed, so a third leaves room without reaching "unrecognisable".
/// Above this share of freeform faces a part *used* to be refused.
///
/// **Kept as a record, no longer a gate.** It was the right call while
/// freeform surfaces could not be drawn: a part is recognisable missing 25%
/// of its faces and is not missing 76%, and the two turbine files proved it
/// — rendered past the gate they came out as a bare hub, because on an
/// impeller the blades *are* the freeform surfaces.
///
/// Now that those surfaces are tessellated, refusing on this would refuse
/// files that draw perfectly. The number stays because the reasoning behind
/// it was measured and a later surface type may need it again.
pub const MAX_FREEFORM_SHARE: f64 = 0.33;

/// What a file contains, counted without parsing it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Census {
    pub faces: usize,
    pub planes: usize,
    pub cylinders: usize,
    pub cones: usize,
    pub tori: usize,
    pub spheres: usize,
    pub freeform_surfaces: usize,
    pub freeform_curves: usize,
    pub extrusions: usize,
    pub revolutions: usize,
    pub triangulated: usize,
    pub pcurves: usize,
    pub assembly_links: usize,
}

impl Census {
    /// The share of faces that sit on a surface this cannot tessellate.
    pub fn freeform_share(&self) -> f64 {
        if self.faces == 0 {
            0.0
        } else {
            self.freeform_surfaces as f64 / self.faces as f64
        }
    }
}

/// Why a file cannot be shown, in the words the user sees.
#[derive(Debug, Clone, PartialEq)]
pub enum Refusal {
    /// Too much geometry. Says the size, because a number is honest where
    /// "too detailed" alone invites the question.
    TooManyFaces { faces: usize },
    /// Mostly freeform. Distinct from the above: a different cause needs a
    /// different sentence, or the message teaches the user nothing.
    MostlyFreeform { share: f64 },
    /// Nothing recognisable in it at all.
    NoSolid,
}

impl Refusal {
    /// **No product that does not exist is named here.** Pointing somebody at a
    /// desktop version they do not have is not help, it is a brush-off; saying
    /// what the file is lets them decide what to do about it.
    pub fn message(&self) -> String {
        match self {
            Refusal::TooManyFaces { faces } => format!(
                "This model has {faces} faces. Pagify 3D shows parts under {MAX_FACES}.",
            ),
            Refusal::MostlyFreeform { share } => format!(
                "{}% of this model uses freeform surfaces, which Pagify 3D can't show yet.",
                (share * 100.0).round() as i64,
            ),
            Refusal::NoSolid => "There is no solid shape in this file.".to_string(),
        }
    }
}

/// Count entity instances in a STEP file's text.
///
/// Instances, not lines. A STEP file may put its whole model on one line, and
/// counting matching lines would report a single face for a part with eight
/// thousand — which is a gate that never fires.
pub fn census(text: &str) -> Census {
    Census {
        faces: count(text, "ADVANCED_FACE"),
        planes: count(text, "PLANE"),
        cylinders: count(text, "CYLINDRICAL_SURFACE"),
        cones: count(text, "CONICAL_SURFACE"),
        tori: count(text, "TOROIDAL_SURFACE"),
        spheres: count(text, "SPHERICAL_SURFACE"),
        // The `_WITH_KNOTS` form specifically. A rational B-spline is written as
        // a complex instance in which `B_SPLINE_SURFACE(` and
        // `B_SPLINE_SURFACE_WITH_KNOTS(` both appear for the *same* entity, so
        // counting the shorter name counts many of them twice. That over-count
        // was in the first pass of the audit and had to be corrected.
        freeform_surfaces: count(text, "B_SPLINE_SURFACE_WITH_KNOTS"),
        freeform_curves: count(text, "B_SPLINE_CURVE_WITH_KNOTS"),
        extrusions: count(text, "SURFACE_OF_LINEAR_EXTRUSION"),
        revolutions: count(text, "SURFACE_OF_REVOLUTION"),
        triangulated: count(text, "TRIANGULATED_FACE_SET"),
        pcurves: count(text, "PCURVE"),
        assembly_links: count(text, "NEXT_ASSEMBLY_USAGE_OCCURRENCE"),
    }
}

/// Whether a file can be shown, and why not if it cannot.
pub fn verdict(census: &Census) -> Result<(), Refusal> {
    if census.faces == 0 {
        return Err(Refusal::NoSolid);
    }
    if census.faces > MAX_FACES {
        return Err(Refusal::TooManyFaces { faces: census.faces });
    }
    Ok(())
}

/// Occurrences of an entity name used as a constructor.
///
/// The trailing `(` is what separates `PLANE(` from `PLANE_ANGLE_UNIT(`, and
/// the leading check is what stops `CIRCLE` matching inside a longer name.
/// Whitespace between the two is legal and several exporters emit it.
fn count(text: &str, entity: &str) -> usize {
    let bytes = text.as_bytes();
    let mut found = 0;
    let mut at = 0;

    while let Some(index) = text[at..].find(entity) {
        let start = at + index;
        let end = start + entity.len();
        at = end;

        // Not part of a longer identifier on the left.
        let left_ok = start == 0 || {
            let before = bytes[start - 1];
            !before.is_ascii_alphanumeric() && before != b'_'
        };
        if !left_ok {
            continue;
        }

        // A constructor call on the right, allowing space before the bracket.
        let mut cursor = end;
        while cursor < bytes.len() && (bytes[cursor] == b' ' || bytes[cursor] == b'\t') {
            cursor += 1;
        }
        if cursor < bytes.len() && bytes[cursor] == b'(' {
            found += 1;
        }
    }

    found
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- counting -----------------------------------------------------------

    /// Instances, not lines. A whole model on one line is normal STEP.
    #[test]
    fn many_entities_on_one_line_are_all_counted() {
        let one_line = "#1=ADVANCED_FACE('',(#2),#3,.T.);#4=ADVANCED_FACE('',(#5),#6,.F.);";
        assert_eq!(2, census(one_line).faces);
    }

    /// `PLANE(` is a plane; `PLANE_ANGLE_UNIT(` is a unit, and every file has one.
    #[test]
    fn a_longer_name_that_starts_the_same_is_not_a_match() {
        let units = "#1=( NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.) );";
        assert_eq!(0, census(units).planes);

        let real = "#7=PLANE('',#8);";
        assert_eq!(1, census(real).planes);
    }

    /// Several exporters put a space before the bracket. Both are legal.
    #[test]
    fn a_space_before_the_bracket_is_still_a_match() {
        assert_eq!(1, census("#1 = ADVANCED_FACE ( '', (#2), #3, .T. ) ;").faces);
    }

    /// A name appearing inside a longer identifier is not that entity.
    #[test]
    fn a_name_embedded_in_another_is_not_a_match() {
        assert_eq!(0, census("#1=MY_PLANE('',#2);").planes);
    }

    /// The over-count that was in the first pass of the real audit.
    ///
    /// A rational B-spline is one entity written as a complex instance in which
    /// both spellings appear. Counting the short name reports it twice, which
    /// inflates the freeform share and would refuse parts that are fine.
    #[test]
    fn a_rational_bspline_is_one_surface_not_two() {
        let complex = "#9=( BOUNDED_SURFACE() B_SPLINE_SURFACE(3,3,((#10)),.UNSPECIFIED.,.F.,.F.,.F.) \
             B_SPLINE_SURFACE_WITH_KNOTS((4),(4),(0.),(1.),.UNSPECIFIED.) \
             RATIONAL_B_SPLINE_SURFACE(((1.))) );";
        assert_eq!(1, census(complex).freeform_surfaces);
    }

    // ---- the gates ----------------------------------------------------------

    #[test]
    fn an_ordinary_part_is_accepted() {
        let census = Census { faces: 465, freeform_surfaces: 4, ..Default::default() };
        assert_eq!(Ok(()), verdict(&census));
    }

    #[test]
    fn too_much_geometry_is_refused_with_the_number() {
        let census = Census { faces: 7981, ..Default::default() };
        let refusal = verdict(&census).unwrap_err();

        assert_eq!(Refusal::TooManyFaces { faces: 7981 }, refusal);
        assert!(refusal.message().contains("7981"), "{}", refusal.message());
    }

    #[test]
    fn a_mostly_freeform_part_is_no_longer_refused() {
        // The impeller: 132 of 173 faces freeform. It was refused while those
        // surfaces could not be drawn, and drawing it past the gate showed
        // why -- a bare hub. Now that they are tessellated it must open.
        let census = Census { faces: 173, freeform_surfaces: 132, ..Default::default() };
        assert_eq!(Ok(()), verdict(&census));
    }

    /// The two refusals must not share a sentence.
    ///
    /// They are different problems with different answers — one is "simplify the
    /// model", the other is "this shape cannot be drawn at all" — and one
    /// message for both teaches the user nothing about either.
    #[test]
    fn the_two_refusals_read_differently() {
        let big = Refusal::TooManyFaces { faces: 9000 }.message();
        let curvy = Refusal::MostlyFreeform { share: 0.9 }.message();
        assert_ne!(big, curvy);
    }

    /// No message names a product the user may not have.
    #[test]
    fn no_refusal_points_at_something_that_may_not_exist() {
        for refusal in [
            Refusal::TooManyFaces { faces: 9000 },
            Refusal::MostlyFreeform { share: 0.9 },
            Refusal::NoSolid,
        ] {
            let message = refusal.message().to_lowercase();
            for brush_off in ["desktop", "computer", "solidworks", "instead use"] {
                assert!(!message.contains(brush_off), "{}", refusal.message());
            }
        }
    }

    #[test]
    fn a_file_with_no_faces_is_refused_as_having_no_solid() {
        assert_eq!(Err(Refusal::NoSolid), verdict(&Census::default()));
    }

    // ---- what the audit of 41 real files settled ----------------------------

    /// The two supplier heatsinks, at 0.9% freeform, must be shown.
    ///
    /// These are the case that changed the design. The plan being checked said
    /// "any `B_SPLINE_SURFACE_WITH_KNOTS` means refusal"; the audit found the
    /// entity present in 51% of real files and, in every one of them, a
    /// minority of the faces — four fillets out of 465 here. Refusing on
    /// presence throws away a part that is 99% drawable.
    #[test]
    fn a_part_with_a_few_freeform_fillets_is_still_shown() {
        // EURO-HS45XL-118-A.STEP and XOLO-HS45XL-117-A.STEP, as measured.
        for (faces, freeform) in [(465, 4), (425, 4)] {
            let census = Census { faces, freeform_surfaces: freeform, ..Default::default() };
            assert_eq!(Ok(()), verdict(&census), "{faces} faces, {freeform} freeform");
        }
    }

    /// The worst real file, at 25%, is still shown; a third is where it stops.
    #[test]
    fn the_most_freeform_real_part_is_still_inside_the_limit() {
        // VERA MINI FRAME.STEP: 19 freeform of 76 faces.
        let census = Census { faces: 76, freeform_surfaces: 19, ..Default::default() };
        assert!(census.freeform_share() < MAX_FREEFORM_SHARE);
        assert_eq!(Ok(()), verdict(&census));
    }

    /// **An accepted cost, named so nobody thinks it was measured.**
    ///
    /// The audit cannot choose [`MAX_FACES`]. Of 41 real files, two were over
    /// 4,000 (6,947 and 7,981) and the largest under was 1,689 — so every
    /// threshold in that gap sorts the corpus identically and the data prefers
    /// none of them. What settles it is a timing measurement on the phone,
    /// which cannot be taken until the tessellator exists.
    ///
    /// Kept as a passing test rather than an ignored one because it asserts
    /// something true and useful today: that the constant still sits inside the
    /// gap the evidence leaves open. If somebody moves it to 1,000, this fails
    /// and tells them what they are doing.
    #[test]
    fn the_face_limit_is_not_settled_by_the_audit() {
        const LARGEST_ACCEPTED_IN_THE_AUDIT: usize = 1689;
        const SMALLEST_REFUSED_IN_THE_AUDIT: usize = 6947;

        assert!(
            MAX_FACES > LARGEST_ACCEPTED_IN_THE_AUDIT,
            "would refuse a file the audit showed is ordinary",
        );
        assert!(
            MAX_FACES < SMALLEST_REFUSED_IN_THE_AUDIT,
            "would accept a file the audit showed is out of reach",
        );
    }

    /// Real files carry no pcurves, so parameter-space projection is required.
    ///
    /// Recorded as a test because it is an assumption the tessellator is built
    /// on: not one of the 41 files had a single `PCURVE`, so the shortcut of
    /// reading 2D curves out of the file never applies and the analytic
    /// projection path is the only path. If a file ever turns up with them,
    /// this is where to notice that the shortcut became available.
    #[test]
    fn a_file_without_pcurves_is_the_normal_case() {
        let ordinary = "#1=ADVANCED_FACE('',(#2),#3,.T.);#3=PLANE('',#4);";
        assert_eq!(0, census(ordinary).pcurves);
    }
}

/// Checking [`census`] against the files the thresholds came from.
///
/// **Ignored, because the files cannot be committed.** They are supplier and
/// customer CAD — the same rule that keeps business-card text out of this
/// repository keeps these out: the geometry *is* the confidential part, so
/// there is no redacted form of it to commit. What is committed is the
/// measurement, in the tests above.
///
/// Run against a directory of real files with:
///
/// ```text
/// PAGIFY_STEP_DIR="/d/Dropbox" cargo test --lib step::audit::corpus -- --ignored --nocapture
/// ```
#[cfg(test)]
mod corpus {
    use super::*;
    use std::path::PathBuf;

    fn step_files(root: &std::path::Path, into: &mut Vec<PathBuf>, depth: usize) {
        if depth == 0 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(root) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                step_files(&path, into, depth - 1);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("stp") || e.eq_ignore_ascii_case("step"))
            {
                into.push(path);
            }
        }
    }

    /// The counter agrees with an independent count, on real files.
    ///
    /// The shell pass that produced the audit and this function are two
    /// separate implementations of the same question. Agreeing is worth
    /// something; the first pass of the shell version over-counted rational
    /// B-splines, and the only reason that was caught was looking at a file by
    /// hand rather than trusting the number.
    #[test]
    #[ignore = "needs real CAD files, which cannot be committed; set PAGIFY_STEP_DIR"]
    fn the_census_agrees_with_the_audit() {
        let Ok(root) = std::env::var("PAGIFY_STEP_DIR") else {
            panic!("set PAGIFY_STEP_DIR to a directory of real .stp files");
        };

        let mut files = Vec::new();
        step_files(std::path::Path::new(&root), &mut files, 8);
        files.sort();

        let mut seen_sizes = std::collections::HashSet::new();
        let mut counted = 0;
        let mut accepted = 0;
        let mut freeform_shares = Vec::new();

        for path in files {
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let census = census(&text);
            if census.faces == 0 {
                continue; // not a STEP file; the extension is also SystemTap's
            }
            if !seen_sizes.insert(text.len()) {
                continue; // the same model mirrored into a second folder
            }
            counted += 1;
            if verdict(&census).is_ok() {
                accepted += 1;
            }
            if census.freeform_surfaces > 0 {
                freeform_shares.push(census.freeform_share());
            }
            println!(
                "{:>5} faces {:>4} freeform {:>4} freeform-curves  {}",
                census.faces,
                census.freeform_surfaces,
                census.freeform_curves,
                path.file_name().unwrap_or_default().to_string_lossy(),
            );
        }

        let worst = freeform_shares.iter().cloned().fold(0.0_f64, f64::max);
        println!(
            "\n{counted} files, {accepted} accepted ({:.0}%); worst freeform share {:.0}%",
            100.0 * accepted as f64 / counted as f64,
            100.0 * worst,
        );

        assert!(counted >= 20, "the audit asked for at least 20 real files");
        assert!(
            worst < MAX_FREEFORM_SHARE,
            "a real file is more freeform than the limit allows: {worst}",
        );
    }
}
