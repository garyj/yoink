#!/bin/sh

set -eu

repository=garyj/yoink
version=latest
prefix=${HOME:?HOME is not set}/.local
uninstall=false
tmpdir=

usage() {
  cat <<'EOF'
Install yoink from a GitHub release.

Usage: install.sh [--version VERSION] [--prefix DIRECTORY]
       install.sh --uninstall [--prefix DIRECTORY]

Options:
  --version VERSION   Install a release such as 0.1.0 or v0.1.0.
  --prefix DIRECTORY  Install under DIRECTORY instead of ~/.local.
  --uninstall         Remove files installed under the selected prefix.
  -h, --help          Show this help.
EOF
}

fail() {
  printf 'yoink installer: %s\n' "$1" >&2
  exit 1
}

cleanup() {
  if [ -n "$tmpdir" ] && [ -d "$tmpdir" ]; then
    rm -rf "$tmpdir"
  fi
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version)
      [ "$#" -ge 2 ] || fail "--version needs a value"
      version=$2
      shift 2
      ;;
    --prefix)
      [ "$#" -ge 2 ] || fail "--prefix needs a value"
      prefix=$2
      shift 2
      ;;
    --uninstall)
      uninstall=true
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      fail "unknown option: $1"
      ;;
  esac
done

[ -n "$prefix" ] || fail "the installation prefix cannot be empty"
case "$prefix" in
  /*) ;;
  *) fail "the installation prefix must be an absolute path" ;;
esac

binary_path=$prefix/bin/yoink
desktop_path=$prefix/share/applications/dev.garyj.yoink.desktop
icon_path=$prefix/share/icons/hicolor/128x128/apps/dev.garyj.yoink.png
doc_path=$prefix/share/doc/yoink

if [ "$uninstall" = true ]; then
  rm -f "$binary_path" "$desktop_path" "$icon_path"
  rm -f "$doc_path/LICENSE" "$doc_path/NOTICE.md" "$doc_path/README.md"
  rm -f "$doc_path/FRONTEND_LICENSES.txt" "$doc_path/THIRD_PARTY_LICENSES.html"
  rm -f "$doc_path/SOURCE_COMMIT"
  rmdir "$doc_path" 2>/dev/null || true
  printf 'Removed yoink from %s. Saved history and configuration were kept.\n' "$prefix"
  exit 0
fi

[ "$(uname -s)" = Linux ] || fail "only Linux is supported"
[ "$(uname -m)" = x86_64 ] || fail "only Linux x86_64 is supported"

for command_name in awk curl sha256sum tar install mktemp; do
  command -v "$command_name" >/dev/null 2>&1 || fail "$command_name is required"
done

if [ "$version" = latest ]; then
  latest_url=$(curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL -o /dev/null \
    -w '%{url_effective}' "https://github.com/$repository/releases/latest")
  version=${latest_url##*/}
fi

version=${version#v}
case "$version" in
  [0-9]* ) ;;
  *) fail "invalid version: $version" ;;
esac
case "$version" in
  *[!0-9A-Za-z.+-]*) fail "invalid version: $version" ;;
esac

archive=yoink-v$version-x86_64-unknown-linux-gnu.tar.gz
release_url=https://github.com/$repository/releases/download/v$version
tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/yoink-install.XXXXXX")
trap cleanup EXIT HUP INT TERM

curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL --retry 3 \
  -o "$tmpdir/$archive" "$release_url/$archive"
curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL --retry 3 \
  -o "$tmpdir/SHA256SUMS" "$release_url/SHA256SUMS"

expected=$(awk -v archive="$archive" '$2 == archive || $2 == "*" archive { print $1; exit }' \
  "$tmpdir/SHA256SUMS")
[ -n "$expected" ] || fail "the release checksum does not list $archive"
actual=$(sha256sum "$tmpdir/$archive" | awk '{ print $1 }')
[ "$actual" = "$expected" ] || fail "checksum verification failed for $archive"

mkdir "$tmpdir/archive"
tar -xzf "$tmpdir/$archive" -C "$tmpdir/archive"
[ -x "$tmpdir/archive/yoink" ] || fail "the release archive does not contain yoink"

install -d "$prefix/bin" "$prefix/share/applications"
install -d "$prefix/share/icons/hicolor/128x128/apps" "$doc_path"
install -m 755 "$tmpdir/archive/yoink" "$binary_path"
install -m 644 "$tmpdir/archive/yoink.png" "$icon_path"
install -m 644 "$tmpdir/archive/LICENSE" "$doc_path/LICENSE"
install -m 644 "$tmpdir/archive/NOTICE.md" "$doc_path/NOTICE.md"
install -m 644 "$tmpdir/archive/README.md" "$doc_path/README.md"
install -m 644 "$tmpdir/archive/FRONTEND_LICENSES.txt" "$doc_path/FRONTEND_LICENSES.txt"
install -m 644 "$tmpdir/archive/THIRD_PARTY_LICENSES.html" "$doc_path/THIRD_PARTY_LICENSES.html"
install -m 644 "$tmpdir/archive/SOURCE_COMMIT" "$doc_path/SOURCE_COMMIT"

cat > "$tmpdir/dev.garyj.yoink.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=yoink
Comment=Clipboard history for Linux X11
Exec="$binary_path"
Icon=dev.garyj.yoink
Terminal=false
Categories=Utility;
EOF
install -m 644 "$tmpdir/dev.garyj.yoink.desktop" "$desktop_path"

printf 'Installed yoink %s to %s\n' "$version" "$binary_path"
case :$PATH: in
  *:"$prefix/bin":*) ;;
  *) printf 'Add %s/bin to PATH, or run %s directly.\n' "$prefix" "$binary_path" ;;
esac
