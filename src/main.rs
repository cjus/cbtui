//! The `cbtui` program: starts the engine, takes over the terminal, and runs the event loop.

use std::io;
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use cbtui::app::{App, NodeSummary};
use cbtui::args::{self, Args, ENGINE_ENV, USAGE};
use cbtui::help::WHEEL_LINES;
use cbtui::node::{Engine, NoEngine, Node, NodeEvent};
use cbtui::pieces::{PieceChoice, Pieces};
use cbtui::review::Review;
use cbtui::ui::{self, Palette};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as TermEvent, MouseButton, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::Rect;
use ratatui_image::picker::Picker;

/// How often the screen is redrawn while a clock runs, so it counts down smoothly.
const CLOCK_REDRAW: Duration = Duration::from_millis(100);

/// Everything the loop reacts to, from one channel.
enum Event {
    /// A key, a click or a resize.
    Term(TermEvent),
    /// Something from the engine.
    Node(NodeEvent),
}

fn main() -> ExitCode {
    let parsed = args::parse(std::env::args().skip(1), std::env::var(ENGINE_ENV).ok());
    let (engine, engine_args, settings, piece_choice, pgn) = match parsed {
        Ok(Args::Run {
            engine,
            engine_args,
            settings,
            pieces,
            pgn,
        }) => (engine, engine_args, settings, pieces, pgn),
        Ok(Args::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(Args::Version) => {
            println!("cbtui {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("cbtui: {e}");
            return ExitCode::FAILURE;
        }
    };

    let review = match pgn.as_deref().map(read_review).transpose() {
        Ok(review) => review,
        Err(e) => {
            eprintln!("cbtui: {e}");
            return ExitCode::FAILURE;
        }
    };

    let (tx, rx) = mpsc::channel();
    let node_tx = tx.clone();
    // Connect before taking over the terminal, so a missing engine is a plain error message.
    // A PGN viewer carries on without one.
    let mut no_engine_reason = None;
    let mut node = match Node::spawn(&engine, &engine_args, move |event| {
        let _ = node_tx.send(Event::Node(event));
    }) {
        Ok(node) => Some(node),
        Err(e) if review.is_some() => {
            no_engine_reason = Some(e.to_string());
            None
        }
        Err(e) => {
            eprintln!("cbtui: {e}");
            eprintln!("Name the engine with --engine <path> or ${ENGINE_ENV}.");
            return ExitCode::FAILURE;
        }
    };
    let summary = node.as_ref().map_or_else(NodeSummary::absent, |node| {
        NodeSummary::from_info(node.info())
    });
    let mut app = match App::new(summary, settings) {
        Ok(app) => app,
        Err(e) => {
            eprintln!("cbtui: {e}");
            if let Some(node) = node {
                node.shutdown();
            }
            return ExitCode::FAILURE;
        }
    };
    app.has_engine = node.is_some();
    if let Some(review) = review {
        app.open_review(review);
    }
    if let Some(reason) = no_engine_reason {
        app.status = format!("{} (No engine: {reason}.)", app.status);
    }
    if let Some(node) = node.as_mut()
        && app.node.caps.strength
        && let Ok(id) = node.levels()
    {
        app.expect_levels(id);
    }
    if let Some(node) = node.as_mut()
        && app.node.caps.techniques
        && let Ok(id) = node.techniques()
    {
        app.expect_techniques(id);
    }
    app.start(engine_of(&mut node, &mut NoEngine));

    let mut terminal = ratatui::init();
    install_mouse_restoring_panic_hook();
    // Ask the terminal whether it can show pictures, and its font's size in pixels. Its
    // answers come back on stdin, so this must happen before the input thread starts reading.
    let (picker, timed_out) = match piece_choice {
        // Pictures need tmux's passthrough, and asking can lose a key there (`query_terminal`).
        PieceChoice::Auto if inside_tmux() => (None, false),
        PieceChoice::Auto => query_terminal(),
        PieceChoice::Letters | PieceChoice::Symbols => (None, false),
    };
    let mut pieces = Pieces::choose(piece_choice, picker);
    app.status = format!("{} Pieces: {}.", app.status, pieces.describe());
    if timed_out {
        app.status.push_str(
            " (The terminal didn't answer in time: if a key seems ignored, press it again.)",
        );
    }

    thread::spawn(move || {
        while let Ok(event) = event::read() {
            if tx.send(Event::Term(event)).is_err() {
                break;
            }
        }
    });

    let _ = execute!(io::stdout(), EnableMouseCapture);
    let result = run(&mut terminal, &mut app, &mut node, &mut pieces, &rx);
    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
    if let Some(node) = node {
        node.shutdown();
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cbtui: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Reads the PGN file at `path` for the viewer, named after the file.
fn read_review(path: &str) -> Result<Review, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("can't read {path}: {e}"))?;
    let name = std::path::Path::new(path)
        .file_name()
        .map_or(path.into(), |name| name.to_string_lossy());
    Review::from_pgn(&text, &name).map_err(|e| format!("can't read the game in {path}: {e}"))
}

/// The engine to ask: `node`, or `none` when there is no node.
fn engine_of<'a>(node: &'a mut Option<Node>, none: &'a mut NoEngine) -> &'a mut dyn Engine {
    match node {
        Some(node) => node,
        None => none,
    }
}

/// Whether cbtui runs inside tmux.
fn inside_tmux() -> bool {
    std::env::var("TMUX").is_ok_and(|v| !v.is_empty())
}

/// Asks the terminal for its image protocol and font size. The second value is true when it
/// didn't answer in time.
///
/// **Pitfall:** when the terminal doesn't answer in time, ratatui-image gives up but leaves
/// its reader thread waiting on stdin, and that thread swallows the next key the person
/// presses. Inside tmux that happened about one start in two, so cbtui doesn't ask there.
fn query_terminal() -> (Option<Picker>, bool) {
    match Picker::from_query_stdio() {
        Ok(picker) => (Some(picker), false),
        Err(ratatui_image::errors::Errors::NoStdinResponse) => (None, true),
        Err(_) => (None, false),
    }
}

/// Makes a panic turn mouse reporting off before Ratatui's own hook restores the terminal,
/// so the shell isn't left printing escape codes for every mouse move.
fn install_mouse_restoring_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), DisableMouseCapture);
        previous(info);
    }));
}

