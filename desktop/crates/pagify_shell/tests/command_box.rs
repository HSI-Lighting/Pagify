//! The command box, tested without a window.
//!
//! Build plan §4: "The split is not ceremony. It is what makes the command box
//! testable without a window, which matters enormously for an app whose entire
//! interface is a command box." This file is the cash value of that claim.

use pagify_shell::command::{dispatch, CommandBox, Dispatch, Escaped, Kind, Mode, Submit};
use pagify_shell::verbs::{self, PageTarget, Verb, ZoomTarget};

fn head(line: &str) -> Dispatch {
    dispatch(line).expect("a non-blank line always dispatches")
}

// ---------------------------------------------------------------------------
// The two namespaces
// ---------------------------------------------------------------------------

/// The load-bearing test of §7's promise that "the drawing aliases [stay]
/// identical to SIMLUX, which matters for anyone who uses both".
///
/// Pagify's table is tried first, so any token it claims that the kernel also
/// claims is *shadowed* — silently, and with no way to notice from either side.
/// This asserts that every such collision is a decision someone wrote down.
///
/// When SIMLUX adds a verb that collides with one of ours, this fails. That is
/// the point: the fix is to rename ours or to add an entry explaining why the
/// shadowing is right, and either way a person decides rather than a merge.
#[test]
fn every_collision_with_the_kernel_is_deliberate() {
    let mut undeclared = Vec::new();

    for token in verbs::claimed_tokens() {
        let kernel_claims = match cad_kernel::parser::parse(token) {
            Ok(_) => true,
            // The kernel recognised the verb and wanted arguments — it claims it.
            Err(problem) => !problem.starts_with("unknown command"),
        };

        let declared = verbs::DELIBERATE_OVERRIDES.iter().any(|o| o.token == token);

        if kernel_claims && !declared {
            undeclared.push(token);
        }
    }

    assert!(
        undeclared.is_empty(),
        "these Pagify verbs shadow cad_kernel verbs without an entry in \
         DELIBERATE_OVERRIDES: {undeclared:?}.\n\
         Either rename them, or add an entry saying what the kernel means by \
         the word and why Pagify overrides it."
    );
}

/// Every declared override must actually still collide. An entry that no longer
/// describes a real collision is stale documentation, and stale documentation
/// about which of two programs owns a word is worse than none.
#[test]
fn no_declared_override_is_stale() {
    for o in verbs::DELIBERATE_OVERRIDES {
        let kernel_claims = match cad_kernel::parser::parse(o.token) {
            Ok(_) => true,
            Err(problem) => !problem.starts_with("unknown command"),
        };
        assert!(
            kernel_claims,
            "`{}` is listed as a deliberate override but the kernel no longer \
             claims it — delete the entry",
            o.token
        );
    }
}

/// The escape hatch each override promises must exist. If `rotate` says
/// selection-rotate is "still on `ro`", `ro` had better still reach the kernel.
#[test]
fn the_aliases_an_override_leaves_behind_still_reach_the_kernel() {
    for o in verbs::DELIBERATE_OVERRIDES {
        let Some(alias) = o.kernel_keeps else { continue };
        assert!(
            matches!(head(alias), Dispatch::Kernel(_)),
            "`{}` promises the kernel command is still on `{alias}`, but `{alias}` \
             does not dispatch to the kernel",
            o.token
        );
    }
}

#[test]
fn pagify_verbs_win_over_the_kernel() {
    assert!(matches!(head("open report.pdf"), Dispatch::Pagify(Verb::Open(_))));
    assert!(matches!(head("save"), Dispatch::Pagify(Verb::Save)));
    assert!(matches!(head("rotate 90"), Dispatch::Pagify(Verb::RotatePage(90))));
    assert!(matches!(head("undo"), Dispatch::Pagify(Verb::Undo)));
}

#[test]
fn the_drawing_verbs_keep_their_simlux_aliases() {
    // §7's own examples, plus the aliases the overrides promise to leave behind.
    //
    // The property is that the word still *belongs* to the kernel — not that a
    // bare alias parses to a command. Several kernel verbs legitimately want
    // arguments (`po` wants a point, and answering "usage: point x,y" in the
    // kernel's own words is correct). What must never happen is Pagify claiming
    // the word, refusing it, or calling it unknown.
    for alias in ["l", "pl", "tr", "f", "o", "ci", "el", "po", "ro", "u", "y", "?"] {
        assert!(
            verbs::parse(alias).is_none(),
            "Pagify's table has started claiming `{alias}`, which SIMLUX users \
             expect to be a drawing command"
        );

        match head(alias) {
            Dispatch::Kernel(_) | Dispatch::Bad(_) => {}
            other => panic!("`{alias}` no longer belongs to the kernel: {other:?}"),
        }
    }

    // And fully-argued forms arrive as real geometry.
    for line in ["l 0,0 10,10", "ci 5,5 2", "po 3,4"] {
        assert!(
            matches!(head(line), Dispatch::Kernel(_)),
            "`{line}` should have reached the kernel as a command"
        );
    }
}

