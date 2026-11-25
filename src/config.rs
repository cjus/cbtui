//! The settings file, `~/.cbtuirc`: the settings cbtui last ran with, so the next run starts
//! where this one left off.
//!
//! ## How it works
//!
//! The file is plain `key = value` lines, with `#` comments, written so a person can edit it
//! too. At start, cbtui reads it ([`Config::parse`]) and the command line overrides it
//! ([`crate::args::parse`] takes it as its defaults). While running, the event loop writes it
//! again whenever a saved setting changes, by an option or a `:` command ([`Config::from_app`],
//! [`Config::to_text`]).
//!
//! `$CBTUI_CONFIG` names another file, for trying settings out or keeping cbtui's tests away
//! from the real one.
//!
//! ## Pitfalls
//!
//! - A line cbtui can't read is skipped with a warning on the status line, never an error: a
//!   typo in the file mustn't stop cbtui starting. Writing the file drops it.
//! - The engine (`engine`, `engine-arg`) is read but only ever written back as it was read: an
//!   `--engine` given once, or `$CBTUI_ENGINE`, isn't made permanent behind the person's back.
//! - Which sides the engine plays is saved from what the person chose, not what a PGN viewer
//!   or an analysis set for a while ([`App::saved_play`]).

use std::path::PathBuf;

use crate::app::{App, MAX_MOVETIME_MS, Settings};
use crate::clock::TimeControl;
use crate::pieces::PieceChoice;
use crate::theme::Theme;

/// Names the settings file instead of `~/.cbtuirc`.
pub const CONFIG_ENV: &str = "CBTUI_CONFIG";

/// The settings file's name in the home directory.
pub const CONFIG_FILE: &str = ".cbtuirc";

/// What the file starts with.
const HEADER: &str = "\
# cbtui's settings, saved as they change. Edit freely: one `key = value` a line.
# Options on the command line win over these; see `cbtui --help`.
";

/// The settings in the file. A setting the file doesn't have is `None`, and keeps its default.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    /// The engine program.
    pub engine: Option<String>,
    /// Its arguments, in order.
    pub engine_args: Vec<String>,
    /// Which sides the engine plays, by [`crate::rules::Color::index`].
    pub play: Option<[bool; 2]>,
    /// The engine's level: `Some(None)` is full strength.
    pub level: Option<Option<f64>>,
    /// How long the engine thinks a move, in milliseconds.
    pub movetime_ms: Option<u64>,
    /// The time control: `Some(None)` is no clock.
    pub clock: Option<Option<TimeControl>>,
    /// How many lines an analysis shows.
    pub lines: Option<u32>,
    /// The board's colours.
    pub theme: Option<Theme>,
    /// How pieces are drawn.
    pub pieces: Option<PieceChoice>,
    /// Whether the board fills the screen.
    pub zoom: Option<bool>,
    /// Whether Black is at the bottom.
    pub flip: Option<bool>,
    /// Whether the engine thinks on the person's time.
    pub ponder: Option<bool>,
}

