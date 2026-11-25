//! The command line.

use crate::app::{MAX_MOVETIME_MS, Settings};

/// The engine started when none is named: `cb2026` from the `PATH`, in CNP mode.
pub const DEFAULT_ENGINE: &str = "cb2026";

/// The engine's arguments when none are given.
pub const DEFAULT_ENGINE_ARGS: &[&str] = &["--cnp"];

/// Names the engine when `--engine` doesn't.
pub const ENGINE_ENV: &str = "CBTUI_ENGINE";

/// The help text.
pub const USAGE: &str = "\
cbtui: play chess in the terminal against an engine that speaks the Chess Node Protocol.

Usage: cbtui [options]

Options:
  --engine <path>        The engine to start (default: $CBTUI_ENGINE, else `cb2026` on the PATH)
  --engine-arg <arg>     An argument for the engine; repeat for more (default: --cnp)
  --play <side>          Which sides the engine plays: white, black (default), both, none
  --level <n>            The engine's playing level, if it has levels (default: full strength)
  --movetime <ms>        How long the engine thinks a move (default: 1000)
  --fen <FEN>            Start from this position
  --ascii                Draw pieces as letters
  -h, --help             This help
  -V, --version          The version";

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Args {
    /// Play, with this engine and these settings.
    Run {
        /// The engine program.
        engine: String,
        /// Its arguments.
        engine_args: Vec<String>,
        /// The session's settings.
        settings: Settings,
    },
    /// Print the help.
    Help,
    /// Print the version.
    Version,
}

/// Reads the arguments (without the program name). `env_engine` is `$CBTUI_ENGINE`.
pub fn parse(
    args: impl IntoIterator<Item = String>,
    env_engine: Option<String>,
) -> Result<Args, String> {
    let mut engine = env_engine.filter(|e| !e.is_empty());
    let mut engine_args: Vec<String> = Vec::new();
    let mut settings = Settings::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "-h" | "--help" => return Ok(Args::Help),
            "-V" | "--version" => return Ok(Args::Version),
            "--engine" => engine = Some(value("--engine")?),
            "--engine-arg" => engine_args.push(value("--engine-arg")?),
            "--play" => {
                settings.engine_plays = match value("--play")?.as_str() {
                    "white" => [true, false],
                    "black" => [false, true],
                    "both" => [true, true],
                    "none" => [false, false],
                    other => {
                        return Err(format!(
                            "--play takes white, black, both or none, not `{other}`"
                        ));
                    }
                }
            }
            "--level" => {
                let text = value("--level")?;
                let level = text
                    .parse::<f64>()
                    .map_err(|_| format!("--level takes a number, not `{text}`"))?;
                settings.level = Some(level);
            }
            "--movetime" => {
                let text = value("--movetime")?;
                settings.movetime_ms = text
                    .parse::<u64>()
                    .ok()
                    .filter(|ms| (1..=MAX_MOVETIME_MS).contains(ms))
                    .ok_or_else(|| {
                        format!(
                            "--movetime takes 1 to {MAX_MOVETIME_MS} milliseconds, not `{text}`"
                        )
                    })?;
            }
            "--fen" => settings.fen = Some(value("--fen")?),
            "--ascii" => settings.ascii = true,
            other => return Err(format!("unknown option `{other}` (try --help)")),
        }
    }
    if engine_args.is_empty() {
        engine_args = DEFAULT_ENGINE_ARGS.iter().map(|s| s.to_string()).collect();
    }
    Ok(Args::Run {
        engine: engine.unwrap_or_else(|| DEFAULT_ENGINE.to_string()),
        engine_args,
        settings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str], env: Option<&str>) -> Result<Args, String> {
        parse(args.iter().map(|s| s.to_string()), env.map(String::from))
    }

    #[test]
    fn defaults_to_cb2026_in_cnp_mode_playing_black() {
        let Args::Run {
            engine,
            engine_args,
            settings,
        } = run(&[], None).unwrap()
        else {
            panic!()
        };
        assert_eq!(engine, "cb2026");
        assert_eq!(engine_args, ["--cnp"]);
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn the_engine_can_be_anything_that_speaks_cnp() {
        let Args::Run {
            engine,
            engine_args,
            ..
        } = run(
            &["--engine", "/opt/cb2027", "--engine-arg", "--protocol=cnp"],
            Some("x"),
        )
        .unwrap()
        else {
            panic!()
        };
        assert_eq!(engine, "/opt/cb2027");
        assert_eq!(engine_args, ["--protocol=cnp"]);

        let Args::Run { engine, .. } = run(&[], Some("/usr/local/bin/cb2027")).unwrap() else {
            panic!()
        };
        assert_eq!(engine, "/usr/local/bin/cb2027");
    }

    #[test]
    fn reads_the_settings() {
        let Args::Run { settings, .. } = run(
            &[
                "--play",
                "white",
                "--level",
                "8",
                "--movetime",
                "250",
                "--ascii",
            ],
            None,
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(settings.engine_plays, [true, false]);
        assert_eq!(settings.level, Some(8.0));
        assert_eq!(settings.movetime_ms, 250);
        assert!(settings.ascii);
    }

    #[test]
    fn mistakes_say_what_was_wrong() {
        assert!(run(&["--play", "red"], None).unwrap_err().contains("red"));
        assert!(run(&["--movetime", "0"], None).is_err());
        assert!(
            run(&["--level"], None)
                .unwrap_err()
                .contains("needs a value")
        );
        assert!(run(&["--bogus"], None).unwrap_err().contains("--bogus"));
        assert_eq!(run(&["--help"], None), Ok(Args::Help));
    }
}
