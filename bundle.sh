#!/bin/sh
# Builds Resona.app (pixel-speaker icon, Info.plist) and installs it into /Applications.
# `./bundle.sh dist` builds a universal (Apple silicon + Intel) app into target/Resona-macos.zip instead,
# without installing: what the release workflow uploads.
set -e
cd "$(dirname "$0")"
if [ "$1" = dist ]; then
  cargo build --release --target aarch64-apple-darwin
  cargo build --release --target x86_64-apple-darwin
  mkdir -p target/universal
  lipo -create -output target/universal/resona \
    target/aarch64-apple-darwin/release/resona target/x86_64-apple-darwin/release/resona
  bin=target/universal/resona
else
  cargo build --release
  bin=target/release/resona
fi

app=target/Resona.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin" "$app/Contents/MacOS/"

# The icon: the pixel speaker in your accent colour (red unless you picked another), drawn by the binary
# itself (`resona icon`: raw 1024px RGBA) and wrapped as a PNG here with stdlib only.
set=target/icon.iconset
rm -rf "$set" && mkdir "$set"
"$bin" icon | python3 -I -c '
import struct, sys, zlib
N = 1024
px = sys.stdin.buffer.read()
assert len(px) == N * N * 4, "resona icon: expected 1024x1024 RGBA"
rows = b"".join(b"\0" + px[y * N * 4:(y + 1) * N * 4] for y in range(N))
chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
open(sys.argv[1], "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", N, N, 8, 6, 0, 0, 0))
                               + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b""))
' "$set/icon_512x512@2x.png"
# sips -s format icns fails on newer macOS (Error 13); iconutil is the supported path.
iconutil -c icns "$set" -o "$app/Contents/Resources/icon.icns"
rm -rf "$set"

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Resona</string>
  <key>CFBundleExecutable</key><string>resona</string>
  <key>CFBundleIdentifier</key><string>dev.resona</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
EOF

codesign --force --deep -s - "$app"
if [ "$1" = dist ]; then
  rm -f target/Resona-macos.zip
  ditto -c -k --norsrc --noextattr --noacl --keepParent "$app" target/Resona-macos.zip
  echo "built target/Resona-macos.zip"
  exit
fi
rm -rf /Applications/Resona.app
mv "$app" /Applications/   # move, not copy: a second copy in target/ shows up in Spotlight
touch /Applications/Resona.app   # nudge Finder/Dock to pick up the new icon
# `resona` in any terminal starts the terminal player from the installed app.
mkdir -p ~/.local/bin
printf '#!/bin/sh\nexec /Applications/Resona.app/Contents/MacOS/resona cli "$@"\n' > ~/.local/bin/resona
chmod +x ~/.local/bin/resona
case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) echo "note: add ~/.local/bin to your PATH to run \`resona\` anywhere" ;; esac
echo "installed /Applications/Resona.app, and \`resona\` in ~/.local/bin"