impl Config {
    /// Reads a settings file's text. The second value has a warning for each line it couldn't
    /// read, which it skipped.
    pub fn parse(text: &str) -> (Config, Vec<String>) {
        let mut config = Config::default();
        let mut warnings = Vec::new();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                warnings.push(format!("line {}: `{line}` isn't `key = value`", number + 1));
                continue;
            };
            if let Err(e) = config.set(key.trim(), value.trim()) {
                warnings.push(format!("line {}: {e}", number + 1));
            }
        }
        (config, warnings)
    }

    /// Sets `key` from its text, or says why it can't.
    fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let bad = |what: &str| format!("{key} takes {what}, not `{value}`");
        match key {
            "engine" => self.engine = Some(value.to_string()),
            "engine-arg" => self.engine_args.push(value.to_string()),
            "play" => self.play = Some(parse_play(value).ok_or_else(|| bad(PLAY_NAMES))?),
            "level" => {
                self.level = Some(match value {
                    "max" => None,
                    _ => Some(value.parse().map_err(|_| bad("a number or max"))?),
                })
            }
            "movetime" => {
                self.movetime_ms = Some(
                    value
                        .parse()
                        .ok()
                        .filter(|ms| (1..=MAX_MOVETIME_MS).contains(ms))
                        .ok_or_else(|| bad(&format!("1 to {MAX_MOVETIME_MS} milliseconds")))?,
                )
            }
            "clock" => {
                self.clock = Some(match value {
                    "off" => None,
                    _ => Some(TimeControl::parse(value).map_err(|e| format!("clock: {e}"))?),
                })
            }
            "lines" => {
                self.lines = Some(
                    value
                        .parse()
                        .ok()
                        .filter(|n| *n >= 1)
                        .ok_or_else(|| bad("a number from 1"))?,
                )
            }
            "theme" => self.theme = Some(Theme::parse(value).ok_or_else(|| bad(&Theme::names()))?),
            "pieces" => {
                self.pieces = Some(PieceChoice::parse(value).ok_or_else(|| bad(PIECE_NAMES))?)
            }
            "zoom" => self.zoom = Some(parse_on(value).ok_or_else(|| bad("on or off"))?),
            "flip" => self.flip = Some(parse_on(value).ok_or_else(|| bad("on or off"))?),
            "ponder" => self.ponder = Some(parse_on(value).ok_or_else(|| bad("on or off"))?),
            _ => return Err(format!("`{key}` isn't a setting")),
        }
        Ok(())
    }

    /// `settings` with what the file sets put in.
    pub fn apply(&self, mut settings: Settings) -> Settings {
        if let Some(play) = self.play {
            settings.engine_plays = play;
        }
        if let Some(level) = self.level {
            settings.level = level;
        }
        if let Some(ms) = self.movetime_ms {
            settings.movetime_ms = ms;
        }
        if let Some(clock) = self.clock {
            settings.clock = clock;
        }
        if let Some(lines) = self.lines {
            settings.lines = lines;
        }
        if let Some(theme) = self.theme {
            settings.theme = theme;
        }
        if let Some(ponder) = self.ponder {
            settings.ponder = ponder;
        }
        settings
    }

    /// The settings to save for `app`, drawn with `pieces`: everything as it is now, except the
    /// engine, kept as `previous` had it, and which sides the engine plays while a viewer or an
    /// analysis has changed that for a while.
    pub fn from_app(app: &App, pieces: PieceChoice, previous: &Config) -> Config {
        let settings = &app.settings;
        Config {
            engine: previous.engine.clone(),
            engine_args: previous.engine_args.clone(),
            play: app.saved_play().or(previous.play),
            level: Some(settings.level),
            movetime_ms: Some(settings.movetime_ms),
            clock: Some(settings.clock),
            lines: Some(settings.lines),
            theme: Some(settings.theme),
            pieces: Some(pieces),
            zoom: Some(app.zoomed),
            flip: Some(app.flipped),
            ponder: Some(settings.ponder),
        }
    }

    /// The file's text: a comment saying what it is, then each setting it has.
    pub fn to_text(&self) -> String {
        let mut lines = vec![HEADER.trim_end().to_string()];
        let mut put = |key: &str, value: String| lines.push(format!("{key} = {value}"));
        if let Some(engine) = &self.engine {
            put("engine", engine.clone());
        }
        for arg in &self.engine_args {
            put("engine-arg", arg.clone());
        }
        if let Some(play) = self.play {
            put("play", play_name(play).to_string());
        }
        if let Some(level) = self.level {
            put("level", level.map_or("max".to_string(), |l| l.to_string()));
        }
        if let Some(ms) = self.movetime_ms {
            put("movetime", ms.to_string());
        }
        if let Some(clock) = self.clock {
            put(
                "clock",
                clock.map_or("off".to_string(), |tc| tc.to_string()),
            );
        }
        if let Some(lines) = self.lines {
            put("lines", lines.to_string());
        }
        if let Some(theme) = self.theme {
            put("theme", theme.to_string());
        }
        if let Some(pieces) = self.pieces {
            put("pieces", pieces.name().to_string());
        }
        if let Some(zoom) = self.zoom {
            put("zoom", on_name(zoom).to_string());
        }
        if let Some(flip) = self.flip {
            put("flip", on_name(flip).to_string());
        }
        if let Some(ponder) = self.ponder {
            put("ponder", on_name(ponder).to_string());
        }
        lines.join("\n") + "\n"
    }
}

