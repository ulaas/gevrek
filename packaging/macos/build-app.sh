#!/usr/bin/env bash
# Build gevrek for macOS as an application bundle, signed:
#
#     packaging/macos/build-app.sh            -> target/macos/Gevrek.app
#
# The bundle holds the window manager (its executable), the command line,
# the shell and the settings program. macOS gives the Accessibility permission to
# the bundle (its id and its signature's team), so it is kept from one
# build to the next, as it is not for a bare binary started from a
# terminal (which gets the terminal's).
#
# Signed with GEVREK_SIGN_IDENTITY where it is set (a name or a hash, as
# `security find-identity -v -p codesigning` lists them); else with the
# first "Developer ID Application" identity of the keychain, else the first
# "Apple Development" one; else ad hoc, which works on this Mac but loses
# the permission with every build.
#
# Not notarized: for a bundle given to others, notarize it afterwards
# (`xcrun notarytool submit ... --wait`, then `xcrun stapler staple`).
set -euo pipefail
cd "$(dirname "$0")/../.."

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
target_dir="${CARGO_TARGET_DIR:-target}"
out="target/macos"
app="$out/Gevrek.app"

echo "Building gevrek $version (release)..."
cargo build --release --bin gevrek-wm --bin gevrek --bin gevrek-studio
# (The shell is a package of its own.)
cargo build --release -p gevrek-shell

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
for program in gevrek-wm gevrek gevrek-studio gevrek-shell; do
    cp "$target_dir/release/$program" "$app/Contents/MacOS/"
done
sed "s/@VERSION@/$version/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"
cp assets/brand/app-icon/macos/gevrek.icns "$app/Contents/Resources/gevrek.icns"
plutil -lint "$app/Contents/Info.plist" >/dev/null

identity="${GEVREK_SIGN_IDENTITY:-}"
if [ -z "$identity" ]; then
    identities=$(security find-identity -v -p codesigning 2>/dev/null || true)
    for kind in "Developer ID Application" "Apple Development"; do
        identity=$(printf '%s\n' "$identities" | sed -n "s/.*\"\($kind: .*\)\"/\1/p" | head -1)
        [ -n "$identity" ] && break
    done
fi
if [ -z "$identity" ]; then
    echo "No signing identity: signed ad hoc (the Accessibility permission is lost with every build)."
    identity="-"
fi
echo "Signing with: $identity"
# The programs inside first, then the bundle (whose executable is the
# window manager). The hardened runtime is what notarization asks for;
# gevrek needs no exception from it, only leave to ask for the location
# (entitlements.plist: the wifi's networks are named only then).
entitlements="$(dirname "$0")/entitlements.plist"
for program in gevrek gevrek-studio gevrek-shell; do
    codesign --force --options runtime --timestamp=none --sign "$identity" \
        --entitlements "$entitlements" \
        --identifier "io.github.ulaas.gevrek.$program" "$app/Contents/MacOS/$program"
done
codesign --force --options runtime --timestamp=none --sign "$identity" \
    --entitlements "$entitlements" "$app"
codesign --verify --strict --deep "$app"

echo "Built $app"
echo "Install it:  packaging/macos/install.sh"
