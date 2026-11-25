#!/usr/bin/env bash
# Signs and notarises a draft release's macOS downloads, and puts them back on the draft.
#
#   scripts/mac-sign-release.sh <tag>            for example: scripts/mac-sign-release.sh v0.1.0
#   scripts/mac-sign-release.sh --trial <run>    sign a Release workflow run's macOS builds only
#
# The Release workflow builds the downloads for Apple Silicon and Intel Macs, but it can't sign
# them: the Developer ID key is in a Mac's keychain, not on GitHub. Run this on that Mac once
# the workflow has made the draft. It downloads the two macOS packages and checks them against
# SHA256SUMS.txt; signs both binaries in each (cbtui and the cb2026 beside it) with the hardened
# runtime and a secure timestamp; sends all four to Apple's notary service in one zip and waits
# for it (a few minutes); checks that macOS finds them notarised and that they still run; then
# repacks them, rewrites their two lines of SHA256SUMS.txt and replaces the three files on the
# draft. A notarisation can't be stapled to a bare binary, so macOS asks Apple online the first
# time a downloaded binary runs. --trial takes a Release workflow run's id instead of a tag and
# stops after signing and checking: nothing goes to Apple and nothing is uploaded.
#
# Needs the GitHub CLI (`gh`, logged in), Xcode's command-line tools, a "Developer ID
# Application" identity in the keychain (set SIGN_IDENTITY to its name or SHA-1 when there are
# several) and notarytool credentials saved as NOTARY_PROFILE (default cb2026-notary, the
# profile cb2026's and Solrac's releases use). It refuses a published release unless
# ALLOW_PUBLISHED=1, since replacing its files changes what people have downloaded, and does
# nothing when the binaries are already notarised. REPO defaults to cjus/cbtui. Adapted from
# cjus/chessengine's scripts/mac-sign-release.sh. See docs/releasing.md.
set -euo pipefail