/// Where the settings file is: `$CBTUI_CONFIG`, else `.cbtuirc` in the home directory
/// (`$HOME`, or on Windows `%USERPROFILE%`). `None` with no home.
pub fn path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(CONFIG_ENV).filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    // Windows has no `HOME`, as a rule; its home directory is `USERPROFILE`.
    ["HOME", "USERPROFILE"]
        .iter()
        .find_map(|name| std::env::var_os(name).filter(|h| !h.is_empty()))
        .map(|home| PathBuf::from(home).join(CONFIG_FILE))
}

/// What `play` takes, for messages.
pub const PLAY_NAMES: &str = "white, black, both or none";

/// What `pieces` takes, for messages.
pub const PIECE_NAMES: &str = "auto, letters or symbols";

/// Which sides the engine plays, from its name: `white`, `black`, `both`, `none`.
pub fn parse_play(name: &str) -> Option<[bool; 2]> {
    match name {
        "white" | "w" => Some([true, false]),
        "black" | "b" => Some([false, true]),
        "both" => Some([true, true]),
        "none" => Some([false, false]),
        _ => None,
    }
}

/// The name of which sides the engine plays.
pub fn play_name(play: [bool; 2]) -> &'static str {
    match play {
        [true, false] => "white",
        [false, true] => "black",
        [true, true] => "both",
        [false, false] => "none",
    }
}

fn parse_on(text: &str) -> Option<bool> {
    match text {
        "on" | "true" | "yes" => Some(true),
        "off" | "false" | "no" => Some(false),
        _ => None,
    }
}

fn on_name(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_reads_and_writes_back_the_same() {
        let text = "\
# a comment
engine = /opt/cb2027
engine-arg = --cnp
engine-arg = --threads=4
play = white
level = 8
movetime = 500
clock = 5+3
lines = 4
theme = dark
pieces = letters
zoom = on
flip = off
ponder = on
";
        let (config, warnings) = Config::parse(text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(config.engine.as_deref(), Some("/opt/cb2027"));
        assert_eq!(config.engine_args, ["--cnp", "--threads=4"]);
        assert_eq!(config.play, Some([true, false]));
        assert_eq!(config.level, Some(Some(8.0)));
        assert_eq!(config.clock, Some(Some(TimeControl::parse("5+3").unwrap())));
        assert_eq!(config.theme, Some(Theme::Dark));
        assert_eq!(config.zoom, Some(true));
        assert_eq!(config.ponder, Some(true));
        // Written out and read again, nothing changes.
        let (again, _) = Config::parse(&config.to_text());
        assert_eq!(again, config);
        assert!(config.to_text().starts_with("# cbtui's settings"));
    }

    #[test]
    fn a_bad_line_is_skipped_with_a_warning() {
        let (config, warnings) =
            Config::parse("theme = pink\nnonsense\ncolour = red\nlevel = max\nclock = off\n");
        assert_eq!(config.theme, None);
        assert_eq!(config.level, Some(None));
        assert_eq!(config.clock, Some(None));
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(
            warnings[0].starts_with("line 1: theme takes"),
            "{warnings:?}"
        );
        assert!(warnings[1].contains("isn't `key = value`"), "{warnings:?}");
        assert!(
            warnings[2].contains("`colour` isn't a setting"),
            "{warnings:?}"
        );
    }

    #[test]
    fn the_file_sets_the_defaults() {
        let (config, _) = Config::parse("play = none\nmovetime = 250\ntheme = blue\n");
        let settings = config.apply(Settings::default());
        assert_eq!(settings.engine_plays, [false, false]);
        assert_eq!(settings.movetime_ms, 250);
        assert_eq!(settings.theme, Theme::Blue);
        // What it doesn't say keeps its default.
        assert_eq!(settings.lines, Settings::default().lines);
    }

    #[test]
    fn what_is_saved_keeps_the_engine_and_the_chosen_sides() {
        use crate::app::NodeSummary;
        let previous = Config {
            engine: Some("/opt/cb2027".into()),
            ..Config::default()
        };
        let mut app = App::new(NodeSummary::absent(), Settings::default()).unwrap();
        app.zoomed = true;
        let saved = Config::from_app(&app, PieceChoice::Symbols, &previous);
        assert_eq!(saved.engine.as_deref(), Some("/opt/cb2027"));
        assert_eq!(saved.play, Some([false, true]));
        assert_eq!(saved.zoom, Some(true));
        assert_eq!(saved.pieces, Some(PieceChoice::Symbols));
    }
}
