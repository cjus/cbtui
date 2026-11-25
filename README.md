# cbtui

Play chess in your terminal against a real engine.

cbtui is a full-screen terminal chess client built with [Ratatui](https://ratatui.rs). It
doesn't play chess itself. It starts an engine that speaks the
[Chess Node Protocol](https://github.com/cjus/chess-node-protocol) (CNP) and asks it for moves:
[cb2026](https://github.com/cjus/chessengine) by default, or any newer CNP engine. Everything
it shows comes from what the engine says it can do, so a newer engine works without changes
here.

![cbtui in Ghostty: a game against cb2026, with picture pieces filling the squares and the move list beside the board](docs/screenshot.png)

## Install

You need Rust 1.97 or newer, and an engine.

```sh
# The engine: cb2026, which speaks CNP with `cb2026 --cnp`.
cargo install --locked --git https://github.com/cjus/chessengine cb2026

# cb2026's default build includes Syzygy tablebase probing, which makes that binary
# GPL-3.0-or-later; add --no-default-features for an MIT OR Apache-2.0 engine without it.
# cbtui shows whichever licence the engine reports (press ? in cbtui). cbtui itself
# links no GPL code.

# cbtui itself.
cargo install --locked --git https://github.com/cjus/cbtui
```

## Play

```sh
cbtui                       # you play White against cb2026 on the PATH
cbtui --play white          # the engine plays White
cbtui --level 8             # a weaker engine, if it has levels
cbtui --clock 5+3           # five minutes each, three seconds added a move
cbtui --theme blue          # the board's colours: brown (default), blue, green or grey
cbtui --engine ~/bin/cb2027 # another CNP engine (or set CBTUI_ENGINE)
cbtui --help                # every option
```

Click a piece and then its square, or use the arrow keys and Enter, or type a move (`e4`,
`Nf3`, `O-O`, `e8=N`, `e2e4`) and press Enter. A pawn that reaches the last rank offers a
queen, rook, bishop and knight on the squares beside it: click one, or press `q`, `r`, `b` or
`n` (Enter is a queen, Esc puts the pawn back). Press `?` for help; when it doesn't fit the
terminal, ↑ ↓, Page Up, Page Down or the mouse wheel scroll it.

| Key | |
| --- | --- |
| `m` | the engine moves (now, if it is thinking) |
| `u` | take back your last move |
| `n` | new game |
| `[` `]` | look back through the game a move at a time (`Home`, `End`); moves wait until you're back at the end |
| `t` | turn the board around |
| `z` | zoom: the board fills the screen, without the side panels; again to bring them back |
| `i` | analyse: the engine studies the position on the board, several lines at once, until `i` again |
| `v` | the engine's evaluation of the position on the board, term by term |
| `w` | follow the search at its root: the move it's on, each root move's share of the effort, what it goes back over |
| `:` | type a command: `:play white\|black\|both\|none`, `:level <n>\|max`, `:time <ms>`, `:lines <n>`, `:techniques`, `:tree [depth]`, `:clock <min>+<sec>\|off`, `:theme [name]`, `:resign`, `:draw`, `:fen <FEN>`, `:pgn <file>` |
| `q` | quit |

With a clock, the clocks start with the first move and stand beside the players' names. An
engine that can read a clock (the CNP `clock` capability, as cb2026 can) is sent both and
decides its own time; one that can't gets a share of its time left each move. `:draw` offers
the engine a draw, which it answers with its next move: from move 30 on, it accepts when its
own score says it is clearly worse (by half a pawn or more), or that it is being mated. `u` takes back a resignation or an agreed draw.

Analysing (`i`), the engine plays neither side and the clocks stop: move for both sides,
or look back with `[` `]`, and the engine panel follows the position on the board with its
best lines (`:lines <n>` for how many, if the engine can find several). In the PGN viewer it
takes the place of the notes and follows you through the game. `i` again puts back who
played what.

`v` asks the engine what its evaluation of the position on the board is made of (material,
mobility, king safety, whatever its terms are), and `:techniques` lists the search
techniques it can switch, to turn them off or on for every search after: try analysing with
null-move pruning off. `w` follows the engine's searches at their root, in place of the
move list: which root move it is searching, a bar for each move's share of its effort, and
the latest thing it went back over (a move that looked better, a score outside its window).
`:tree [depth]` has the engine search the position on the board and keep the search's tree
near the root, then shows it in place of the side panels: the moves the search looked at,
each with its score and what the search decided about it (refuted by a reply, left out,
searched shallower or again), and the board shows the position you're at. The arrows walk
it: ↑ ↓ choose a move, → goes into it, ← back up, Esc closes. Each of these appears only
with an engine that has it.

## View a PGN game

```sh
cbtui --pgn game.pgn        # the first game in the file; no engine needed
```

The board is read-only and the panels show the game's players, event and result, its moves
with the variations in brackets, and the comments on the move shown.

| Key | |
| --- | --- |
| `←` `→` | a move back or on, along the line you're in (also `[` `]`) |
| `↑` `↓` | the start of the game, the end of the line (also `Home`, `End`) |
| `1`–`9` | into a variation that starts at this move (the bottom panel lists them) |
| `:play white\|black\|both\|none` | play on from the position shown: the engine plays the side(s) named, `none` for you to play both |

## Terminal compatibility

cbtui runs in any terminal at least 61×13 characters with a font that has box-drawing
characters, on macOS and Linux (Windows should work, through crossterm, but is untested). What
changes from terminal to terminal is how the pieces look.

### Pieces as pictures

At start, cbtui asks the terminal whether it can show images and how big its font is in pixels
(through [ratatui-image](https://github.com/ratatui/ratatui-image)). If it can, the pieces are
pictures that fill their squares, the squares are square, and the board grows to fill its
panel. The pictures are Colin M. L. Burnett's pieces, from Wikimedia Commons, under the BSD
licence (`assets/pieces/`).

| Terminal | Image protocol | Pieces |
| --- | --- | --- |
| Ghostty | kitty | pictures |
| kitty (0.28 or later) | kitty | pictures |
| iTerm2 | iTerm2 | pictures |
| WezTerm | iTerm2 | pictures |
| Rio | iTerm2 | pictures |
| foot | Sixel | pictures |
| xterm, started with `-ti 340` | Sixel | pictures |
| mlterm, Black Box (Flatpak), Bobcat | Sixel / iTerm2 | pictures |
| macOS Terminal | none | letters |
| Alacritty, Konsole, Warp, Contour | none that works | letters |
| anything inside tmux | (not asked) | letters |

The list follows ratatui-image's
[compatibility matrix](https://github.com/ratatui/ratatui-image#compatibility-matrix). A
terminal that isn't listed gets pictures if it answers with a protocol, and letters if it
doesn't.

- **tmux:** image protocols only get through tmux with passthrough set up, and asking tmux
  about them can lose your next key press, so cbtui doesn't ask: inside tmux the pieces are
  always letters.
- **SSH:** images are escape sequences, so they normally get through SSH; what counts is the
  terminal you are sitting at.
- **Zooming:** the font size is asked once, at start. If you zoom the font afterwards (⌘+ or
  Ctrl+), restart cbtui to redraw the pictures at the new size.

### Pieces as letters

Everywhere else the pieces are two letters, side then piece: `WK` is White's king, `BP` a black
pawn. They are white or black too, so they read with or without colour. Here is a 76×20
terminal:

```text
┌ Board ────────────────────┐┌ cb2026 1.0.0+f41bbcc ───────────────────────┐
│                           ││White: you   Black: cb2026                   │
│                           ││800 ms a move                                │
│                           ││White to move                                │
│8 BR BN BB BQ BK BB BN BR  ││                                             │
│7 BP BP          BP BP BP  │└─────────────────────────────────────────────┘
│6       BP    BP           │┌ Moves ──────────────────────────────────────┐
│5    WB    BP              ││  1. e4       e6                             │
│4             WP           ││  2. Nf3      d5                             │
│3                WN        ││  3. Bb5+     c6                             │
│2 WP WP WP WP    WP WP WP  │└─────────────────────────────────────────────┘
│1 WR WN WB WQ WK       WR  │┌ Engine ─────────────────────────────────────┐
│   a  b  c  d  e  f  g  h  ││depth 19/30  -0.18  1.1M nodes  1.4M/s  0.8s │
│                           ││c6 Bd3 dxe4 Bxe4 Nf6 Nc3 c5 O-O Nxe4 Nxe4 Nc6│
│                           ││b3 Be7 Bb2 O-O d3 b6 Qd2 Nd4 Nxd4 cxd4       │
│                           ││                                             │
│                           ││                                             │
└───────────────────────────┘└─────────────────────────────────────────────┘
cb2026 played c6.
```

`--pieces letters` asks for letters even where pictures would work, and `--pieces symbols` for
chess symbols (`♚`), one character each. `--ascii` is the same as `--pieces letters`.

### Colour

24-bit when the terminal sets `COLORTERM=truecolor` (or `24bit`), and whenever the pieces are
pictures, since every terminal that shows images has 24-bit colour; otherwise the nearest of
the 256 standard colours, which every modern terminal has, macOS Terminal included.

## How it works

cbtui runs the engine as a child process and talks to it in CNP: JSON-RPC 2.0, one message
per line on the engine's stdin and stdout. Chess rules (legal moves, SAN, PGN, game endings)
come from `cb2026-chess`, cb2026's rules library, which cbtui links. Only `src/rules.rs` uses
it, so another rules library could take its place. See `docs/PRD.md` for the design.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option.