usage() {
  sed -n '2,25p' "$0"
  exit 2
}
TRIAL=0
case "${1:-}" in
  "" | -h | --help) usage ;;
  --trial)
    [[ $# -eq 2 ]] || usage
    TRIAL=1
    RUN="$2"
    ;;
  *)
    [[ $# -eq 1 ]] || usage
    TAG="$1"
    ;;
esac
REPO="${REPO:-cjus/cbtui}"
NOTARY_PROFILE="${NOTARY_PROFILE:-cb2026-notary}"
# Each Mac target the workflow builds, and the architecture its binaries must have.
TARGETS="aarch64-apple-darwin:arm64 x86_64-apple-darwin:x86_64"
# The binaries in each package.
BINARIES="cbtui cb2026"

fail() {
  echo "mac-sign-release: $*" >&2
  exit 1
}

for tool in gh codesign security xcrun ditto lipo shasum plutil; do
  command -v "$tool" >/dev/null || { echo "mac-sign-release: '$tool' is not installed" >&2; exit 2; }
done

# The identity: SIGN_IDENTITY, or the keychain's only Developer ID Application one, by its SHA-1
# so that a renewed certificate beside an old one of the same name can't make it ambiguous.
IDENTITIES="$(security find-identity -v -p codesigning)"
if [[ -z "${SIGN_IDENTITY:-}" ]]; then
  FOUND="$(printf '%s\n' "$IDENTITIES" | grep '"Developer ID Application: ' || true)"
  COUNT="$(printf '%s' "$FOUND" | grep -c . || true)"
  [[ "$COUNT" -ge 1 ]] || fail "no Developer ID Application identity in the keychain (docs/releasing.md)"
  [[ "$COUNT" -eq 1 ]] ||
    fail "$COUNT Developer ID Application identities in the keychain; set SIGN_IDENTITY to one's SHA-1:
$FOUND"
  SIGN_IDENTITY="$(printf '%s' "$FOUND" | awk '{print $2}')"
fi
SIGNER="$(printf '%s\n' "$IDENTITIES" | { grep -F -- "$SIGN_IDENTITY" || true; } | sed -n 's/^[^"]*"\(.*\)"$/\1/p' | sed -n 1p)"
[[ -n "$SIGNER" ]] || fail "no valid signing identity matches SIGN_IDENTITY=$SIGN_IDENTITY"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/cbtui-sign.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

# The packages, unpacked into $WORK/unpacked/<name>/, and the names, into NAMES.
NAMES=()
if [[ "$TRIAL" == 1 ]]; then
  echo "Downloading the macOS builds of run $RUN ($REPO)..."
  for spec in $TARGETS; do
    gh run download "$RUN" -R "$REPO" -n "${spec%%:*}" -D "$WORK/downloaded"
  done
  cd "$WORK"
  mkdir unpacked
  for spec in $TARGETS; do
    package="$(ls downloaded/cbtui-*-"${spec%%:*}".tar.gz 2>/dev/null | sed -n 1p)"
    [[ -n "$package" ]] || fail "run $RUN has no ${spec%%:*} package"
    tar -xzf "$package" -C unpacked
    NAMES+=("$(basename "$package" .tar.gz)")
  done
else
  DRAFT="$(gh release view "$TAG" -R "$REPO" --json isDraft --jq .isDraft 2>/dev/null)" ||
    fail "no release $TAG in $REPO: push the tag and wait for the Release workflow's draft"
  if [[ "$DRAFT" != "true" && "${ALLOW_PUBLISHED:-0}" != "1" ]]; then
    fail "$TAG is published, and replacing its files changes what people have downloaded; set ALLOW_PUBLISHED=1 to do it anyway"
  fi
  cd "$WORK"
  mkdir downloaded unpacked
  PATTERNS=(-p SHA256SUMS.txt)
  for spec in $TARGETS; do
    PATTERNS+=(-p "cbtui-$TAG-${spec%%:*}.tar.gz")
  done
  echo "Downloading the macOS packages and SHA256SUMS.txt from $TAG ($REPO)..."
  gh release download "$TAG" -R "$REPO" -D downloaded "${PATTERNS[@]}"
  # Each package is checked against the line the workflow wrote for it, then unpacked.
  for spec in $TARGETS; do
    name="cbtui-$TAG-${spec%%:*}"
    [[ -f "downloaded/$name.tar.gz" ]] || fail "$TAG has no $name.tar.gz"
    line="$(awk -v file="$name.tar.gz" '$2 == file' downloaded/SHA256SUMS.txt)"
    [[ -n "$line" ]] || fail "SHA256SUMS.txt has no line for $name.tar.gz"
    (cd downloaded && printf '%s\n' "$line" | shasum -a 256 -c - >/dev/null) ||
      fail "$name.tar.gz doesn't match its line in SHA256SUMS.txt"
    tar -xzf "downloaded/$name.tar.gz" -C unpacked
    NAMES+=("$name")
  done
fi

# Every binary is there, for the right architecture.
i=0
for spec in $TARGETS; do
  name="${NAMES[$i]}"
  arch="${spec#*:}"
  for binary in $BINARIES; do
    bin="unpacked/$name/$binary"
    [[ -f "$bin" ]] || fail "$name has no $binary"
    [[ "$(lipo -archs "$bin")" == "$arch" ]] || fail "$name/$binary is $(lipo -archs "$bin"), not $arch"
  done
  i=$((i + 1))
done

# codesign's "notarized" requirement looks only at the tickets this Mac already has; asking
# Gatekeeper first (spctl) fetches the binary's from Apple. spctl itself always rejects a bare
# binary ("does not seem to be an app"), so only codesign's answer counts.
notarised() {
  spctl --assess --type execute "$1" >/dev/null 2>&1 || true
  codesign --verify --strict --test-requirement="=notarized" "$1" >/dev/null 2>&1
}

if [[ "$TRIAL" == 0 ]]; then
  ALREADY=1
  for name in "${NAMES[@]}"; do
    for binary in $BINARIES; do
      notarised "unpacked/$name/$binary" || ALREADY=0
    done
  done
  if [[ "$ALREADY" == 1 ]]; then
    echo "Every macOS binary in $TAG is already signed and notarised: nothing to do."
    exit 0
  fi
fi

echo "Signing with $SIGNER..."
mkdir notarize
for name in "${NAMES[@]}"; do
  mkdir -p "notarize/$name"
  for binary in $BINARIES; do
    bin="unpacked/$name/$binary"
    codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$bin"
    codesign --verify --strict "$bin"
    info="$(codesign -dvv "$bin" 2>&1)"
    grep -qxF "Authority=$SIGNER" <<< "$info" || fail "$bin isn't signed by $SIGNER"
    grep -q "flags=.*runtime" <<< "$info" || fail "$bin isn't signed with the hardened runtime"
    cp "$bin" "notarize/$name/$binary"
  done
done

# Signing must not change what the binaries do: each says its version, the Intel ones through
# Rosetta when it is installed.
check_runs() {
  local i=0 name arch binary bin version
  for spec in $TARGETS; do
    name="${NAMES[$i]}"
    arch="${spec#*:}"
    i=$((i + 1))
    if ! arch "-$arch" /usr/bin/true 2>/dev/null; then
      echo "${spec%%:*}: not run (this Mac can't run $arch binaries; install Rosetta to check them)."
      continue
    fi
    for binary in $BINARIES; do
      bin="unpacked/$name/$binary"
      version="$(arch "-$arch" "$bin" --version 2>/dev/null || true)"
      [[ "$version" == "$binary "* ]] || fail "${spec%%:*}'s signed $binary doesn't run"
      echo "${spec%%:*}: $version"
    done
  done
}

if [[ "$TRIAL" == 1 ]]; then
  check_runs
  echo
  echo "Trial done: run $RUN's macOS binaries sign as $SIGNER and still run."
  echo "Nothing was sent to Apple or uploaded."
  exit 0
fi

echo "Sending the four binaries to Apple's notary service and waiting for it..."
ditto -c -k --keepParent notarize cbtui-notarize.zip
xcrun notarytool submit cbtui-notarize.zip --keychain-profile "$NOTARY_PROFILE" --wait \
  --timeout 1h --output-format json > notary.json || true
STATUS="$(plutil -extract status raw -o - notary.json 2>/dev/null || echo unknown)"
SUBMISSION="$(plutil -extract id raw -o - notary.json 2>/dev/null || echo unknown)"
if [[ "$STATUS" != "Accepted" ]]; then
  cat notary.json >&2 || true
  if [[ "$SUBMISSION" != "unknown" ]]; then
    xcrun notarytool log "$SUBMISSION" --keychain-profile "$NOTARY_PROFILE" >&2 || true
  fi
  fail "the notary service answered '$STATUS' for submission $SUBMISSION, so nothing was uploaded"
fi
echo "Notarised: submission $SUBMISSION, $STATUS."

# macOS asks Apple for the notarisation; it is usually there at once, but give it a minute.
for name in "${NAMES[@]}"; do
  for binary in $BINARIES; do
    for attempt in 1 2 3 4 5 6; do
      notarised "unpacked/$name/$binary" && break
      [[ "$attempt" -lt 6 ]] || fail "macOS doesn't find $name/$binary notarised"
      sleep 10
    done
  done
done
check_runs

# Repacked as the workflow packs them, without macOS's extended attributes. Only their lines
# of SHA256SUMS.txt change.
mkdir signed uploaded
cp downloaded/SHA256SUMS.txt signed/SHA256SUMS.txt
UPLOADS=()
for name in "${NAMES[@]}"; do
  (cd unpacked && COPYFILE_DISABLE=1 tar --no-mac-metadata --no-xattrs -czf "../signed/$name.tar.gz" "$name")
  [[ "$(tar -tzf "downloaded/$name.tar.gz" | sort)" == "$(tar -tzf "signed/$name.tar.gz" | sort)" ]] ||
    fail "the repacked $name.tar.gz doesn't hold the same files as the original"
  sum="$(shasum -a 256 "signed/$name.tar.gz" | awk '{print $1}')"
  awk -v file="$name.tar.gz" -v sum="$sum" '$2 == file {print sum "  " file; next} {print}' \
    signed/SHA256SUMS.txt > signed/SHA256SUMS.next
  mv signed/SHA256SUMS.next signed/SHA256SUMS.txt
  (cd signed && awk -v file="$name.tar.gz" '$2 == file' SHA256SUMS.txt | shasum -a 256 -c - >/dev/null) ||
    fail "the new SHA256SUMS.txt doesn't match the repacked $name.tar.gz"
  UPLOADS+=("signed/$name.tar.gz")
done
UPLOADS+=(signed/SHA256SUMS.txt)

echo "Replacing the two packages and SHA256SUMS.txt on $TAG..."
gh release upload "$TAG" -R "$REPO" --clobber "${UPLOADS[@]}"
gh release download "$TAG" -R "$REPO" -D uploaded "${PATTERNS[@]}"
for file in "${UPLOADS[@]}"; do
  cmp -s "$file" "uploaded/$(basename "$file")" || fail "$(basename "$file") on $TAG isn't the file uploaded"
done

echo
echo "Done: $TAG's macOS binaries are signed by $SIGNER and notarised (submission $SUBMISSION)."
echo "SHA256SUMS.txt on $TAG now reads:"
cat signed/SHA256SUMS.txt
