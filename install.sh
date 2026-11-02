#!/bin/sh
# Install the Baton CLI from GitHub Releases. Override BATON_VERSION or BATON_INSTALL_DIR if needed.
set -eu

case "$(uname -s)" in
  Linux) os=unknown-linux-gnu ;;
  Darwin) os=apple-darwin ;;
  *) echo "Unsupported operating system: $(uname -s)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64|amd64) arch=x86_64 ;;
  arm64|aarch64) arch=aarch64 ;;
  *) echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

repo=https://github.com/klab365/baton
asset="baton-${arch}-${os}.tar.gz"
if [ -n "${BATON_VERSION:-}" ]; then
  case "$BATON_VERSION" in
    v*) tag=$BATON_VERSION ;;
    *) tag="v$BATON_VERSION" ;;
  esac
  url="$repo/releases/download/$tag/$asset"
else
  url="$repo/releases/latest/download/$asset"
fi
install_dir=${BATON_INSTALL_DIR:-"$HOME/.local/bin"}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
curl -fLsS "$url" -o "$tmp/baton.tar.gz"
tar -xzf "$tmp/baton.tar.gz" -C "$tmp"
mkdir -p "$install_dir"
install -m 755 "$tmp/baton" "$install_dir/baton"
printf 'Installed baton to %s/baton\n' "$install_dir"
printf 'Ensure %s is on your PATH. Run baton in your project and open the displayed address.\n' "$install_dir"
