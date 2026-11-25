//! The `cbtui` program: starts the engine, takes over the terminal, and runs the event loop.

use std::io;
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use cbtui::app::{App, NodeSummary};
use cbtui::args::{self, Args, ENGINE_ENV, USAGE};
use cbtui::node::{Node, NodeEvent};
use cbtui::pieces::{PieceChoice, Pieces};
use cbtui::ui::{self, Palette};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as TermEvent, MouseButton, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::Rect;
use ratatui_image::picker::Picker;

/// Everything the loop reacts to, from one channel.
enum Event {
    /// A key, a click or a resize.
    Term(TermEvent),
    /// Something from the engine.
    Node(NodeEvent),
}

fn main() -> ExitCode {
    let parsed = args::parse(std::env::args().skip(1), std::env::var(ENGINE_ENV).ok());
    let (engine, engine_args, settings, piece_choice) = match parsed {
        Ok(Args::Run {
            engine,
            engine_args,
            settings,
            pieces,
        }) => (engine, engine_args, settings, pieces),
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

    let (tx, rx) = mpsc::channel();
    let node_tx = tx.clone();
    // Connect before taking over the terminal, so a missing engine is a plain error message.
    let mut node = match Node::spawn(&engine, &engine_args, move |event| {
        let _ = node_tx.send(Event::Node(event));
    }) {
        Ok(node) => node,
        Err(e) => {
            eprintln!("cbtui: {e}");
            eprintln!("Name the engine with --engine <path> or ${ENGINE_ENV}.");
            return ExitCode::FAILURE;
        }
    };
    let summary = NodeSummary::from_info(node.info());
    let mut app = match App::new(summary, settings) {
        Ok(app) => app,
        Err(e) => {
            eprintln!("cbtui: {e}");
            node.shutdown();
            return ExitCode::FAILURE;
        }
    };
    if app.node.caps.strength
        && let Ok(id) = node.levels()
    {
        app.expect_levels(id);
    }
    app.start(&mut node);

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
    node.shutdown();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cbtui: {e}");
            ExitCode::FAILURE
        }
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
    node: &mut Node,
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
        let Ok(first) = rx.recv() else {
            return Ok(());
        };
        let aspect = pieces.cell_aspect();
        handle(first, app, node, area, aspect);
        while let Ok(event) = rx.try_recv() {
            handle(event, app, node, area, aspect);
        }
    }
    Ok(())
}

fn handle(event: Event, app: &mut App, node: &mut Node, area: Rect, aspect: Option<f32>) {
    match event {
        Event::Term(TermEvent::Key(key)) => app.handle_key(key, node),
        Event::Term(TermEvent::Mouse(mouse)) => {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                && !app.show_help
                && let Some(square) =
                    ui::square_at(area, aspect, app.flipped, mouse.column, mouse.row)
            {
                app.handle_click(square, node);
            }
        }
        // A resize just needs the redraw that follows every event.
        Event::Term(_) => {}
        Event::Node(event) => app.handle_node(event, node),
    }
}