/// Draws, waits for the next event, handles it and every other one already waiting, and
/// draws again, until the user quits.
fn run(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    node: &mut Option<Node>,
    pieces: &mut Pieces,
    rx: &Receiver<Event>,
) -> io::Result<()> {
    // A terminal that shows pictures has 24-bit colour, and the pictures need it to match.
    let palette = match pieces {
        Pieces::Images(_) => Palette::TRUE_COLOR,
        Pieces::Letters | Pieces::Symbols => Palette::detect(),
    };
    let mut area = Rect::default();
    // Engine-vs-engine at speed shouldn't pause on every redraw; at most once per batch.
    while !app.quit {
        terminal.draw(|frame| {
            area = frame.area();
            ui::draw(frame, app, &palette, pieces);
        })?;
        // The help scrolls against what the screen shows.
        app.screen = area;
        // While a clock runs, wake up to redraw it (and to see a flag fall) even when
        // nothing happens.
        let first = if app.clock_running() {
            match rx.recv_timeout(CLOCK_REDRAW) {
                Ok(event) => Some(event),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
        } else {
            match rx.recv() {
                Ok(event) => Some(event),
                Err(_) => return Ok(()),
            }
        };
        // The time comes first, so a move is charged to the clock as it stands now.
        app.tick(Instant::now(), engine_of(node, &mut NoEngine));
        let aspect = pieces.cell_aspect();
        if let Some(first) = first {
            handle(first, app, node, area, aspect);
        }
        while let Ok(event) = rx.try_recv() {
            handle(event, app, node, area, aspect);
        }
    }
    Ok(())
}

fn handle(event: Event, app: &mut App, node: &mut Option<Node>, area: Rect, aspect: Option<f32>) {
    let mut none = NoEngine;
    let node = engine_of(node, &mut none);
    match event {
        Event::Term(TermEvent::Key(key)) => app.handle_key(key, node),
        Event::Term(TermEvent::Mouse(mouse)) if app.show_help => match mouse.kind {
            MouseEventKind::ScrollDown => app.scroll_help(WHEEL_LINES),
            MouseEventKind::ScrollUp => app.scroll_help(-WHEEL_LINES),
            _ => {}
        },
        Event::Term(TermEvent::Mouse(mouse)) => {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                && let Some(square) = ui::square_at(
                    area,
                    aspect,
                    app.zoomed,
                    app.flipped,
                    mouse.column,
                    mouse.row,
                )
            {
                app.handle_click(square, node);
            }
        }
        // A resize just needs the redraw that follows every event.
        Event::Term(_) => {}
        Event::Node(event) => app.handle_node(event, node),
    }
}
