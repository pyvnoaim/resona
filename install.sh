#!/bin/sh
# Installs the latest Resona release on a Mac:
#   curl -fsSL https://raw.githubusercontent.com/pyvnoaim/resona/main/install.sh | sh
# Puts Resona.app into /Applications and `resona` (the terminal player) into ~/.local/bin.
# Run it again to update. Downloaded with curl, the app carries no quarantine flag, so Gatekeeper lets the
# unnotarised (ad-hoc signed) build open.
set -e
[ "$(uname)" = Darwin ] || { echo "install.sh is for macOS; on Linux and Windows download a build from the releases page"; exit 1; }
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
# The app's own update downloads the zip first (to show its progress) and hands it over in RESONA_ZIP.
zip=${RESONA_ZIP:-$tmp/Resona.zip}
if [ -z "$RESONA_ZIP" ]; then
  echo "downloading the latest Resona..."
  curl -fsSL https://github.com/pyvnoaim/resona/releases/latest/download/Resona-macos.zip -o "$zip"
fi
ditto -x -k "$zip" "$tmp"
rm -rf /Applications/Resona.app
mv "$tmp/Resona.app" /Applications/
touch /Applications/Resona.app
mkdir -p ~/.local/bin
printf '#!/bin/sh\nexec /Applications/Resona.app/Contents/MacOS/resona cli "$@"\n' > ~/.local/bin/resona
chmod +x ~/.local/bin/resona
case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) echo "note: add ~/.local/bin to your PATH to run \`resona\` anywhere" ;; esac
echo "installed /Applications/Resona.app: open it from Applications or Spotlight"
