//! A drawing of 700,000 lines froze the window: the UI thread asked the engine about the
//! page under the pointer, the rail's thumbnails and a page's first look, and each of those
//! parses the whole page and waits for the one lock a render holds for seconds. These hold
//! the lock from another thread — what a render does — and check a frame does not wait.

use super::*;
use crate::ui_tests::{async_harness, fixture, harness_60fps_cold};
use egui_kittest::Harness;
use std::time::{Duration, Instant};

/// Hold the engine's one lock for `ms` from another thread, as a page render does, and
/// return once it has it.
fn hold_the_engine(ms: u64) -> std::thread::JoinHandle<()> {
    let (taken, got) = std::sync::mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        pdf_core::registry::exclusive(|| {
            taken.send(()).ok();
            std::thread::sleep(Duration::from_millis(ms));
        })
    });
    got.recv().expect("the holder never took the lock");
    holder
}

fn move_pointer_over_the_page(h: &mut Harness<'static, PagifyApp>, frames: usize) {
    let centre = h.state().tab().view_state.viewport_rect.expect("the page was never drawn").center();
    for i in 0..frames {
        h.input_mut().events.push(egui::Event::PointerMoved(centre + egui::vec2(i as f32, 0.0)));
        h.run_steps(1);
    }
}

/// **A hover is not a call into the engine.** It used to read the page's annotations and
/// links, which wait for any render and then parse the page themselves.
#[test]
fn hovering_a_page_does_not_wait_for_the_engine() {
    let mut h = async_harness(&fixture("two-column.pdf"));
    let holder = hold_the_engine(1500);
    let started = Instant::now();
    move_pointer_over_the_page(&mut h, 12);
    let took = started.elapsed();
    holder.join().expect("holder");
    assert!(took < Duration::from_millis(700), "12 frames of a hover took {took:?} with the engine busy");
}

/// **Nor are the first frames**: a page's first look was rendered right there, on the UI
/// thread. (The rail's thumbnails were too — nine seconds to open the drawing — but this
/// test does not catch their coming back: it passed with them put back. The measurement
/// on the drawing itself, `hover_scroll_zoom_frame_times_on_a_real_file`, does.)
#[test]
fn the_first_frames_do_not_wait_for_the_engine() {
    let mut h = harness_60fps_cold(&fixture("two-column.pdf"), true);
    let holder = hold_the_engine(1500);
    let started = Instant::now();
    h.run_steps(10);
    let took = started.elapsed();
    holder.join().expect("holder");
    let stats = h.state().render_state.render_stats;
    assert!(took < Duration::from_millis(700), "the first 10 frames took {took:?} with the engine busy");
    assert_eq!(stats.on_ui_thread, 0, "a page was rendered on the UI thread: {stats:?}");
}

/// The thumbnails and the page itself do come, from the worker, once the engine is free.
#[test]
fn the_rail_and_the_page_are_drawn_by_the_worker() {
    let mut h = async_harness(&fixture("two-column.pdf"));
    let mut thumb = false;
    for _ in 0..600 {
        std::thread::sleep(Duration::from_millis(10));
        h.run_steps(1);
        let doc = h.state().tab().doc.as_ref().expect("doc");
        thumb = doc.caches.thumbs.contains_key(&0);
        if thumb && !doc.caches.exact.is_empty() {
            break;
        }
    }
    let doc = h.state().tab().doc.as_ref().expect("doc");
    assert!(thumb, "the first page's thumbnail never arrived");
    assert!(!doc.caches.exact.is_empty(), "the page's own picture never arrived");
    assert_eq!(h.state().render_state.render_stats.on_ui_thread, 0);
}

/// A page drawn from its exact picture is not also rendered as a quantised one — the
/// second render of the same page at nearly the same size, which on the drawing was
/// 2-3 s of the one lock for a picture nobody draws from.
#[test]
fn a_page_drawn_exact_is_not_also_rendered_quantised() {
    let mut h = async_harness(&fixture("two-column.pdf"));
    for _ in 0..600 {
        std::thread::sleep(Duration::from_millis(10));
        h.run_steps(1);
        if !h.state().tab().doc.as_ref().expect("doc").caches.exact.is_empty() {
            break;
        }
    }
    // And a while longer, for a quantised one to arrive if it was asked for.
    for _ in 0..60 {
        std::thread::sleep(Duration::from_millis(10));
        h.run_steps(1);
    }
    let doc = h.state().tab().doc.as_ref().expect("doc");
    assert!(!doc.caches.exact.is_empty(), "setup: no exact picture");
    assert!(doc.caches.textures.is_empty(), "a quantised picture was made as well: {:?}", doc.caches.textures.keys().collect::<Vec<_>>());
}

/// What the pointer needs to know about a page arrives from the facts worker, and says
/// where the words are.
#[test]
fn the_facts_about_a_page_arrive_and_know_where_its_words_are() {
    let mut h = async_harness(&fixture("two-column.pdf"));
    let mut got = None;
    for i in 0..600 {
        let centre = h.state().tab().view_state.viewport_rect.expect("drawn").center();
        h.input_mut().events.push(egui::Event::PointerMoved(centre + egui::vec2((i % 7) as f32, 0.0)));
        std::thread::sleep(Duration::from_millis(10));
        h.run_steps(1);
        let page = h.state().tab().view_state.page;
        if let Some(facts) = h.state().tab().doc.as_ref().and_then(|d| d.caches.facts.get(&page).cloned()) {
            got = Some((page, facts));
            break;
        }
    }
    let (_, facts) = got.expect("the facts never arrived");
    assert!(facts.weight.is_some(), "the page was not counted");
    let chars = facts.characters.as_ref().expect("no characters");
    let word = chars.line_rects(0..chars.len()).into_iter().next().expect("a line of words");
    let in_a_word = pagify_shell::page_space::AppPoint::new(((word.left + word.right) / 2.0) as f64, ((word.top + word.bottom) / 2.0) as f64);
    assert!(facts.word_at(in_a_word), "no word where the page has one");
    assert!(!facts.word_at(pagify_shell::page_space::AppPoint::new(-50.0, -50.0)), "a word off the page");
    assert!(!facts.link_at(in_a_word), "this page has no links");
}

/// An edit throws the facts away with everything else read from the page.
#[test]
fn an_edit_discards_the_facts() {
    let mut h = async_harness(&fixture("two-column.pdf"));
    for i in 0..600 {
        move_pointer_over_the_page(&mut h, 1);
        std::thread::sleep(Duration::from_millis(10));
        if h.state().tab().doc.as_ref().is_some_and(|d| !d.caches.facts.is_empty()) {
            break;
        }
        assert!(i < 599, "the facts never arrived");
    }
    h.state_mut().tab_mut().doc.as_mut().expect("doc").rendered_is_stale();
    assert!(h.state().tab().doc.as_ref().expect("doc").caches.facts.is_empty(), "the facts outlived the edit");
}
