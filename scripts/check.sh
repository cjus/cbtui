#!/usr/bin/env bash
# Everything CI checks, run locally: format, lints, tests, docs, licences and advisories.
# The steps are the same as .github/workflows/ci.yml's: change both together.
#
# The real-engine tests (tests/engine.rs) run when CBTUI_TEST_ENGINE names a CNP engine, for
# example the cb2026 that Cargo.toml's chessengine commit builds:
#
#   cargo install --locked --git https://github.com/cjus/chessengine --rev <rev> cb2026 --root /tmp/cb2026
#   CBTUI_TEST_ENGINE=/tmp/cb2026/bin/cb2026 scripts/check.sh
set -euo pipefail
cd "$(dirname "$0")/.."

step() { printf '\n== %s\n' "$*"; }

step Format
cargo fmt --all --check
step Clippy
cargo clippy --all-targets --locked -- -D warnings
step Tests
if [[ -z "${CBTUI_TEST_ENGINE:-}" ]]; then
    echo "(CBTUI_TEST_ENGINE isn't set: the real-engine tests will skip)"
fi
cargo test --locked
step Docs
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items --locked
step "Licences and advisories"
cargo deny check
printf '\nAll checks passed.\n'