#[test]
fn cad_only_verbs_are_refused_with_a_reason_not_run() {
    for token in ["wall", "w", "wallstyle", "blockdiff", "units", "scene"] {
        match head(token) {
            Dispatch::Refused { why, .. } => {
                assert!(!why.is_empty(), "`{token}` refused with no reason");
            }
            other => panic!("`{token}` should have been refused, got {other:?}"),
        }
    }
}

/// A refused verb must be one the kernel would otherwise have run — otherwise
/// the refusal list is guarding nothing and should be deleted.
#[test]
fn every_refusal_guards_a_verb_the_kernel_really_has() {
    for r in verbs::REFUSED {
        let kernel_claims = match cad_kernel::parser::parse(r.token) {
            Ok(_) => true,
            Err(problem) => !problem.starts_with("unknown command"),
        };
        assert!(
            kernel_claims,
            "`{}` is on the refusal list but the kernel does not claim it",
            r.token
        );
    }
}

// ---------------------------------------------------------------------------
// Telling "I don't know that word" from "I know it, but not like that"
// ---------------------------------------------------------------------------

#[test]
fn unknown_verbs_are_still_reported_as_unknown() {
    // Also the pin for the one string assumption this crate makes about the
    // kernel — see the comment in command.rs. If SIMLUX rewords its
    // "unknown command" error, this fails rather than silently degrading every
    // unknown verb into an argument fault.
    match head("frobnicate") {
        Dispatch::Unknown(token) => assert_eq!(token, "frobnicate"),
        other => panic!("expected Unknown, got {other:?}"),
    }
}

#[test]
fn a_pagify_verb_with_bad_arguments_is_not_reported_as_unknown() {
    // The case that makes the three-way return in verbs::parse necessary. If
    // `page` fell through to the kernel on an argument fault, the answer would
    // be "unknown command 'page'", which is a lie about a word we own.
    match head("page banana") {
        Dispatch::Bad(problem) => {
            assert!(problem.contains("page"), "unhelpful message: {problem}");
            assert!(!problem.contains("unknown command"), "reported as unknown: {problem}");
        }
        other => panic!("expected Bad, got {other:?}"),
    }
}

#[test]
fn a_kernel_verb_with_bad_arguments_answers_in_the_kernels_own_words() {
    // The kernel knows why a fillet radius is wrong and we do not. Its message
    // must arrive verbatim rather than being flattened into something of ours.
    match head("fillet -1") {
        Dispatch::Bad(problem) => assert!(
            problem.contains("radius"),
            "expected the kernel's own wording, got: {problem}"
        ),
        other => panic!("expected Bad, got {other:?}"),
    }
}

#[test]
fn blank_lines_are_not_errors() {
    assert!(dispatch("").is_none());
    assert!(dispatch("   ").is_none());
    assert!(dispatch("\t \n").is_none());
}

// ---------------------------------------------------------------------------
// Pagify's own arguments
// ---------------------------------------------------------------------------

#[test]
fn page_and_zoom_take_the_words_a_reader_would_reach_for() {
    assert_eq!(head("page 4").pagify(), Some(Verb::Page(PageTarget::Number(4))));
    assert_eq!(head("page next").pagify(), Some(Verb::Page(PageTarget::Next)));
    assert_eq!(head("page last").pagify(), Some(Verb::Page(PageTarget::Last)));
    assert_eq!(head("next").pagify(), Some(Verb::Page(PageTarget::Next)));

    assert_eq!(head("zoom fit").pagify(), Some(Verb::Zoom(ZoomTarget::Fit)));
    assert_eq!(head("fit").pagify(), Some(Verb::Zoom(ZoomTarget::Fit)));
    assert_eq!(head("zoom 150").pagify(), Some(Verb::Zoom(ZoomTarget::Factor(1.5))));
    assert_eq!(head("zoom 150%").pagify(), Some(Verb::Zoom(ZoomTarget::Factor(1.5))));

    // Pages are one-based on screen and zero-based in the engine. The box is
    // where a reader types, so it takes what a reader would say.
    assert!(matches!(head("page 0"), Dispatch::Bad(_)));
}

#[test]
fn a_page_only_turns_in_quarters() {
    assert_eq!(head("rotate").pagify(), Some(Verb::RotatePage(90)));
    assert_eq!(head("rotate 180").pagify(), Some(Verb::RotatePage(180)));
    assert_eq!(head("rotate -90").pagify(), Some(Verb::RotatePage(270)));
    assert!(matches!(head("rotate 45"), Dispatch::Bad(_)));
}

