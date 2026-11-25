# Status

What is merged, what is in flight, and how to carry on. Keep it current in every PR that
changes the picture.

## Where things are (2026-11-24)

- **M1 · Playable: built**, in the first PR. A game against cb2026 over CNP: board with mouse,
  arrow keys and typed SAN or coordinates; the engine plays white, black, both or none; levels
  with their ratings (from `engine/levels`) and time per move; live thinking (depth, score,
  nodes, speed, PV in SAN); move list; undo, new game, FEN, flip; PGN save; help overlay.
- **Piece pictures: built** (second PR). On terminals with an image protocol the pieces are
  the Cburnett pictures, filling square squares, and the board fills its panel; elsewhere and
  in tmux they are two letters (`WK`). Checked in Ghostty by Carlos (2026-11-24, the README's
  screenshot); kitty, iTerm2 and WezTerm not tried yet.
- Known: ratatui-image's terminal query leaves a thread reading stdin when the terminal
  doesn't answer in time, which swallows a key. cbtui doesn't ask inside tmux, where that
  happened; elsewhere it says so on the status line if it happens.
- Tests: 37 unit tests (the app against a fake engine, the CNP client against a fake node over
  pipes, the layout and mouse mapping with Ratatui's test backend), and 2 against a real cb2026
  (`tests/engine.rs`, run when `CBTUI_TEST_ENGINE` is set; CI builds cb2026 at the pinned
  commit and sets it).

## Pins

| What | Where | Pinned to |
| --- | --- | --- |
| `cb2026-chess` (rules) and the engine CI tests against | `Cargo.toml` | chessengine `f41bbcc` |
| `cnp` (protocol types) | `Cargo.toml` | chess-node-protocol `ba9e0ce` (the same as chessengine at `f41bbcc`) |
| Rust | `rust-toolchain.toml` | 1.97.0 |

Bump the chessengine and cnp pins together, to a chessengine `main` commit and the cnp commit
that chessengine pins there.

## Next

M2 (`docs/PRD.md`): clocks with increment, resign and draw offers, PGN load, a promotion
picker, stepping through the move list.

## How to carry on

1. Read `CLAUDE.md` and `docs/PRD.md`.
2. Build an engine to test against (see `scripts/check.sh`), and run
   `CBTUI_TEST_ENGINE=… scripts/check.sh`.
3. Try it for real: `CBTUI_ENGINE=… cargo run --release`.
