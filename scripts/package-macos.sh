#!/bin/sh
# Builds dist/PeroxIDE.app (and a zip of it) from a release build.
# The app carries the runtime SDK in Contents/Resources/sdk so new GUI apps
# can depend on perox-ui / perox-slint without the source tree.
set -eu
cd "$(dirname "$0")/.."

VERSION="${1:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
cargo build --release -p peroxide

APP=dist/PeroxIDE.app
rm -rf "$APP" "dist/PeroxIDE-$VERSION-macos-arm64.zip"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/sdk"
cp target/release/peroxide "$APP/Contents/MacOS/peroxide"

# SDK: the permissive runtime crates plus a small workspace for their inherited fields.
SDK="$APP/Contents/Resources/sdk"
for c in perox-ui perox-slint; do
  mkdir -p "$SDK/$c"
  cp -R "crates/$c/Cargo.toml" "crates/$c/src" "$SDK/$c/"
  [ -f "crates/$c/README.md" ] && cp "crates/$c/README.md" "$SDK/$c/"
done
cp LICENSE-MIT LICENSE-APACHE "$SDK/"
cat > "$SDK/Cargo.toml" <<EOF
[workspace]
resolver = "3"
members = ["perox-ui", "perox-slint"]

[workspace.package]
version = "$VERSION"
edition = "2024"
rust-version = "1.100"
license = "MIT OR Apache-2.0"
repository = "https://github.com/deveshrai/perox-tools"
publish = false

[workspace.dependencies]
$(grep -E '^(rfd|slint|slint-build|i-slint-compiler) = ' Cargo.toml)

[workspace.lints.rust]
unsafe_code = "deny"
missing_debug_implementations = "warn"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
EOF

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>PeroxIDE</string>
  <key>CFBundleDisplayName</key><string>PeroxIDE</string>
  <key>CFBundleIdentifier</key><string>dev.perox.peroxide</string>
  <key>CFBundleExecutable</key><string>peroxide</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

# Ad-hoc signature so Apple silicon will run it; not notarized.
codesign --force --deep --sign - "$APP"
(cd dist && ditto -c -k --keepParent PeroxIDE.app "PeroxIDE-$VERSION-macos-arm64.zip")
echo "dist/PeroxIDE-$VERSION-macos-arm64.zip"
