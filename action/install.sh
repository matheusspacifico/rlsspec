#!/usr/bin/env bash
# Installs the rlsspec release named by INPUT_VERSION (or the action's own vX.Y.Z ref) for this runner,
# checked against the release's sha256, and puts it on PATH. RLSSPEC_ACTION_BINARY (a local build) skips
# the download: the action's own CI uses it to test a checkout before any release exists.
set -euo pipefail

fail() {
  echo "::error title=rlsspec::$1"
  exit 1
}

if [[ -n "${RLSSPEC_ACTION_BINARY:-}" ]]; then
  [[ -x "$RLSSPEC_ACTION_BINARY" ]] || fail "RLSSPEC_ACTION_BINARY is not an executable: $RLSSPEC_ACTION_BINARY"
  dir="$(cd "$(dirname "$RLSSPEC_ACTION_BINARY")" && pwd)"
  echo "$dir" >> "$GITHUB_PATH"
  echo "Using the local build $RLSSPEC_ACTION_BINARY"
  exit 0
fi

release_tag='^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'
version="${INPUT_VERSION:-}"
if [[ -z "$version" ]]; then
  [[ "${ACTION_REF:-}" =~ $release_tag ]] ||
    fail "set the 'version' input (e.g. version: 0.1.0): the action is used at '${ACTION_REF:-a local path}', not at a release tag like v0.1.0"
  version="$ACTION_REF"
fi
[[ "$version" == v* ]] || version="v$version"
[[ "$version" =~ $release_tag ]] || fail "'version' must be a release like 0.1.0, got '${INPUT_VERSION}'"

case "${RUNNER_OS}/${RUNNER_ARCH}" in
  Linux/X64) target=x86_64-unknown-linux-musl ;;
  Linux/ARM64) target=aarch64-unknown-linux-musl ;;
  macOS/X64) target=x86_64-apple-darwin ;;
  macOS/ARM64) target=aarch64-apple-darwin ;;
  Windows/*) fail "Windows runners are not supported yet: run rlsspec on ubuntu-latest or macos-latest" ;;
  *) fail "no rlsspec build for ${RUNNER_OS}/${RUNNER_ARCH}" ;;
esac

repository="${ACTION_REPOSITORY:-matheusspacifico/rlsspec}"
base="https://github.com/${repository}/releases/download"
archive="rlsspec-${target}.tar.xz"
dir="${RUNNER_TEMP}/rlsspec-${version}-${target}"
mkdir -p "$dir"
cd "$dir"

download() {
  curl --proto '=https' --tlsv1.2 -fsSL --retry 3 -o "$2" "$1" ||
    fail "cannot download $1 (does release ${version} exist?)"
}
download "${base}/${version}/${archive}" "$archive"
download "${base}/${version}/${archive}.sha256" "${archive}.sha256"

expected="$(awk '{print $1}' "${archive}.sha256")"
if command -v sha256sum > /dev/null; then
  actual="$(sha256sum "$archive" | awk '{print $1}')"
else
  actual="$(shasum -a 256 "$archive" | awk '{print $1}')"
fi
[[ -n "$expected" && "$actual" == "$expected" ]] ||
  fail "checksum mismatch for ${archive}: expected ${expected:-nothing}, got ${actual}"

tar xf "$archive" --strip-components=1 "rlsspec-${target}/rlsspec"
echo "$dir" >> "$GITHUB_PATH"
echo "Installed $("$dir/rlsspec" version) (${target}, sha256 ${actual})"
