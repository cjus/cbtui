#!/usr/bin/env bash
# Writes the third-party notices that go with each release download of cbtui, into
# target/licences/THIRD-PARTY-NOTICES.txt.
#
#   scripts/notices.sh <chessengine>
#
# <chessengine> is a checkout of the chessengine repository at the commit Cargo.toml pins: the
# downloads bundle the cb2026 built from it, without Syzygy tablebases, so its crates' notices
# go in too. The licences of the crates both programs are built with ask that their notices go
# with every copy: the MIT and BSD licences' copyright lines, Apache-2.0's NOTICE files. This
# gathers them with cargo-about (scripts/notices/about.toml and the template
# scripts/notices/licences.hbs), which reads each crate's own licence files, and adds any
# crate's NOTICE file, found through scripts/notices/crates.hbs, which lists the crates'
# folders. It adds the piece pictures' licence (assets/pieces/LICENSE.txt) too. The release
# workflow runs it once, on Linux, and puts the file in every download.
#
# Needs cargo-about 0.9 (cargo install --locked cargo-about@0.9.2 --features cli) and the
# network: it fetches, from a crate's repository, the licence files the crate didn't package.
set -euo pipefail

if [[ $# -ne 1 || ! -f "$1/crates/cli/Cargo.toml" ]]; then
  sed -n '2,19p' "$0"
  exit 2
fi
ENGINE=$(cd "$1" && pwd)
cd "$(dirname "$0")/.."
ROOT=$(pwd)
if ! cargo about --version >/dev/null 2>&1; then
  echo "cargo-about isn't installed: cargo install --locked cargo-about@0.9.2 --features cli" >&2
  exit 1
fi

OUT="${CARGO_TARGET_DIR:-target}/licences"
NOTICES="$OUT/THIRD-PARTY-NOTICES.txt"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$OUT"
CONFIG="$ROOT/scripts/notices/about.toml"
RULE="--------------------------------------------------------------------------------"

# The licences, then the NOTICE files, of one program's crates. Arguments: what to tell
# cargo-about about the program (its manifest, its features).
notices_of() {
  cargo about generate --locked -c "$CONFIG" "$@" -o "$WORK/licences" \
    "$ROOT/scripts/notices/licences.hbs"
  cargo about generate --locked -c "$CONFIG" "$@" -o "$WORK/folders" \
    "$ROOT/scripts/notices/crates.hbs"
  cat "$WORK/licences"
  shopt -s nullglob nocaseglob
  while read -r manifest; do
    [[ -n "$manifest" ]] || continue
    folder=$(dirname "$manifest")
    for notice in "$folder"/NOTICE*; do
      printf '\n%s\nThe NOTICE file of %s\n\n' "$RULE" "$(basename "$folder")"
      cat "$notice"
    done
  done < "$WORK/folders"
  shopt -u nullglob nocaseglob
}

{
  cat <<'HEAD'
Third-party notices
===================

This download holds two programs, each under the MIT licence or the Apache License 2.0, at
your choice (LICENSE-MIT, LICENSE-APACHE), both Copyright (c) 2026 Carlos Justiniano:

- cbtui, the terminal chess client: https://github.com/cjus/cbtui
- cb2026, the chess engine it plays against, built without Syzygy tablebase probing, so that
  it links no GPL code: https://github.com/cjus/chessengine

Each is built with the open-source crates (Rust libraries) below, each under its own licence.
Each licence's text follows, with the crates that use it. Where a crate offers a choice of
licences, it is taken under the one named.


cbtui
=====

HEAD
  notices_of --manifest-path "$ROOT/Cargo.toml"
  printf '\n%s\nThe piece pictures (assets/pieces), by Colin M. L. Burnett\n\n' "$RULE"
  cat "$ROOT/assets/pieces/LICENSE.txt"
  printf '\n\ncb2026\n======\n\n'
  notices_of --manifest-path "$ENGINE/crates/cli/Cargo.toml" --no-default-features
} > "$NOTICES"
echo "Wrote $NOTICES"
