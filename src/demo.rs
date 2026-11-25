//! The demo (`cbtui --demo`): a short game that shows cbtui's features by itself, for a first
//! look or a screen recording.
//!
//! ## How it works
//!
//! A [`Demo`] is a script of [`Step`]s: key presses, pauses, captions on the status line, and
//! waits for the engine's reply. The event loop calls [`Demo::run`] whenever it wakes, and
//! wakes at [`Demo::next_wake`] at the latest; `run` takes every step that's due. Keys go
//! through [`App::handle_key`], as a person's do, so the demo shows exactly what pressing them
//! shows. A typed move or command is typed a character at a time, so it can be seen.
//!
//! The script: a move with the arrow keys (the piece's squares showing), two typed moves, the
//! engine thinking and pondering, an analysis, the search at its root, the evaluation, the
//! board zoomed through every theme and turned around, and back. Then it's the person's game.
//!
//! ## Pitfalls
//!
//! - The engine's replies take as long as they take, so moves wait for the person's turn
//!   ([`Step::YourMove`]) rather than for a time.
//! - A step the engine can't do (an analysis without `progress`, say) shows the app's own
//!   explanation, as it would for a person, and the script carries on.
//! - Any key or click stops the demo. That key is used up, so a `q` meant to stop it doesn't
//!   also quit, and half-typed text is cleared.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::app::App;
use crate::node::Engine;
use crate::theme::Theme;

/// Between the characters of something typed.
pub const TYPING: Duration = Duration::from_millis(70);
/// Between the arrow keys that walk the cursor.
pub const CURSOR_STEP: Duration = Duration::from_millis(250);
/// A short pause, to see what a key did.
pub const BEAT: Duration = Duration::from_millis(700);
/// Long enough to take in a screen.
pub const LOOK: Duration = Duration::from_millis(2_000);
/// Long enough for a search to fill its panel.
pub const WATCH: Duration = Duration::from_millis(3_500);
/// How long each theme shows.
pub const THEME_SHOWN: Duration = Duration::from_millis(1_200);
/// How often a wait for the engine looks again, besides when its messages arrive.
pub const POLL: Duration = Duration::from_millis(100);

/// One thing the demo does.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Press a key.
    Key(KeyCode),
    /// Wait this long.
    Pause(Duration),
    /// Put this on the status line.
    Say(String),
    /// Wait until it's the person's move and the engine has answered.
    YourMove,
}

/// The demo's progress through its script.
#[derive(Clone, Debug)]
pub struct Demo {
    steps: VecDeque<Step>,
    next_at: Instant,
}

impl Demo {
    /// The demo, starting at `now`, for a session whose board starts as `theme` (which it
    /// goes back to at the end).
    pub fn new(now: Instant, theme: Theme) -> Demo {
        Demo {
            steps: script(theme).into(),
            next_at: now,
        }
    }

    /// When it next has something to do; `None` once it's done.
    pub fn next_wake(&self) -> Option<Instant> {
        (!self.steps.is_empty()).then_some(self.next_at)
    }

    /// Whether the script has run to its end.
    pub fn finished(&self) -> bool {
        self.steps.is_empty()
    }

    /// Takes every step due at `now`.
    pub fn run(&mut self, now: Instant, app: &mut App, engine: &mut dyn Engine) {
        while now >= self.next_at {
            let Some(step) = self.steps.front().cloned() else {
                return;
            };
            match step {
                Step::Key(code) => {
                    app.handle_key(KeyEvent::from(code), engine);
                    // Typing goes at typing speed; anything else waits for its own pause.
                    self.next_at = now + TYPING;
                }
                Step::Pause(pause) => self.next_at = now + pause,
                Step::Say(text) => app.status = text,
                Step::YourMove => {
                    if app.engine_to_move() || app.searching() {
                        self.next_at = now + POLL;
                        return;
                    }
                }
            }
            self.steps.pop_front();
        }
    }

    /// Stops the demo for the person to take over: clears anything half-typed and says so.
    pub fn interrupt(self, app: &mut App) {
        app.input = None;
        app.status = "Demo stopped: it's your game now. Press ? for help.".into();
    }
}

/// The steps that type `text` and press Enter.
fn typed(text: &str) -> Vec<Step> {
    text.chars()
        .map(|c| Step::Key(KeyCode::Char(c)))
        .chain([Step::Key(KeyCode::Enter)])
        .collect()
}

/// The steps that press `key`, caption it, and leave it on screen for `pause`.
fn press(key: char, caption: &str, pause: Duration) -> Vec<Step> {
    vec![
        Step::Key(KeyCode::Char(key)),
        Step::Say(caption.into()),
        Step::Pause(pause),
    ]
}

