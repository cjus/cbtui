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
- **Zoom: built** (fourth PR). `z` or `:zoom` gives the board the whole screen but the
  status line, which then ends with the last move, whose move it is and the engine's score.
  Letter pieces now grow past 7×3, up to 13×6, in either mode. A wide terminal is held back
  by its height, so zoom gains most in narrow or tall ones. Tried in tmux (2026-11-24).
- **Engine panels: tidied** (fifth PR). The engine's title shows its version with a short
  commit hash (`1.0.0+f41bbcc`); the whole version and the licence moved to the help screen
  (`?`). The engine panel grows to 10 rows where there is room (from 5), and a long game
  ending wraps instead of being cut off.
- **PGN viewer: built** (sixth PR). `--pgn <file>` opens the file's first game read-only,
  with no engine needed: arrows step through it, digits enter variations (nested ones too),
  and the panels show the tags, the moves with variations in brackets, and the comments and
  glyphs. `:play <side>` plays on from the position shown. In a game, `[` `]` (and Home,
  End) look back through the moves. Tried in tmux with and without cb2026 (2026-11-24).
- **Promotion picker: built** (seventh PR). A pawn moved to the last rank by click or
  arrows offers Q R B N on the promotion square and the three behind it; a click or
  `q` `r` `b` `n` chooses, Enter is a queen, Esc puts the pawn back. Tried in tmux
  (2026-11-24).
- **M2 · Clocks and game flow: built** (eighth PR). `--clock 5+3` or `:clock 5+3` plays to a
  clock with an increment, from the first move; the clocks stand beside the players, the
  running one highlighted, and a flag ends the game (a draw against a lone king). cb2026 is
  sent both clocks (`limits.clock`); an engine without the `clock` capability gets a share
  of its time. `:resign`, and `:draw`, which the engine answers with its next move from its
  score. `u` takes back a resignation or an agreed draw, and takebacks put the clocks back.
  Tried in tmux (2026-11-24): cb2026 against itself at 0.2+0 played 72 moves until White's
  flag fell; at 5+3, a draw offer after 1. e4 was accepted (cb2026 scored itself -0.17), and
  a resignation was taken back.
- **Analysis: built** (ninth PR). `i` or `:analyse`: the engine searches the position on the
  board until `i` again, with several lines (`:lines <n>`, 3 by default, if it has
  `multipv`), and the search starts again whenever the board changes: a move for either
  side, `[` `]`, or a step in the PGN viewer, where it takes the notes' place. The engine
  plays neither side meanwhile and the clocks stop. cb2026 throttles its progress with one
  interval for all lines, so with several lines cbtui asks for every report
  (`progressMs` 0): about one message per line per depth. Tried in tmux (2026-11-24).
- **Evaluation and techniques: built** (tenth PR); M3 is done. `v` or `:eval` shows the
  engine's evaluation of the position on the board, term by term (`engine/evaluate`, quiet
  when it can). `:techniques` lists the engine's techniques (`engine/techniques`, asked once
  at the start) to switch with Space; switches go with every search, and an analysis starts
  again with them. cb2026 names several techniques alike ("Evaluation", "Time"), so a shared
  name shows the description instead. Tried in tmux (2026-11-24): the terms of 1. e4 add up
  to its +0.33, and with alpha-beta off an analysis crawls to depth 4.
- **Following the root: built** (eleventh PR). `w` or `:root` asks searches for
  `engine/root` (every 250 ms, and at once on a re-search) and shows it in place of the move
  list: the depth, the root move being searched and the best so far, a bar for each root
  move's share of the effort, and the latest re-search in words. It works for the engine's
  moves and for analyses; an analysis starts again when it's switched. Tried in tmux with
  cb2026 (2026-11-24).
- **Tree browser: built** (twelfth PR); M4 is done, and with it every planned milestone.
  `:tree [depth]` sends `engine/tree` for the position on the board and opens the answer in
  place of the side panels: the search's result, the node walked to with its window and
  score, its children with their scores, depth and decisions in brief, and the highlighted
  one's decisions in full. → asks `engine/treeNode` for the next level, ← goes back, and the
  board shows the node's position. An expired tree closes and says to search again. Tried in
  tmux (2026-11-24): a depth-8 tree after 1. e4 e5 2. Nf3 Nc6, walked into 3. Bb5 Nd4.
- **Draw offers: stricter** (fourteenth PR, Carlos's choice in #14). The engine considers a
  draw only from move 30, and accepts only when it is clearly worse (-0.50 or less for
  itself) or being mated; declining early, it says so. Tried in tmux with cb2026
  (2026-11-25): declined after 1. e4, accepted at move 40 a queen down.
- Known: ratatui-image's terminal query leaves a thread reading stdin when the terminal
  doesn't answer in time, which swallows a key. cbtui doesn't ask inside tmux, where that
  happened; elsewhere it says so on the status line if it happens.
- Tests: 102 unit tests (the app against a fake engine, the CNP client against a fake node over
  pipes, the layout and mouse mapping with Ratatui's test backend), and 4 against a real cb2026
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

Engine features cb2026 offers that no milestone covers yet (mate search, Chess960, pondering,
root-move restriction, depth limits): Carlos to decide in #4.

Every milestone in `docs/PRD.md` is built. What's left is the "Later" row (engines on
another computer, a UCI backend, themes), to be decided now that M3 is done. Later for the
viewer: more than the first game
in a file, and drawing the arrows and squares PGN comments can carry (`[%cal]`, `[%csl]`).

## How to carry on

1. Read `CLAUDE.md` and `docs/PRD.md`.
2. Build an engine to test against (see `scripts/check.sh`), and run
   `CBTUI_TEST_ENGINE=… scripts/check.sh`.
3. Try it for real: `CBTUI_ENGINE=… cargo run --release`.
