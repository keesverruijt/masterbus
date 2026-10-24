#!/bin/sh
# Install the masterbus command-line tools on Linux or macOS:
#
#   curl -fsSL https://raw.githubusercontent.com/keesverruijt/masterbus/main/install.sh | sh
#
# Downloads the latest release for this machine and puts masterbus-tui,
# masterbus-signalk, masterbus-set-field and masterbus-dump in /usr/local/bin
# (asking for your password if that needs sudo). Set MASTERBUS_INSTALL_DIR to
# install elsewhere, or MASTERBUS_VERSION (e.g. v0.4.6) for a given release.
set -eu

REPO=keesverruijt/masterbus
TOOLS="masterbus-tui masterbus-signalk masterbus-set-field masterbus-dump"
DIR=${MASTERBUS_INSTALL_DIR:-/usr/local/bin}

fail() {
	echo "masterbus install: $*" >&2
	exit 1
}

case "$(uname -s)/$(uname -m)" in
Linux/x86_64 | Linux/amd64) target=x86_64-unknown-linux-musl ;;
Linux/aarch64 | Linux/arm64) target=aarch64-unknown-linux-musl ;;
Linux/armv7l | Linux/armv8l) target=armv7-unknown-linux-musleabihf ;;
Darwin/arm64) target=aarch64-apple-darwin ;;
Darwin/x86_64) target=x86_64-apple-darwin ;;
*) fail "no ready-made build for $(uname -s) $(uname -m); see CONTRIBUTING.md to build it yourself" ;;
esac

if [ -n "${MASTERBUS_VERSION:-}" ]; then
	url="https://github.com/$REPO/releases/download/$MASTERBUS_VERSION/masterbus-$target.tar.gz"
else
	url="https://github.com/$REPO/releases/latest/download/masterbus-$target.tar.gz"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "Downloading $url"
if command -v curl >/dev/null 2>&1; then
	curl -fsSL "$url" -o "$tmp/masterbus.tar.gz" || fail "download failed"
elif command -v wget >/dev/null 2>&1; then
	wget -qO "$tmp/masterbus.tar.gz" "$url" || fail "download failed"
else
	fail "needs curl or wget"
fi
tar xzf "$tmp/masterbus.tar.gz" -C "$tmp"

sudo=
if ! mkdir -p "$DIR" 2>/dev/null || [ ! -w "$DIR" ]; then
	command -v sudo >/dev/null 2>&1 || fail "cannot write to $DIR; set MASTERBUS_INSTALL_DIR to a folder you own"
	echo "Installing to $DIR needs administrator rights; sudo may ask for your password."
	sudo=sudo
	$sudo mkdir -p "$DIR"
fi
for tool in $TOOLS; do
	$sudo install -m 755 "$tmp/masterbus-$target/$tool" "$DIR/$tool"
done

echo
echo "Installed $TOOLS in $DIR."
case ":$PATH:" in
*":$DIR:"*) echo "Start with: masterbus-tui" ;;
*) echo "$DIR is not on your PATH; start with: $DIR/masterbus-tui" ;;
esac
