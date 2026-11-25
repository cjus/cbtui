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

3. When the Release workflow is done, open the draft under Releases, read the notes (from
   `.github/release-notes.md`, with the merged PRs after them), and publish it.

A pull request that changes the workflow, the release notes or the notices builds every
download without releasing, and `gh workflow run release.yml` does the same by hand (signing
too, if it can).

## Signing for macOS

macOS keeps an unsigned download from running until its quarantine flag is cleared. With
these repository secrets (Settings → Secrets and variables → Actions), the workflow signs both
binaries with a Developer ID and notarises them; without them it builds unsigned and warns:

| Secret | What |
| --- | --- |
| `APPLE_CERTIFICATE` | The "Developer ID Application" certificate and its private key, exported from Keychain Access as a `.p12`, base64-encoded (`base64 -i cert.p12 \| pbcopy`) |
| `APPLE_CERTIFICATE_PASSWORD` | The password the `.p12` was exported with |
| `APPLE_SIGNING_IDENTITY` | The certificate's name, like `Developer ID Application: Name (TEAMID)` (`security find-identity -v -p codesigning`) |
| `APPLE_ID` | The Apple ID of the developer account |
| `APPLE_TEAM_ID` | Its ten-character team ID (developer.apple.com → Membership) |
| `APPLE_APP_PASSWORD` | An app-specific password for that Apple ID (account.apple.com → Sign-In and Security) |

A command-line program can't have its notarisation ticket stapled to it, so macOS looks it up
online the first time the program runs.
