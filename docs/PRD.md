# cbtui: product requirements

Owner: Carlos. Started 2026-11-24.

## What it is

cbtui is a full-screen terminal chess client. A person plays and studies chess in a terminal
against an engine that speaks the Chess Node Protocol (CNP), cb2026 to begin with. It is a
client: the engine is a separate program that cbtui starts and talks to, and cbtui never
searches or evaluates a position itself.

## Why a separate program

cb2026 is an engine used under chess GUIs, and its own product requirements rule out a
full-screen terminal UI (chessengine #9). A separate client leaves the engine as it is (its
protocols, binary and tests don't change), and outlives one engine generation. The name is
`cbtui`, not `cb2026-tui`, because it should drive later engines (a cb2027) as well.

## Goals

- **Fun to play in a terminal**: a clear board, mouse and keyboard moves, typed SAN, the
  engine's thinking live, a move list, PGN out.
- **Engine-agnostic**: any CNP engine works. The engine is configuration (`--engine`,
  `CBTUI_ENGINE`), and every feature is switched on from the engine's `initialize` answer
  (capabilities and limits), never from its name or version.
- **Works everywhere a terminal does**: macOS, Linux, Windows terminals, and over SSH;
  256-colour fallback, a clear message when the terminal is too small.
- **Pieces you can see**: pictures that fill their squares on terminals that can show images
  (kitty's protocol, iTerm2's or Sixel), two letters (`WK`, `BP`) everywhere else.
- **Generic and public**: the repository is public (`MIT OR Apache-2.0`). It shows what the
  engine reports in general terms. It does not carry any other product's coaching or
  explanation methods.

## Non-goals

- Playing strength of its own: no search, no evaluation, no opening book in cbtui.
- Other protocols for now. UCI could come later as a second backend, for engines that don't
  speak CNP; CNP stays the main one because it carries more (levels, evaluation breakdowns,
  search trees).
- A graphical (non-terminal) interface.

## Architecture

One crate, a library and a binary:

| Module | What it does |
| --- | --- |
| `node` | Starts the engine, the CNP handshake, a reader thread that turns messages into events, requests by id, polite shutdown |
| `rules` | Legal moves, SAN, FEN, PGN, game endings: a thin layer over `cb2026-chess`; the only module that names it |
| `app` | The session's state and what keys, clicks and engine messages do to it. No terminal, no process: tested with a fake engine |
| `ui` | Drawing with Ratatui; the same layout code places mouse clicks |
| `pieces` | Pieces as pictures (ratatui-image finds the terminal's image protocol, resvg draws the SVGs at a square's size) or as two letters |
| `args` | The command line |
| `main.rs` | The terminal, and one event loop over one channel (keys, mouse, engine) |

Dependencies are pinned by commit where they are git dependencies: `cnp` (the protocol's
types) and `cb2026-chess` (rules). The engine CI tests against is built from the same
chessengine commit as the rules crate.

Licences: every linked crate is permissive (`cargo deny check`). The engine is a separate
program; cbtui shows the licence it reports, with its whole version, on the help screen.

## Milestones

| Milestone | Scope | Done when |
| --- | --- | --- |
| **M1 · Playable** | CNP connection; board with mouse, keyboard and typed moves; engine plays either side, both or none; levels (if the engine has them) and time per move; live thinking; move list; undo, new game, FEN, flip; PGN save; help | A full game against cb2026 in a terminal, with no illegal moves and a clean exit |
| M2 · Clocks and game flow | Chess clocks with increment (CNP `clock` limits), resign and draw offers, PGN load (done: the PGN viewer), promotion picker, move-list navigation (done: `[` `]`) | A timed game against cb2026 to the end |
| M3 · Analysis | Infinite analysis with MultiPV (capabilities `multipv`), the evaluation broken into terms (`engine/evaluate`), technique switches (`engine/techniques`), each shown only when the engine has it | Analysing a position shows several lines and the evaluation's terms with cb2026 |
| M4 · Look inside | Following the search at its root (`engine/root`) and browsing its search tree (`engine/tree`, `engine/treeNode`), generic views of what the engine reports | The tree browser opens a cb2026 search and walks it |
| Later | Engines on another computer through cb2026's network client; a UCI backend; themes | Decided when M3 is done |

## Decisions

- [x] Separate public repository `cjus/cbtui`, a CNP client of cb2026 (Carlos, 2026-11-24).
- [x] Named `cbtui` so that it can use later engines, such as a cb2027 (Carlos, 2026-11-24).
- [x] Generic: nothing that shares another product's way of explaining moves (Carlos,
      2026-11-24).
- [x] CNP first, `cb2026-chess` for rules, Ratatui with its crossterm backend (2026-11-24).
- [x] Pieces: pictures where the terminal can show them, Colin M. L. Burnett's set
      (BSD, from Wikimedia Commons); two letters, `WK`, elsewhere (Carlos, 2026-11-24).
- [x] A PGN viewer: `--pgn <file>` shows a game read-only, arrows step through it (in games,
      `[` `]`, so the arrows stay the board's cursor), digits enter variations, `:play` plays
      on from the position shown, and no engine is needed (Carlos, 2026-11-24).
- [x] A zoom key (`z`) that hides the side panels so the board fills the screen; text squares
      grow past 7×3 when there is room (Carlos, 2026-11-24).
