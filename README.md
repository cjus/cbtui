# cbtui

Play chess in your terminal against a real engine.

cbtui is a full-screen terminal chess client built with [Ratatui](https://ratatui.rs). It
doesn't play chess itself. It starts an engine that speaks the
[Chess Node Protocol](https://github.com/cjus/chess-node-protocol) (CNP) and asks it for moves:
[cb2026](https://github.com/cjus/chessengine) by default, or any newer CNP engine. Everything
it shows comes from what the engine says it can do, so a newer engine works without changes
here.

```text
┌ Board ────────────────────┐┌ cb2026 1.0.0+f41bbcc85c7a ──────────────────┐
│                           ││White: you   Black: cb2026                   │
│                           ││800 ms a move                                │
│                           ││Licence: GPL-3.0-or-later                    │
│8  ♜  ♞  ♝  ♛  ♚  ♝  ♞  ♜  ││White to move                                │
│7  ♟  ♟           ♟  ♟  ♟  │└─────────────────────────────────────────────┘
│6        ♟     ♟           │┌ Moves ──────────────────────────────────────┐
│5     ♝     ♟              ││  1. e4       e6                             │
│4              ♟           ││  2. Nf3      d5                             │
│3                 ♞        ││  3. Bb5+     c6                             │
│2  ♟  ♟  ♟  ♟     ♟  ♟  ♟  ││                                             │
│1  ♜  ♞  ♝  ♛  ♚        ♜  ││                                             │
│   a  b  c  d  e  f  g  h  │└─────────────────────────────────────────────┘
│                           │┌ Engine ─────────────────────────────────────┐
│                           ││depth 18/32  -0.07  1.0M nodes  1.3M/s  0.8s │
│                           ││c6 Bd3 dxe4 Bxe4 Nf6 Nc3 Be7 O-O Nxe4 Nxe4 c5│
│                           ││d4 cxd4 Nxd4 O-O Bf4 b6 Nb5 a6 Nbd6 Nc6 c3 e5│
└───────────────────────────┘└─────────────────────────────────────────────┘
cb2026 played c6.
```

(A 76×20 terminal. In colour the squares are shaded and the pieces white and black; on a
larger terminal the board grows.)

## Install

You need Rust 1.97 or newer, and an engine.

```sh
# The engine: cb2026, which speaks CNP with `cb2026 --cnp`.
cargo install --locked --git https://github.com/cjus/chessengine cb2026

# cb2026's default build includes Syzygy tablebase probing, which makes that binary
# GPL-3.0-or-later; add --no-default-features for an MIT OR Apache-2.0 engine without it.
# cbtui shows whichever licence the engine reports. cbtui itself links no GPL code.

# cbtui itself.
cargo install --locked --git https://github.com/cjus/cbtui
```

## Play

```sh
cbtui                       # you play White against cb2026 on the PATH
cbtui --play white          # the engine plays White
cbtui --level 8             # a weaker engine, if it has levels
cbtui --engine ~/bin/cb2027 # another CNP engine (or set CBTUI_ENGINE)
cbtui --help                # every option
```

Click a piece and then its square, or use the arrow keys and Enter, or type a move (`e4`,
`Nf3`, `O-O`, `e8=N`, `e2e4`) and press Enter. Press `?` for help.

| Key | |
| --- | --- |
| `m` | the engine moves (now, if it is thinking) |
| `u` | take back your last move |
| `n` | new game |
| `t` | turn the board around |
| `:` | type a command: `:play white\|black\|both\|none`, `:level <n>\|max`, `:time <ms>`, `:fen <FEN>`, `:pgn <file>` |
| `q` | quit |

### Pieces

In a terminal that can show images (Ghostty, kitty, WezTerm, iTerm2, or any with Sixel), the
pieces are pictures that fill their squares, and the board grows to fill its panel. Everywhere
else, and inside tmux, they are two letters: `WK` is White's king, `BP` a black pawn.
`--pieces letters` or `--pieces symbols` (chess symbols, one cell each) choose for yourself.

The pictures are Colin M. L. Burnett's pieces, from Wikimedia Commons, under the BSD licence
(`assets/pieces/`).

Colours are 24-bit when your terminal sets `COLORTERM=truecolor` or shows pictures, otherwise
the nearest of the 256 standard colours.

## How it works

cbtui runs the engine as a child process and talks to it in CNP: JSON-RPC 2.0, one message
per line on the engine's stdin and stdout. Chess rules (legal moves, SAN, PGN, game endings)
come from `cb2026-chess`, cb2026's rules library, which cbtui links. Only `src/rules.rs` uses
it, so another rules library could take its place. See `docs/PRD.md` for the design.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option.