/// The whole demo, for a board that starts as `theme`.
pub fn script(theme: Theme) -> Vec<Step> {
    use Step::{Key, Pause, Say, YourMove};
    let mut steps = vec![
        Say("The cbtui demo: any key stops it.".into()),
        Pause(LOOK),
        // The cursor starts on e2: pick the pawn up, showing where it can go, and walk it.
        Key(KeyCode::Enter),
        Say("Arrows and Enter move a piece; the dots show where it can go.".into()),
        Pause(BEAT),
        Key(KeyCode::Up),
        Pause(CURSOR_STEP),
        Key(KeyCode::Up),
        Pause(CURSOR_STEP),
        Key(KeyCode::Enter),
        YourMove,
        Pause(BEAT),
    ];
    // Developing moves that no reasonable reply makes bad (a bishop to c4 runs into ...d5).
    for mv in ["Nf3", "Nc3"] {
        steps.extend(typed(mv));
        steps.extend([YourMove, Pause(BEAT)]);
    }
    steps.extend([
        Say("The engine thinks on your time about the reply it expects.".into()),
        Pause(LOOK),
    ]);
    steps.extend(press('i', "i: analysis, several lines at once.", WATCH));
    steps.extend(press(
        'w',
        "w: the search at its root, and each move's share of it.",
        WATCH,
    ));
    steps.extend([
        Key(KeyCode::Char('w')),
        Key(KeyCode::Char('i')),
        Pause(BEAT),
    ]);
    steps.extend(press(
        'v',
        "v: the engine's evaluation, term by term.",
        LOOK,
    ));
    steps.extend([Key(KeyCode::Esc), Pause(BEAT)]);
    steps.extend(press('z', "z: zoom, the board fills the screen.", LOOK));
    let mut shown = theme;
    for _ in 1..Theme::ALL.len() {
        shown = shown.next();
        steps.extend(typed(&format!(":theme {shown}")));
        steps.push(Pause(THEME_SHOWN));
    }
    steps.extend(press('t', "t: turn the board around.", LOOK));
    steps.push(Key(KeyCode::Char('t')));
    steps.extend(typed(&format!(":theme {theme}")));
    steps.extend([
        Pause(BEAT),
        Key(KeyCode::Char('z')),
        Say("That was the demo. Play on, or press ? for help and q to quit.".into()),
    ]);
    steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{NodeSummary, Settings};
    use cnp::engine::{
        EngineCapabilities, EvaluateParams, SearchParams, TreeNodeParams, TreeParams,
    };

    /// Answers every request with a new id, and counts the searches.
    #[derive(Default)]
    struct Counter {
        searches: i64,
    }

    impl Engine for Counter {
        fn search(&mut self, _: &SearchParams) -> Result<i64, String> {
            self.searches += 1;
            Ok(self.searches)
        }
        fn stop(&mut self, _: i64) -> Result<(), String> {
            Ok(())
        }
        fn ponder_hit(&mut self, _: i64) -> Result<(), String> {
            Ok(())
        }
        fn evaluate(&mut self, _: &EvaluateParams) -> Result<i64, String> {
            Ok(1_000)
        }
        fn tree(&mut self, _: &TreeParams) -> Result<i64, String> {
            Ok(2_000)
        }
        fn tree_node(&mut self, _: &TreeNodeParams) -> Result<i64, String> {
            Ok(3_000)
        }
    }

    fn app() -> App {
        let node = NodeSummary {
            caps: EngineCapabilities {
                progress: true,
                ..EngineCapabilities::default()
            },
            ..NodeSummary::absent()
        };
        let mut app = App::new(node, Settings::default()).unwrap();
        app.has_engine = true;
        app
    }

    #[test]
    fn the_script_shows_the_features_and_ends_where_it_started() {
        let steps = script(Theme::Green);
        let keys: String = steps
            .iter()
            .filter_map(|s| match s {
                Step::Key(KeyCode::Char(c)) => Some(*c),
                _ => None,
            })
            .collect();
        for wanted in ["Nf3", "Nc3", "iw", "wi", "v", "z", ":theme dark", "t"] {
            assert!(keys.contains(wanted), "{wanted} in {keys}");
        }
        // Every other theme once, then back to the first.
        for theme in Theme::ALL {
            assert!(keys.contains(&format!(":theme {theme}")), "{theme}");
        }
        assert!(keys.ends_with(":theme greenz"), "{keys}");
    }

    #[test]
    fn moves_wait_for_the_engine_and_a_key_hands_over() {
        let (mut app, mut engine) = (app(), Counter::default());
        let start = Instant::now();
        let mut demo = Demo::new(start, Theme::Brown);
        // Long enough for the arrows to play e4, but the engine hasn't answered.
        let mut now = start;
        for _ in 0..200 {
            now += POLL;
            demo.run(now, &mut app, &mut engine);
        }
        let sans: Vec<&str> = app.game.moves().iter().map(|m| m.san.as_str()).collect();
        assert_eq!(sans, ["e4"]);
        assert!(app.searching());
        assert_eq!(demo.steps.front(), Some(&Step::YourMove));
        assert_eq!(demo.next_wake(), Some(now + POLL));

        // A key stops it, half-typed text and all.
        app.input = Some("Nf".into());
        demo.interrupt(&mut app);
        assert_eq!(app.input, None);
        assert!(app.status.starts_with("Demo stopped"), "{}", app.status);
    }
}