#[test]
fn a_named_but_unbuilt_verb_says_so_rather_than_pretending_not_to_know_it() {
    // Was `redact` until redaction was built, and the change of example is the
    // point rather than an inconvenience: this test failing is how a verb that
    // has quietly become real announces itself. A planned verb that stayed
    // planned in the table after it worked would tell every user who reached
    // for it that the program cannot do the thing it can do.
    match head("bookmark") {
        Dispatch::Pagify(Verb::Planned { verb, phase }) => {
            assert_eq!(verb, "bookmark");
            assert!(!phase.is_empty());
        }
        other => panic!("expected Planned, got {other:?}"),
    }
}

/// And the ones that used to be the example are now built.
///
/// The list grows as the table shrinks. Each of these was once what the test
/// above pointed at, and each moving here is the whole point of that test.
#[test]
fn the_verbs_that_were_promises_are_no_longer() {
    assert_eq!(head("redact").pagify(), Some(Verb::Redact));
    assert_eq!(head("certify").pagify(), Some(Verb::Certify(None)));
    assert_eq!(head("timestamp").pagify(), Some(Verb::TimeStamp(None)));
    assert_eq!(head("validate").pagify(), Some(Verb::Validate));
    assert_eq!(head("signature").pagify(), Some(Verb::Signature { draw: false }));
    assert_eq!(
        head("managesignatures").pagify(),
        Some(Verb::ManageSignatures(pagify_shell::verbs::Signatures::Open))
    );
    assert_eq!(head("applysignatures").pagify(), Some(Verb::ApplySignatures));
    assert_eq!(head("documentstatus").pagify(), Some(Verb::DocumentStatus));
    // The long name is what the ribbon presses; the short one is what a person
    // types.
    assert_eq!(head("status").pagify(), Some(Verb::DocumentStatus));
    assert_eq!(head("signrectangle").pagify(), Some(Verb::SignRectangle));
    assert_eq!(head("signline").pagify(), Some(Verb::SignLine));
    assert_eq!(head("predefinedtext").pagify(), Some(Verb::PredefinedText(None)));
    assert_eq!(head("moveobject").pagify(), Some(Verb::MoveThing));
    assert_eq!(head("editobject").pagify(), Some(Verb::EditObject));
}

