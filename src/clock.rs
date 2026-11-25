//! Chess clocks: each side's time, with an increment after every move.
//!
//! ## How it works
//!
//! - **Time is an input.** [`Clocks`] never reads the system clock: every method that needs
//!   the time takes `now`, an [`Instant`] the caller passes in. The app gets it from the event
//!   loop (`App::tick`), and tests move time by hand.
//! - **The clocks start with the game's first move.** Until then neither runs, so a person
//!   can read the board, and an engine playing White opens on its own time budget.
//! - **A move presses the clock** ([`Clocks::press`]): the mover is charged the time since
//!   their clock started, gets the increment, and the opponent's clock starts.
//! - **A flag falls** when the running clock reaches zero ([`Clocks::flagged`]). What that
//!   means for the game (a loss, or a draw against a lone king) is the rules library's call.
//!
//! ## Pitfalls
//!
//! - An engine is charged from when cbtui pressed its clock until cbtui handles its answer,
//!   so it pays for the pipe and for cbtui's own work, a few milliseconds a move. That's the
//!   honest figure: it's the time that passed on the board.
//! - Time left is read when the screen is drawn, so the display only moves when something
//!   redraws it. The event loop redraws often while a clock runs.

use std::fmt;
use std::time::Instant;

use crate::rules::Color;

/// The longest starting time a clock may have, in minutes.
pub const MAX_BASE_MINUTES: f64 = 600.0;

/// The largest increment, in seconds.
pub const MAX_INCREMENT_SECONDS: f64 = 600.0;

/// Below this much time a clock shows tenths of a second, in milliseconds.
pub const TENTHS_BELOW_MS: u64 = 20_000;

const MS_PER_SECOND: u64 = 1000;
const MS_PER_MINUTE: u64 = 60 * MS_PER_SECOND;
const MS_PER_HOUR: u64 = 60 * MS_PER_MINUTE;

/// A time control: how long each side starts with, and what each move adds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeControl {
    /// Each side's starting time, in milliseconds.
    pub base_ms: u64,
    /// Added to the mover's clock after each move, in milliseconds.
    pub increment_ms: u64,
}

impl TimeControl {
    /// Reads `minutes+seconds`, the way games are usually described: `5+3` is five minutes
    /// each and three seconds a move, `0.5+0` thirty seconds each, and `10` ten minutes with
    /// no increment.
    pub fn parse(text: &str) -> Result<TimeControl, String> {
        let usage = || format!("a clock is <minutes>+<seconds>, like 5+3, not `{text}`");
        let (minutes, seconds) = text.split_once('+').unwrap_or((text, "0"));
        let minutes: f64 = minutes.trim().parse().map_err(|_| usage())?;
        let seconds: f64 = seconds.trim().parse().map_err(|_| usage())?;
        if !(minutes > 0.0 && minutes <= MAX_BASE_MINUTES) {
            return Err(format!(
                "a clock starts with more than 0 and at most {MAX_BASE_MINUTES} minutes, not `{text}`"
            ));
        }
        if !(0.0..=MAX_INCREMENT_SECONDS).contains(&seconds) {
            return Err(format!(
                "an increment is 0 to {MAX_INCREMENT_SECONDS} seconds, not `{text}`"
            ));
        }
        Ok(TimeControl {
            base_ms: (minutes * MS_PER_MINUTE as f64).round() as u64,
            increment_ms: (seconds * MS_PER_SECOND as f64).round() as u64,
        })
    }

    /// The time control as PGN's `TimeControl` tag writes it: seconds, then the increment in
    /// seconds (`300+3`).
    pub fn pgn_tag(&self) -> String {
        format!(
            "{}+{}",
            self.base_ms / MS_PER_SECOND,
            self.increment_ms / MS_PER_SECOND
        )
    }
}

impl fmt::Display for TimeControl {
    /// `5+3`: minutes, then the increment in seconds; fractions only when there are some.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let minutes = self.base_ms as f64 / MS_PER_MINUTE as f64;
        let seconds = self.increment_ms as f64 / MS_PER_SECOND as f64;
        write!(f, "{minutes}+{seconds}")
    }
}

/// Both sides' clocks.
#[derive(Clone, Debug, PartialEq)]
pub struct Clocks {
    control: TimeControl,
    /// Each side's time when its clock last stopped, in milliseconds, by [`Color::index`].
    left: [u64; 2],
    /// The clock that is running, and since when.
    running: Option<(Color, Instant)>,
}

impl Clocks {
    /// Both clocks at the control's starting time, neither running.
    pub fn new(control: TimeControl) -> Clocks {
        Clocks {
            control,
            left: [control.base_ms; 2],
            running: None,
        }
    }

    /// The time control.
    pub fn control(&self) -> TimeControl {
        self.control
    }

    /// The clock that is running, if one is.
    pub fn running(&self) -> Option<Color> {
        self.running.map(|(color, _)| color)
    }

    /// `color`'s time left at `now`, in milliseconds; never below zero.
    pub fn left(&self, color: Color, now: Instant) -> u64 {
        let stored = self.left[color.index()];
        match self.running {
            Some((running, since)) if running == color => {
                stored.saturating_sub(elapsed_ms(since, now))
            }
            _ => stored,
        }
    }

    /// Both sides' time left at `now`, by [`Color::index`]: what to keep to put the clocks
    /// back after a takeback ([`Clocks::set_times`]).
    pub fn times(&self, now: Instant) -> [u64; 2] {
        [self.left(Color::White, now), self.left(Color::Black, now)]
    }

