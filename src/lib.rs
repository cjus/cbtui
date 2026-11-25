//! # cbtui: a terminal chess client for Chess Node Protocol engines
//!
//! cbtui is a full-screen terminal program for playing chess against an engine. It doesn't
//! play chess itself: it starts an engine (cb2026 by default, or any engine that speaks the
//! [Chess Node Protocol](https://github.com/cjus/chess-node-protocol)) as a child process and
//! asks it for moves. Everything it offers comes from what the engine says it can do, so a
//! newer engine works unchanged.
//!
//! The parts, each with its own documentation:
//!
//! - [`node`]: the engine process and the CNP conversation with it;
//! - [`rules`]: legality, notation and game endings, from a rules library;
//! - [`clock`]: chess clocks with an increment;
//! - [`analysis`]: the engine's lines while it analyses the position on the board;
//! - [`app`]: the state of a session, and what input and engine messages do to it;
//! - [`review`]: a game read from PGN, with its variations, to step through;
//! - [`ui`]: drawing it with [Ratatui](https://ratatui.rs);
//! - [`pieces`]: pieces as pictures where the terminal can show them, else as letters;
//! - [`args`]: the command line.
//!
//! The binary (`src/main.rs`) ties them together: it runs the terminal, and a loop that
//! takes key presses, mouse clicks and engine messages from one channel, one at a time.

pub mod analysis;
pub mod app;
pub mod args;
pub mod clock;
pub mod node;
pub mod pieces;
pub mod review;
pub mod rules;
pub mod ui;