/// **Drawing one and placing one are the same word, differently.**
///
/// `signature` is what somebody types when they want to sign; `signature draw`
/// is what they type when they want a new mark. Anything else is refused with
/// both, rather than guessed at.
#[test]
fn the_signature_tool_draws_or_places() {
    assert_eq!(head("signature").pagify(), Some(Verb::Signature { draw: false }));
    assert_eq!(head("signature place").pagify(), Some(Verb::Signature { draw: false }));
    assert_eq!(head("signature draw").pagify(), Some(Verb::Signature { draw: true }));
    assert_eq!(head("signature new").pagify(), Some(Verb::Signature { draw: true }));

    match head("signature sideways") {
        Dispatch::Bad(said) => {
            assert!(said.contains("signature draw"), "it did not say what does work: {said}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// **Managing takes a name, and says so when one is missing.**
#[test]
fn managing_signatures_names_which_one() {
    use pagify_shell::verbs::Signatures as S;

    assert_eq!(head("managesignatures").pagify(), Some(Verb::ManageSignatures(S::Open)));
    assert_eq!(head("managesignatures list").pagify(), Some(Verb::ManageSignatures(S::List)));
    assert_eq!(
        head("managesignatures use Signature 2").pagify(),
        Some(Verb::ManageSignatures(S::Use("Signature 2".into()))),
        "a name with a space in it is one name"
    );
    assert_eq!(
        head("managesignatures delete work").pagify(),
        Some(Verb::ManageSignatures(S::Forget("work".into())))
    );
    // Renaming takes only the new name: two names could not be told apart.
    assert_eq!(
        head("managesignatures rename my mark").pagify(),
        Some(Verb::ManageSignatures(S::Rename("my mark".into())))
    );

    match head("managesignatures use") {
        Dispatch::Bad(said) => assert!(said.contains("list"), "it did not say how to find out: {said}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(matches!(head("managesignatures sideways"), Dispatch::Bad(_)));
}

/// **`move` still belongs to the drawing kernel.**
///
/// Moving something on the page is `moveobject`. The short word already moves
/// marks, and is tested doing so — a new tool taking it would have broken a
/// working one to name itself.
#[test]
fn moving_a_page_object_does_not_take_the_drawing_verb() {
    assert_eq!(head("moveobject").pagify(), Some(Verb::MoveThing));
    assert_ne!(head("move").pagify(), Some(Verb::MoveThing));
}

/// **A snippet is whatever somebody types, including a word a verb would want.**
///
/// Free text rather than sub-verbs: reserving `list` or `delete` out of it
/// would mean a snippet that happens to be one could never be kept, and
/// somebody typing their own name should not have to know which words are
/// spoken for.
#[test]
fn predefined_text_takes_whatever_words_it_is_given() {
    assert_eq!(head("predefinedtext").pagify(), Some(Verb::PredefinedText(None)));
    assert_eq!(
        head("predefinedtext Jane Smith").pagify(),
        Some(Verb::PredefinedText(Some("Jane Smith".into())))
    );
    // Words a sub-verb would have claimed.
    assert_eq!(
        head("predefinedtext list").pagify(),
        Some(Verb::PredefinedText(Some("list".into())))
    );
    assert_eq!(
        head("predefinedtext delete").pagify(),
        Some(Verb::PredefinedText(Some("delete".into())))
    );
}

/// **A button must not say "not built yet" about something the program does.**
///
/// The PagiSign tab has a Check, a Cross and a Dot, and every one of them said
/// it was planned — for marks `fillsign` had been making since it was wired.
/// Three doors onto one implementation, which is what they always were.
#[test]
fn the_pagisign_marks_are_the_fillsign_marks() {
    assert_eq!(head("signcheck").pagify(), Some(Verb::FillSign(Some("tick".into()))));
    assert_eq!(head("signcross").pagify(), Some(Verb::FillSign(Some("cross".into()))));
    assert_eq!(head("signdot").pagify(), Some(Verb::FillSign(Some("dot".into()))));

    // And none of them is still described as a promise.
    for verb in ["signcheck", "signcross", "signdot", "signline", "signrectangle"] {
        assert!(
            !matches!(head(verb), Dispatch::Pagify(Verb::Planned { .. })),
            "{verb} still says it is not built"
        );
    }
}

/// **The rule and the box are fill-and-sign marks, and `fillsign` finds them.**
///
/// It is its own verb because it takes two corners where the others take a
/// point — but somebody reaching for it through the tool the others live on is
/// asking for the right thing, so they get it rather than a refusal.
#[test]
fn the_box_is_reachable_from_the_tool_its_siblings_live_on() {
    assert_eq!(head("signrectangle").pagify(), Some(Verb::SignRectangle));
    assert_eq!(head("fillsign rectangle").pagify(), Some(Verb::SignRectangle));
    assert_eq!(head("fillsign box").pagify(), Some(Verb::SignRectangle));
    assert_eq!(head("signline").pagify(), Some(Verb::SignLine));
    assert_eq!(head("fillsign line").pagify(), Some(Verb::SignLine));
    assert_eq!(head("fillsign strike").pagify(), Some(Verb::SignLine));

    // And the others are unchanged.
    assert_eq!(head("fillsign tick").pagify(), Some(Verb::FillSign(Some("tick".into()))));

    match head("fillsign triangle") {
        Dispatch::Bad(said) => {
            assert!(said.contains("rectangle"), "it did not offer the box: {said}");
            assert!(said.contains("line"), "it did not offer the rule: {said}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Submit semantics — §7
// ---------------------------------------------------------------------------

#[test]
fn enter_space_and_the_button_all_submit() {
    for how in [Submit::Enter, Submit::Space, Submit::Button] {
        let mut box_ = CommandBox::default();
        box_.input_mut().push_str("fit");
        assert!(box_.submit(how).is_some(), "{how:?} should submit");
        assert!(box_.input().is_empty(), "{how:?} should clear the input");
    }
}

#[test]
fn space_is_a_space_while_typing_text_content() {
    // The rule that makes it possible to type a caption. Enter still submits.
    let mut box_ = CommandBox::default();
    box_.set_mode(Mode::TextContent);
    box_.input_mut().push_str("site plan");

    assert!(!box_.space_submits());
    assert!(box_.submit(Submit::Space).is_none(), "space must not submit here");
    assert_eq!(box_.input(), "site plan", "and must not eat the input");

    assert!(box_.submit(Submit::Enter).is_some(), "Enter still submits");
}

#[test]
fn enter_on_an_empty_box_repeats_the_last_command() {
    let mut box_ = CommandBox::default();
    box_.input_mut().push_str("zoom in");
    box_.submit(Submit::Enter);

    let repeated = box_.submit(Submit::Enter).expect("Enter on empty repeats");
    assert_eq!(repeated.pagify(), Some(Verb::Zoom(ZoomTarget::In)));
    assert_eq!(box_.last_command(), Some("zoom in"));
}

#[test]
fn an_empty_box_with_no_history_does_nothing() {
    let mut box_ = CommandBox::default();
    assert!(box_.submit(Submit::Enter).is_none());
    assert!(box_.submit(Submit::Space).is_none());
    assert!(box_.history().is_empty(), "nothing happened, so nothing to say");
}

#[test]
fn a_stray_space_in_an_empty_box_does_not_repeat() {
    // Only Enter repeats. Space repeating would make a rested thumb destructive.
    let mut box_ = CommandBox::default();
    box_.input_mut().push_str("zoom in");
    box_.submit(Submit::Enter);

    assert!(box_.submit(Submit::Space).is_none());
    assert!(box_.submit(Submit::Button).is_none());
}

#[test]
fn submitting_echoes_into_the_history() {
    let mut box_ = CommandBox::default();
    box_.input_mut().push_str("zoom fit");
    box_.submit(Submit::Enter);

    let entry = box_.history().first().expect("an echo");
    assert_eq!(entry.kind, Kind::Echo);
    assert_eq!(entry.text, "zoom fit");
}

#[test]
fn repeating_a_command_does_not_fill_the_recall_list_with_copies() {
    let mut box_ = CommandBox::default();
    box_.input_mut().push_str("ci");
    box_.submit(Submit::Enter);
    for _ in 0..3 {
        box_.submit(Submit::Enter);
    }

    box_.recall_previous();
    assert_eq!(box_.input(), "ci");
    box_.recall_previous();
    assert_eq!(box_.input(), "ci", "there should be only one entry to walk back to");
}

// ---------------------------------------------------------------------------
// Escape — §7 and phase 1
// ---------------------------------------------------------------------------

#[test]
fn escape_clears_typing_first_and_only_then_returns_to_the_pointer() {
    let mut box_ = CommandBox::default();
    box_.input_mut().push_str("fill");

    assert_eq!(box_.escape(), Escaped::ClearedInput);
    assert!(box_.input().is_empty());

    assert_eq!(box_.escape(), Escaped::ReturnedToPointer);
}

#[test]
fn escape_leaves_text_content_mode() {
    // Otherwise Escape out of a half-typed caption would leave the box still
    // treating space as a character, and the next command would not submit.
    let mut box_ = CommandBox::default();
    box_.set_mode(Mode::TextContent);
    box_.escape();
    assert_eq!(box_.mode(), Mode::Command);
    assert!(box_.space_submits());
}

// ---------------------------------------------------------------------------
// History recall
// ---------------------------------------------------------------------------

#[test]
fn recall_walks_back_and_forward_to_the_live_input() {
    let mut box_ = CommandBox::default();
    for line in ["zoom fit", "page 2", "l"] {
        box_.input_mut().push_str(line);
        box_.submit(Submit::Enter);
    }

    box_.recall_previous();
    assert_eq!(box_.input(), "l");
    box_.recall_previous();
    assert_eq!(box_.input(), "page 2");
    box_.recall_previous();
    assert_eq!(box_.input(), "zoom fit");
    box_.recall_previous();
    assert_eq!(box_.input(), "zoom fit", "and stops at the oldest");

    box_.recall_next();
    assert_eq!(box_.input(), "page 2");
    box_.recall_next();
    assert_eq!(box_.input(), "l");
    box_.recall_next();
    assert_eq!(box_.input(), "", "forward past the newest returns to an empty box");
}

// ---------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------

#[test]
fn help_covers_both_namespaces() {
    let text = verbs::help_text(None).join("\n");
    assert!(text.contains("open"), "no document verbs in help");
    assert!(text.contains("SIMLUX"), "no mention of the drawing namespace in help");
}

#[test]
fn help_on_an_overridden_word_explains_what_it_means_in_each_program() {
    let text = verbs::help_text(Some("rotate")).join("\n");
    assert!(text.contains("selection"), "does not say what SIMLUX means by it: {text}");
    assert!(text.contains("ro"), "does not name the alias that still works: {text}");
}

#[test]
fn a_refusal_reports_the_reason_rather_than_failing_silently() {
    let mut box_ = CommandBox::default();
    box_.input_mut().push_str("wall");
    let dispatch = box_.submit(Submit::Enter).expect("dispatched");
    box_.report(&dispatch);

    let last = box_.history().last().expect("something said");
    assert_eq!(last.kind, Kind::Error);
    assert!(last.text.contains("polyline"), "no alternative offered: {}", last.text);
}

#[test]
fn the_prompt_names_the_document_it_will_act_on() {
    let mut box_ = CommandBox::default();
    assert!(box_.prompt().render().starts_with("pagify"));

    box_.prompt_mut().document = Some("survey.pdf".into());
    let rendered = box_.prompt().render();
    assert!(rendered.contains("survey.pdf"), "prompt does not name the document: {rendered}");
}

// ---------------------------------------------------------------------------

/// Test-only convenience: the Pagify verb, if that is what this was.
trait DispatchExt {
    fn pagify(&self) -> Option<Verb>;
}

impl DispatchExt for Dispatch {
    fn pagify(&self) -> Option<Verb> {
        match self {
            Dispatch::Pagify(v) => Some(v.clone()),
            _ => None,
        }
    }
}

/// Pagify handles PDFs and files related to them. It does not open or write
/// drawings, and no line typed into the box may reach the kernel's `.dxf` /
/// `.rsm` reader or writer.
///
/// Both of the kernel's file commands are shadowed by Pagify's own table, so
/// this passes today by construction. It is written against the *guarantee*
/// rather than the mechanism so that removing an override, reordering the two
/// lookups, or adding an alias cannot quietly make a drawing path reachable
/// again.
#[test]
fn no_route_through_the_box_can_reach_a_drawing_file() {
    let attempts = [
        "open", "open plan.dxf", "open plan.rsm",
        "save", "save plan.dxf", "saveas", "saveas plan.rsm",
        "OPEN plan.dxf", "  save   plan.dxf  ",
    ];

    for line in attempts {
        if let Some(Dispatch::Kernel(command)) = dispatch(line) {
            assert!(
                !matches!(
                    *command,
                    cad_kernel::parser::Command::Open(_)
                        | cad_kernel::parser::Command::SaveAs(_)
                ),
                "`{line}` reached the kernel's drawing-file handling: {command:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Paths, as people actually type and paste them
// ---------------------------------------------------------------------------

#[test]
fn a_path_with_spaces_opens() {
    // The bug that made it impossible to open most files on a Mac: splitting
    // the line on whitespace truncated the path at the first space, so
    // `~/My Documents/report.pdf` became `~/My`.
    match head("open /Users/someone/My Documents/HSI CATALOG 2026.pdf") {
        Dispatch::Pagify(Verb::Open(path)) => {
            assert_eq!(path.to_string_lossy(), "/Users/someone/My Documents/HSI CATALOG 2026.pdf");
        }
        other => panic!("expected Open, got {other:?}"),
    }
}

#[test]
fn a_quoted_or_escaped_path_loses_its_quoting() {
    // Both are what a drag from Finder or a paste from a terminal produces.
    for line in [
        r#"open "/tmp/My File.pdf""#,
        r"open /tmp/My\ File.pdf",
    ] {
        match head(line) {
            Dispatch::Pagify(Verb::Open(path)) => {
                assert_eq!(path.to_string_lossy(), "/tmp/My File.pdf", "for `{line}`");
            }
            other => panic!("`{line}` gave {other:?}"),
        }
    }
}

#[test]
fn a_tilde_is_expanded_because_nothing_else_will_expand_it() {
    // There is no shell here. An unexpanded `~` is a directory that does not
    // exist, and the file simply refuses to open.
    let home = std::env::var("HOME").expect("a home directory");
    match head("open ~/Downloads/a.pdf") {
        Dispatch::Pagify(Verb::Open(path)) => {
            assert_eq!(path, std::path::PathBuf::from(&home).join("Downloads/a.pdf"));
        }
        other => panic!("expected Open, got {other:?}"),
    }
}

#[test]
fn open_with_no_path_asks_for_a_file_rather_than_complaining() {
    assert!(matches!(head("open"), Dispatch::Pagify(Verb::OpenDialog)));
}

#[test]
fn import_can_still_tell_a_path_from_a_page_range() {
    match head("import /tmp/other.pdf 1-3") {
        Dispatch::Pagify(Verb::Import { source, pages }) => {
            assert_eq!(source.to_string_lossy(), "/tmp/other.pdf");
            assert_eq!(pages, "1-3");
        }
        other => panic!("expected Import, got {other:?}"),
    }

    // And a spaced path with no range is all path.
    match head("import /tmp/My Other File.pdf") {
        Dispatch::Pagify(Verb::Import { source, pages }) => {
            assert_eq!(source.to_string_lossy(), "/tmp/My Other File.pdf");
            assert_eq!(pages, "all");
        }
        other => panic!("expected Import, got {other:?}"),
    }
}

/// Bare `lock` still arms the rectangle tool, and naming pages locks them
/// whole. Two different operations, so two variants — see `Verb::LockPages`.
#[test]
fn lock_takes_pages_or_arms_the_rectangle_tool() {
    assert_eq!(head("lock").pagify(), Some(Verb::Lock));

    match head("lock all") {
        Dispatch::Pagify(Verb::LockPages(spec)) => assert_eq!(spec, "all"),
        other => panic!("expected LockPages, got {other:?}"),
    }
    match head("lock 1-3,7") {
        Dispatch::Pagify(Verb::LockPages(spec)) => assert_eq!(spec, "1-3,7"),
        other => panic!("expected LockPages, got {other:?}"),
    }
    // The button on the Protect tab spells it out; the shorthand exists so a
    // reader who types it is not told the word is unknown.
    match head("lockall") {
        Dispatch::Pagify(Verb::LockPages(spec)) => assert_eq!(spec, "all"),
        other => panic!("expected LockPages, got {other:?}"),
    }
}

/// `timestamp` needs an address, and has no default.
///
/// **The verb that uses the network.** A default would make the program contact
/// somewhere nobody chose, and which authority to trust is not the program's
/// decision to make.
#[test]
fn timestamp_takes_an_address_and_assumes_none() {
    assert_eq!(head("timestamp").pagify(), Some(Verb::TimeStamp(None)));
    assert_eq!(
        head("timestamp http://tsa.example.com/x").pagify(),
        Some(Verb::TimeStamp(Some("http://tsa.example.com/x".into())))
    );
}

/// `certify` reports; with a certificate it signs.
#[test]
fn certify_reports_or_signs_with_a_certificate() {
    assert_eq!(head("certify").pagify(), Some(Verb::Certify(None)));
    // `sign` is the word people reach for.
    assert_eq!(head("sign").pagify(), Some(Verb::Certify(None)));

    let Some(Verb::Certify(Some(path))) = head("certify /tmp/me.p12").pagify() else {
        panic!("a certificate path did not parse");
    };
    assert!(path.to_string_lossy().ends_with("me.p12"), "{path:?}");
}

/// `fillsign` types where you click; with a word it places that mark.
#[test]
fn fillsign_types_or_places_a_mark() {
    assert_eq!(head("fillsign").pagify(), Some(Verb::FillSign(None)));
    assert_eq!(head("fillsign tick").pagify(), Some(Verb::FillSign(Some("tick".into()))));
    assert_eq!(head("fillsign cross").pagify(), Some(Verb::FillSign(Some("cross".into()))));

    // A word it does not know is refused rather than silently typing instead.
    match head("fillsign squiggle") {
        Dispatch::Bad(why) => assert!(why.contains("tick"), "unhelpful: {why}"),
        other => panic!("an unknown mark was accepted: {other:?}"),
    }
}

/// `sensitivity` says, marks, or unmarks — and refuses a word it does not know
/// rather than treating it as "report", which would silently do nothing.
#[test]
fn sensitivity_reports_marks_and_unmarks() {
    assert_eq!(head("sensitivity").pagify(), Some(Verb::Sensitivity(None)));
    assert_eq!(
        head("sensitivity confidential").pagify(),
        Some(Verb::Sensitivity(Some("confidential".into())))
    );
    // However it is typed.
    assert_eq!(
        head("sensitivity SECRET").pagify(),
        Some(Verb::Sensitivity(Some("SECRET".into())))
    );
    // Taking it off is an empty level rather than a fifth one.
    assert_eq!(head("sensitivity none").pagify(), Some(Verb::Sensitivity(Some(String::new()))));

    match head("sensitivity very secret") {
        Dispatch::Bad(why) => assert!(why.contains("confidential"), "unhelpful: {why}"),
        other => panic!("an unknown level was accepted: {other:?}"),
    }
}

/// `whiteout` is not a kind of `redact`. One covers, the other destroys, and
/// the table keeps them as far apart as the tools are.
#[test]
fn whiteout_is_its_own_verb_and_not_a_kind_of_redaction() {
    assert_eq!(head("whiteout").pagify(), Some(Verb::Whiteout));
    assert_ne!(head("whiteout").pagify(), Some(Verb::Redact));
}

/// `smartredact` reports; `smartredact redact` acts. Two words for the same
/// reason `hiddendata` has two: what it finds are candidates.
#[test]
fn smartredact_reports_before_it_redacts() {
    assert_eq!(head("smartredact").pagify(), Some(Verb::SmartRedact { redact: false }));
    assert_eq!(
        head("smartredact redact").pagify(),
        Some(Verb::SmartRedact { redact: true })
    );

    // And a word it does not know is refused rather than treated as "report" —
    // the safe-looking default is the one that quietly does nothing.
    match head("smartredact evrything") {
        Dispatch::Bad(why) => assert!(why.contains("evrything"), "unhelpful: {why}"),
        other => panic!("a misspelling was accepted: {other:?}"),
    }
}

/// `hiddendata` reports; `hiddendata clean` removes. Two words rather than
/// one, because a person deciding whether to sanitise needs to know the cost.
#[test]
fn hiddendata_reports_before_it_removes() {
    assert_eq!(head("hiddendata").pagify(), Some(Verb::HiddenData { clean: false }));
    assert_eq!(head("hiddendata clean").pagify(), Some(Verb::HiddenData { clean: true }));
    // The words people reach for instead.
    assert_eq!(head("sanitise").pagify(), Some(Verb::HiddenData { clean: false }));
    assert_eq!(head("sanitize remove").pagify(), Some(Verb::HiddenData { clean: true }));

    // And a word it does not know is refused rather than treated as "report".
    match head("hiddendata cleen") {
        Dispatch::Bad(why) => assert!(why.contains("cleen"), "unhelpful: {why}"),
        other => panic!("a misspelling was accepted: {other:?}"),
    }
}

/// `secure` puts a password on the file; it is not a locking verb.
///
/// Kept apart in the table as well as in the mind: locking hides content inside
/// a document anyone can open, securing shuts the whole file.
#[test]
fn securing_is_its_own_verb_and_not_a_kind_of_locking() {
    use pagify_shell::verbs::SecureOptions;

    // Bare `secure` permits everything, which is what "put a password on this"
    // means when nothing else is said.
    assert_eq!(head("secure").pagify(), Some(Verb::Secure(SecureOptions::default())));
    assert_eq!(head("unsecure").pagify(), Some(Verb::Unsecure));
    assert_ne!(head("secure").pagify(), Some(Verb::Lock));

    // The permissions are words after it, combinable and readable aloud.
    let Some(Verb::Secure(options)) = head("secure noprint nocopy").pagify() else {
        panic!("`secure noprint nocopy` did not parse");
    };
    assert!(!options.printing && !options.copying);
    assert!(options.editing && options.annotating, "it forbade more than it was told to");

    let Some(Verb::Secure(options)) = head("secure readonly").pagify() else {
        panic!("`secure readonly` did not parse");
    };
    assert_eq!(options.describe(), "reading only");

    // And a word it does not know is refused rather than ignored.
    match head("secure nopriting") {
        Dispatch::Bad(why) => assert!(why.contains("nopriting"), "unhelpful: {why}"),
        other => panic!("an unknown permission was accepted: {other:?}"),
    }
}

/// `lock` means the words you selected; `lockarea` is the rectangle, kept for
/// what a selection cannot express.
#[test]
fn lock_and_lockarea_are_different_gestures() {
    assert_eq!(head("lock").pagify(), Some(Verb::Lock));
    assert_eq!(head("lockarea").pagify(), Some(Verb::LockArea));
    assert!(matches!(head("lock all").pagify(), Some(Verb::LockPages(_))));
}

#[test]
fn outlinedfont_with_no_path_asks_for_a_file_too() {
    use pagify_shell::verbs::OutlinedFontAction;
    assert_eq!(
        head("outlinedfont").pagify(),
        Some(Verb::OutlinedFont(OutlinedFontAction::Dialog))
    );
}

#[test]
fn outlinedfont_takes_a_path_directly_for_scripting_and_replay() {
    use pagify_shell::verbs::OutlinedFontAction;
    match head("outlinedfont /tmp/My Font.ttf") {
        Dispatch::Pagify(Verb::OutlinedFont(OutlinedFontAction::Add(path))) => {
            assert_eq!(path.to_string_lossy(), "/tmp/My Font.ttf");
        }
        other => panic!("expected OutlinedFont(Add), got {other:?}"),
    }
}

#[test]
fn outlinedfont_remove_takes_a_path_not_an_index() {
    // A path survives the list changing under it between when it was shown
    // and when the command runs; an index would not.
    use pagify_shell::verbs::OutlinedFontAction;
    match head("outlinedfont remove /tmp/My Font.ttf") {
        Dispatch::Pagify(Verb::OutlinedFont(OutlinedFontAction::Remove(path))) => {
            assert_eq!(path.to_string_lossy(), "/tmp/My Font.ttf");
        }
        other => panic!("expected OutlinedFont(Remove), got {other:?}"),
    }

    match head("outlinedfont remove") {
        Dispatch::Bad(problem) => assert!(problem.contains("remove"), "unhelpful: {problem}"),
        other => panic!("expected Bad, got {other:?}"),
    }
}

#[test]
fn outlinedfont_remove_unquotes_a_path_with_spaces() {
    // The Home panel's own Remove button quotes the path it emits — see
    // `home.rs` — for exactly this reason: a font under `~/Library/Fonts`
    // routinely has one.
    use pagify_shell::verbs::OutlinedFontAction;
    match head(r#"outlinedfont remove "/Users/name/Library/Fonts/My Font.ttf""#) {
        Dispatch::Pagify(Verb::OutlinedFont(OutlinedFontAction::Remove(path))) => {
            assert_eq!(path.to_string_lossy(), "/Users/name/Library/Fonts/My Font.ttf");
        }
        other => panic!("expected OutlinedFont(Remove), got {other:?}"),
    }
}

#[test]
fn outlinedfont_clear_empties_the_list() {
    use pagify_shell::verbs::OutlinedFontAction;
    assert_eq!(head("outlinedfont clear").pagify(), Some(Verb::OutlinedFont(OutlinedFontAction::Clear)));
    // Case-insensitive, like the other sub-words this table matches (`page NEXT` etc).
    assert_eq!(head("outlinedfont CLEAR").pagify(), Some(Verb::OutlinedFont(OutlinedFontAction::Clear)));
}

/// A blind spot the guard had, found the hard way.
///
/// `every_collision_with_the_kernel_is_deliberate` tests each Pagify token on
/// its own, and a bare token is not the whole story: an arm written as
/// `"find" | "f" if !tail.is_empty()` claims `f 25` while leaving bare `f`
/// alone. The bare test passes, `fillet 25` silently becomes a text search, and
/// nothing says so.
///
/// So the kernel's aliases are exercised **with arguments** as well.
#[test]
fn an_alias_with_arguments_still_reaches_the_kernel() {
    let argued = [
        ("f 25", "fillet"),
        ("o 10", "offset"),
        ("l 0,0 10,10", "line"),
        ("ci 5,5 2", "circle"),
        ("po 3,4", "point"),
        ("ro", "rotate selection"),
        ("cha 5", "chamfer"),
    ];

    for (line, meaning) in argued {
        assert!(
            verbs::parse(line).is_none(),
            "Pagify's table claims `{line}`, which SIMLUX users type for {meaning}"
        );
        match head(line) {
            Dispatch::Kernel(_) | Dispatch::Bad(_) => {}
            other => panic!("`{line}` ({meaning}) no longer reaches the kernel: {other:?}"),
        }
    }
}