    /// Sets both sides' time, and stops the clocks.
    pub fn set_times(&mut self, times: [u64; 2]) {
        self.left = times;
        self.running = None;
    }

    /// `mover` has moved: charges them the time since their clock started (nothing on the
    /// game's first move, when no clock runs yet), adds the increment, and starts the
    /// opponent's clock.
    pub fn press(&mut self, mover: Color, now: Instant) {
        self.stop(now);
        let left = &mut self.left[mover.index()];
        *left = left.saturating_add(self.control.increment_ms);
        self.running = Some((!mover, now));
    }

    /// Starts `color`'s clock (after a takeback, say), stopping the other one.
    pub fn start(&mut self, color: Color, now: Instant) {
        self.stop(now);
        self.running = Some((color, now));
    }

    /// Stops the running clock, charging its side the time it ran.
    pub fn stop(&mut self, now: Instant) {
        if let Some((color, _)) = self.running {
            self.left[color.index()] = self.left(color, now);
            self.running = None;
        }
    }

    /// The side whose flag has fallen at `now`: the running clock, if it has reached zero.
    pub fn flagged(&self, now: Instant) -> Option<Color> {
        self.running().filter(|&color| self.left(color, now) == 0)
    }
}

fn elapsed_ms(since: Instant, now: Instant) -> u64 {
    let elapsed = now.saturating_duration_since(since);
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

/// A clock's time as people read it: `1:05:00`, `4:52`, and under [`TENTHS_BELOW_MS`]
/// with tenths, `0:09.3`. Seconds are rounded down, as on a chess clock.
pub fn clock_text(ms: u64) -> String {
    let hours = ms / MS_PER_HOUR;
    let minutes = ms % MS_PER_HOUR / MS_PER_MINUTE;
    let seconds = ms % MS_PER_MINUTE / MS_PER_SECOND;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else if ms < TENTHS_BELOW_MS {
        let tenths = ms % MS_PER_SECOND / (MS_PER_SECOND / 10);
        format!("{minutes}:{seconds:02}.{tenths}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn reads_time_controls_as_minutes_plus_seconds() {
        let tc = TimeControl::parse("5+3").unwrap();
        assert_eq!((tc.base_ms, tc.increment_ms), (300_000, 3000));
        assert_eq!(tc.to_string(), "5+3");
        assert_eq!(tc.pgn_tag(), "300+3");
        assert_eq!(TimeControl::parse("0.5+0").unwrap().base_ms, 30_000);
        assert_eq!(TimeControl::parse("0.5").unwrap().to_string(), "0.5+0");
        assert_eq!(TimeControl::parse("10").unwrap().increment_ms, 0);
        assert!(TimeControl::parse("0+5").is_err());
        assert!(TimeControl::parse("blitz").unwrap_err().contains("blitz"));
        assert!(TimeControl::parse("5+-1").is_err());
    }

    #[test]
    fn clocks_start_with_the_first_move_and_add_the_increment() {
        let t0 = Instant::now();
        let mut clocks = Clocks::new(TimeControl::parse("1+2").unwrap());
        assert_eq!(clocks.running(), None);
        // White thinks 10 s over the first move: free, and the increment is added.
        clocks.press(Color::White, t0 + ms(10_000));
        assert_eq!(clocks.left(Color::White, t0 + ms(10_000)), 62_000);
        assert_eq!(clocks.running(), Some(Color::Black));
        // Black's clock runs from White's move.
        assert_eq!(clocks.left(Color::Black, t0 + ms(15_000)), 55_000);
        clocks.press(Color::Black, t0 + ms(15_000));
        assert_eq!(clocks.times(t0 + ms(16_000)), [61_000, 57_000]);
    }

    #[test]
    fn a_flag_falls_at_zero_and_time_never_goes_negative() {
        let t0 = Instant::now();
        let mut clocks = Clocks::new(TimeControl::parse("0.5").unwrap());
        clocks.press(Color::White, t0);
        assert_eq!(clocks.flagged(t0 + ms(29_999)), None);
        assert_eq!(clocks.flagged(t0 + ms(30_000)), Some(Color::Black));
        assert_eq!(clocks.left(Color::Black, t0 + ms(99_000)), 0);
        clocks.stop(t0 + ms(99_000));
        assert_eq!(clocks.running(), None);
        assert_eq!(clocks.flagged(t0 + ms(99_000)), None);
    }

    #[test]
    fn a_takeback_puts_the_times_back() {
        let t0 = Instant::now();
        let mut clocks = Clocks::new(TimeControl::parse("1").unwrap());
        clocks.press(Color::White, t0);
        let before = clocks.times(t0 + ms(5000));
        clocks.press(Color::Black, t0 + ms(5000));
        clocks.set_times(before);
        assert_eq!(clocks.running(), None);
        clocks.start(Color::Black, t0 + ms(9000));
        assert_eq!(clocks.left(Color::Black, t0 + ms(10_000)), 54_000);
    }

    #[test]
    fn times_read_as_on_a_chess_clock() {
        assert_eq!(clock_text(300_000), "5:00");
        assert_eq!(clock_text(292_999), "4:52");
        assert_eq!(clock_text(3_900_000), "1:05:00");
        assert_eq!(clock_text(9_350), "0:09.3");
        assert_eq!(clock_text(0), "0:00.0");
    }
}
