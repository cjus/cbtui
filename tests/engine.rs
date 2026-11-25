//! cbtui against a real engine: `CBTUI_TEST_ENGINE=/path/to/cb2026 cargo test`.
//!
//! Without that variable the tests say so and pass, so `cargo test` works on a machine with no
//! engine. CI installs cb2026 at the commit `Cargo.toml` pins and sets it.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use cbtui::app::{App, NodeSummary, Settings};
use cbtui::node::Node;

fn engine() -> Option<String> {
    let engine = std::env::var("CBTUI_TEST_ENGINE")
        .ok()
        .filter(|e| !e.is_empty());
    if engine.is_none() {
        eprintln!("CBTUI_TEST_ENGINE isn't set: skipping the real-engine test");
    }
    engine
}

#[test]
fn the_engine_plays_both_sides_for_a_few_moves() {
    let Some(engine) = engine() else {
        return;
    };
    let (tx, rx) = mpsc::channel();
    let mut node = Node::spawn(&engine, &["--cnp".to_string()], move |event| {
        let _ = tx.send(event);
    })
    .expect("the engine starts and answers initialize");
    let summary = NodeSummary::from_info(node.info());
    assert!(summary.caps.progress, "cb2026 reports progress");

    let settings = Settings {
        engine_plays: [false, false],
        movetime_ms: 50,
        ..Settings::default()
    };
    let mut app = App::new(summary, settings).unwrap();
    app.command("play both", &mut node);
    assert!(app.searching());

    let deadline = Instant::now() + Duration::from_secs(30);
    while app.game.moves().len() < 6 {
        let event = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("the engine keeps answering");
        app.handle_node(event, &mut node);
        assert!(!app.disconnected, "{}", app.status);
    }
    // Every move went through the rules, so they're legal. Searched moves carry their score
    // and depth as a PGN comment (a move from an opening book may have none).
    assert!(app.game.moves().iter().any(|m| m.comment.is_some()));
    app.command("play none", &mut node);
    node.shutdown();
}

#[test]
fn a_takeback_mid_search_lets_the_next_search_start_at_once() {
    let Some(engine) = engine() else {
        return;
    };
    let (tx, rx) = mpsc::channel();
    let mut node = Node::spawn(&engine, &["--cnp".to_string()], move |event| {
        let _ = tx.send(event);
    })
    .unwrap();
    let settings = Settings {
        movetime_ms: 2000,
        ..Settings::default()
    };
    let mut app = App::new(NodeSummary::from_info(node.info()), settings).unwrap();
    app.command(
        "fen rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1",
        &mut node,
    );
    assert!(app.searching(), "the engine plays Black, to move");
    // A new game while it thinks: the old search is stopped and a new one starts.
    app.command("new", &mut node);
    app.command("play black", &mut node);
    app.command("time 100", &mut node);
    app.command("play white", &mut node);
    let deadline = Instant::now() + Duration::from_secs(20);
    while app.game.moves().is_empty() {
        let event = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("the engine answers");
        app.handle_node(event, &mut node);
        assert!(!app.status.contains("couldn't search"), "{}", app.status);
    }
    assert_eq!(
        app.game.moves().len(),
        1,
        "only the live search's move is played"
    );
    node.shutdown();
}

#[test]
fn an_analysis_finds_several_lines_and_the_evaluation_has_terms() {
    let Some(engine) = engine() else {
        return;
    };
    let (tx, rx) = mpsc::channel();
    let mut node = Node::spawn(&engine, &["--cnp".to_string()], move |event| {
        let _ = tx.send(event);
    })
    .unwrap();
    let summary = NodeSummary::from_info(node.info());
    assert!(summary.caps.multipv && summary.caps.evaluate && summary.caps.techniques);
    let mut app = App::new(summary, Settings::default()).unwrap();
    app.expect_techniques(node.techniques().unwrap());
    app.command("analyse", &mut node);
    app.command("eval", &mut node);

    // Three lines (the default), every technique listed, and the terms of the evaluation.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let lines = app.analysis.as_ref().map_or(0, |a| a.lines.len());
        if lines >= 3 && app.techniques.is_some() && app.evaluation.is_some() {
            break;
        }
        let event = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("the engine keeps reporting");
        app.handle_node(event, &mut node);
        assert!(!app.disconnected, "{}", app.status);
    }
    assert!(!app.evaluation.as_ref().unwrap().result.terms.is_empty());
    assert!(!app.techniques.as_ref().unwrap().is_empty());
    app.command("analyse", &mut node);
    assert!(app.analysis.is_none());
    node.shutdown();
}
