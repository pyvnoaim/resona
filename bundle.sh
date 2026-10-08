#!/bin/sh
# Builds Resona.app (pixel-speaker icon, Info.plist) and installs it into /Applications.
set -e
cd "$(dirname "$0")"
cargo build --release

app=target/Resona.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/resona "$app/Contents/MacOS/"

# The icon: Wired red pixel speaker on a near-black rounded square, 1024px, drawn with stdlib only.
set=target/icon.iconset
rm -rf "$set" && mkdir "$set"
python3 -I - "$set/icon_512x512@2x.png" <<'EOF'
import struct, sys, zlib
art = """
......X........
.....XX........
....XXX...X....
XXXXXXX....X...
XXXXXXX.X...X..
XXXXXXX..X..X..
XXXXXXX..X..X..
XXXXXXX..X..X..
XXXXXXX.X...X..
XXXXXXX....X...
....XXX...X....
.....XX........
......X........
""".split()
N, cell, pad, r = 1024, 44, 100, 180
ox, oy = (N - cell * len(art[0])) // 2 + cell, (N - cell * len(art)) // 2
red, panel, clear = (224, 40, 46, 255), (11, 10, 11, 255), (0, 0, 0, 0)

def px(x, y):
    # rounded square body
    cx, cy = min(max(x, pad + r), N - pad - r), min(max(y, pad + r), N - pad - r)
    if not (pad <= x < N - pad and pad <= y < N - pad) or (x - cx) ** 2 + (y - cy) ** 2 > r * r:
        return clear
    i, j = (y - oy) // cell, (x - ox) // cell
    return red if 0 <= i < len(art) and 0 <= j < len(art[0]) and y >= oy and x >= ox and art[i][j] == "X" else panel

rows = b"".join(b"\0" + bytes(c for x in range(N) for c in px(x, y)) for y in range(N))
chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
open(sys.argv[1], "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", N, N, 8, 6, 0, 0, 0))
                               + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b""))
EOF
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
rm -rf /Applications/Resona.app
mv "$app" /Applications/   # move, not copy: a second copy in target/ shows up in Spotlight
touch /Applications/Resona.app   # nudge Finder/Dock to pick up the new icon
# `resona` in any terminal starts the terminal player from the installed app.
mkdir -p ~/.local/bin
printf '#!/bin/sh\nexec /Applications/Resona.app/Contents/MacOS/resona cli "$@"\n' > ~/.local/bin/resona
chmod +x ~/.local/bin/resona
case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) echo "note: add ~/.local/bin to your PATH to run \`resona\` anywhere" ;; esac
echo "installed /Applications/Resona.app, and \`resona\` in ~/.local/bin"
