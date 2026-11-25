# Releasing cbtui

A release is a version tag. Pushing it runs `.github/workflows/release.yml`, which builds a
download for each platform and attaches them to a **draft** GitHub release for Carlos to read
over and publish.

## What a download holds

`cbtui-<version>-<target>.tar.gz` (`.zip` on Windows) unpacks to a folder with:

- `cbtui`, and `cb2026` built at the chessengine commit `Cargo.toml` pins for the rules crate,
  with `--no-default-features` (no Syzygy tablebases), so the whole download is
  `MIT OR Apache-2.0`. cbtui starts the cb2026 beside it when no engine is named
  (`args::bundled_engine`).
- `README.md`, `LICENSE-MIT`, `LICENSE-APACHE`, and `THIRD-PARTY-NOTICES.txt` from
  `scripts/notices.sh`: the licences of every crate either program is built with, and the
  piece pictures' licence.

The targets: `aarch64-apple-darwin`, `x86_64-apple-darwin` (cross-compiled on Apple silicon),
`x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` (built on Ubuntu 22.04, so glibc
2.35 or later), and `x86_64-pc-windows-msvc`. The release also has `SHA256SUMS.txt`.

## Making one

1. On `main`, with CI green, set `version` in `Cargo.toml` (run `cargo build` to update
   `Cargo.lock`), and merge that in a PR.
2. Tag the merge commit and push the tag:

   ```sh
   git tag v0.2.0 && git push origin v0.2.0
   ```

3. When the Release workflow is done, sign the macOS downloads on the Mac with the Developer
   ID key: `scripts/mac-sign-release.sh v0.2.0` (below).
4. Open the draft under Releases, read the notes (from `.github/release-notes.md`, with the
   merged PRs after them), and publish it.

A pull request that changes the workflow, the release notes or the notices builds every
download without releasing, and `gh workflow run release.yml` does the same by hand.

## Signing for macOS

The workflow can't sign the macOS binaries: the Developer ID key is in a Mac's keychain, not on
GitHub. On that Mac, once the draft is there:

```sh
scripts/mac-sign-release.sh v0.2.0
```

It downloads the two macOS packages, checks them against `SHA256SUMS.txt`, signs `cbtui` and
`cb2026` in each with the hardened runtime, has Apple notarise all four, checks that macOS
finds them notarised and that they still run, and replaces the two packages and
`SHA256SUMS.txt` on the draft. Then publish the draft.

It uses the keychain's "Developer ID Application" identity (Carlos's, as an individual) and
the `cb2026-notary` notarytool profile, the same that cb2026's and Solrac's releases use; set
`SIGN_IDENTITY` or `NOTARY_PROFILE` for others. A command-line program can't have its
notarisation ticket stapled to it, so macOS looks it up online the first time the program
runs.

To try the signing without a release, `scripts/mac-sign-release.sh --trial <run-id>` signs a
Release workflow run's macOS builds and checks they run, sending nothing to Apple and
uploading nothing.
