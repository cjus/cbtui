# cbtui: instructions for Claude

cbtui (repo `cjus/cbtui`, public) is a full-screen terminal chess client, built with Ratatui,
for engines that speak the Chess Node Protocol: cb2026 now, later engines too. Product
requirements: `docs/PRD.md`. Owner: Carlos.

**Start here:** `docs/status.md` says what is built, what is pinned to what, and what is next.
Keep it up to date in every PR that changes the picture.

## Rules

1. **A client, not an engine.** cbtui never searches or evaluates. Moves and analysis come from
   the engine over CNP (`src/node.rs`); legality and notation come from `src/rules.rs`.
2. **Engine-agnostic.** Turn features on from the engine's `initialize` answer (capabilities,
   limits), never from its name or version. A feature the engine lacks is hidden or explained,
   never an error.
3. **Generic and public.** This repository is public. Show what the engine reports in general
   terms. Never add another product's coaching or move-explanation methods, and never link
   private repositories. If cbtui needs something new from an engine or from CNP, ask for a
   generic primitive, described generically.
4. **`src/rules.rs` is the only module that names `cb2026-chess`.** Everything else uses
   `crate::rules`.
5. **`App` has no terminal and no process.** It reacts to input and engine events and calls the
   `Engine` trait, so it's tested with a fake engine. Drawing and mouse placement share
   `ui::layout` and `ui::board_geometry`.
6. **Every public item has rustdoc** (`missing_docs = "deny"`), and no `unsafe`
   (`unsafe_code = "forbid"`). Module docs say what the module does, how, and its pitfalls.
7. **Permissive dependencies only** (`cargo deny check`): cbtui is `MIT OR Apache-2.0`. An
   engine is a separate program, whatever its licence.
8. Named constants for tunable numbers (timings, sizes), never magic numbers in logic.

## Definition of done for a change

1. `scripts/check.sh` passes with `CBTUI_TEST_ENGINE` set to a cb2026 built at the pinned
   chessengine commit: fmt, clippy, tests (including the real-engine ones), docs, licences.
2. CI's `CI` check is green on the PR.
3. Work is on a `claude/<topic>` branch from the latest `main`, with a PR to `main`. Claude may
   merge its own PRs once `CI` is green, the checks passed on the latest commit, the PR merges
   cleanly and no review comment is unanswered.
4. A change you can see is tried in a real terminal (a tmux pane is fine), not only in tests.
5. `docs/status.md`, and `docs/PRD.md` if scope or decisions change, are updated in the same PR.

## Items for Carlos

Anything Carlos needs to do, decide or try is a GitHub issue in this repo assigned to `cjus`
with the `needs-carlos` label: why it matters, the exact steps, the expected result, and
"Done when".
