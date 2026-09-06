#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ -n "$(git status --porcelain --untracked-files=normal)" ]]; then
  echo "Commit the release sources before packaging." >&2
  exit 1
fi
version=$(bun scripts/check-version.js)
if [[ $(uname -s) != Linux || $(uname -m) != x86_64 ]]; then
  echo "Release packaging supports Linux x86_64." >&2
  exit 1
fi
binary=target/release/yoink
bun install --frozen-lockfile
bun run tauri build --no-bundle -- --locked
test "$("$binary" --version)" = "yoink $version"
test -s build/FRONTEND_LICENSES.txt
output=${1:-target/dist}
mkdir -p "$output"
output=$(realpath "$output")
stage=$(mktemp -d "$output/.stage.XXXXXX")
archive="yoink-v$version-x86_64-unknown-linux-gnu.tar.gz"
source_archive="yoink-v$version-source.tar.gz"
mkdir "$stage/binary" "$stage/source"
install -m 755 "$binary" "$stage/binary/yoink"
cp LICENSE NOTICE.md README.md build/FRONTEND_LICENSES.txt "$stage/binary/"
cp -R docs assets "$stage/binary/"
cp src-tauri/icons/128x128.png "$stage/binary/yoink.png"
cargo-about generate --locked --workspace packaging/licenses.hbs -o "$stage/binary/THIRD_PARTY_LICENSES.html"
git rev-parse HEAD > "$stage/binary/SOURCE_COMMIT"
git archive HEAD | tar -xf - -C "$stage/source"
mkdir -p "$stage/source/.cargo"
cargo vendor --locked "$stage/source/vendor" > "$stage/source/.cargo/config.toml"
sed -i 's|directory = ".*"|directory = "vendor"|' "$stage/source/.cargo/config.toml"
bun install --frozen-lockfile --cwd "$stage/source"
tar --owner=0 --group=0 --numeric-owner -czf "$output/$archive" -C "$stage/binary" .
tar --owner=0 --group=0 --numeric-owner -czf "$output/$source_archive" -C "$stage/source" .
(
  cd "$output"
  sha256sum "$archive" "$source_archive" > SHA256SUMS
)
printf 'Release files: %s\nStaging files: %s\n' "$output" "$stage"
