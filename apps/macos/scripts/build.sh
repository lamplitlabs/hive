#!/bin/sh
# Builds PULSE.app: the Swift package in apps/macos, the console's own fonts
# (web/hub/public/fonts), the icon (Resources/AppIcon.svg), an Info.plist, and
# a signature whose requirement every build meets, so macOS keeps its answers
# (folder access, notifications) across rebuilds - the same idea as the
# `identifier "pulse"` signature `pulse service install` gives pulse (ADR-035).
#
#   apps/macos/scripts/build.sh              build apps/macos/build/PULSE.app (release)
#   apps/macos/scripts/build.sh --install    and copy it to /Applications (~/Applications when not writable)
#   apps/macos/scripts/build.sh --open       and open it
#   apps/macos/scripts/build.sh --dmg        and make apps/macos/build/PULSE.dmg
#   apps/macos/scripts/build.sh --universal  one binary for Apple silicon and Intel
#   apps/macos/scripts/build.sh --debug      a debug build (faster, with assertions)
#
# Distribution outside this Mac needs a Developer ID certificate:
#   SIGN_IDENTITY="Developer ID Application: Name (TEAMID)" apps/macos/scripts/build.sh --dmg
#   NOTARY_PROFILE=<notarytool keychain profile> ...      also notarizes and staples the disk image
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
repo=$(cd "$here/../.." && pwd)
bundle_id=io.github.manishtiwari25.pulse
out="$here/build"
app="$out/PULSE.app"

install=0 open_app=0 dmg=0 universal=0 configuration=release
for arg in "$@"; do
  case "$arg" in
    --install) install=1 ;;
    --open) open_app=1 ;;
    --dmg) dmg=1 ;;
    --universal) universal=1 ;;
    --debug) configuration=debug ;;
    -h|--help) sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "build.sh: unknown option $arg (see --help)" >&2; exit 2 ;;
  esac
done

say() { printf '%s\n' "$*" >&2; }

# The version PULSE's workspace carries, and a build number from the commit time.
version=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' "$repo/Cargo.toml" | head -1)
[ -n "$version" ] || version=0.1.0
build=$(git -C "$repo" log -1 --format=%cd --date=format:%Y%m%d.%H%M 2>/dev/null || date +%Y%m%d.%H%M)
revision=$(git -C "$repo" rev-parse --short=12 HEAD 2>/dev/null || echo unknown)
if [ -n "$(git -C "$repo" status --porcelain -- apps/macos 2>/dev/null)" ]; then revision="$revision+dirty"; fi

arch_flags=""
if [ "$universal" = 1 ]; then arch_flags="--arch arm64 --arch x86_64"; fi
# SwiftPM links without -isysroot, which records the deployment target as the
# SDK the app was built with; AppKit and WebKit then behave as if built for
# macOS 13 (window chrome, linked-on-or-after defaults). Name the real SDK.
sdk=$(xcrun --sdk macosx --show-sdk-path)
sdk_flags="-Xswiftc -Xclang-linker -Xswiftc -isysroot -Xswiftc -Xclang-linker -Xswiftc $sdk"

say "Building PULSE.app $version ($build, $revision, $configuration)"
# shellcheck disable=SC2086
swift build --package-path "$here" -c "$configuration" --product PULSE $arch_flags $sdk_flags >&2
# shellcheck disable=SC2086
bin_dir=$(swift build --package-path "$here" -c "$configuration" --product PULSE $arch_flags --show-bin-path)
[ -x "$bin_dir/PULSE" ] || { say "build.sh: no PULSE binary in $bin_dir"; exit 1; }

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/Fonts"
cp "$bin_dir/PULSE" "$app/Contents/MacOS/PULSE"
# The linker adds a search path into the Xcode that built it; the app only
# uses the Swift runtime macOS ships in /usr/lib/swift. A universal binary
# lists each path once per slice; one deletion removes it from every slice.
otool -l "$app/Contents/MacOS/PULSE" | awk '/LC_RPATH/ { getline; getline; print $2 }' | sort -u | while read -r rpath; do
  case "$rpath" in
    /Applications/*|*/Toolchains/*|*/Developer/*) install_name_tool -delete_rpath "$rpath" "$app/Contents/MacOS/PULSE" ;;
  esac
done
linked_sdk=$(otool -l "$app/Contents/MacOS/PULSE" | awk '/LC_BUILD_VERSION/ { found = 1 } found && $1 == "sdk" { print $2; exit }')
case "$linked_sdk" in
  13.*|"") say "build.sh: the binary records SDK '$linked_sdk', not $(xcrun --sdk macosx --show-sdk-version)"; exit 1 ;;
esac
printf 'APPL????' > "$app/Contents/PkgInfo"

# The console's typefaces, for the screens the app draws itself.
fonts="$repo/web/hub/public/fonts"
if [ -d "$fonts" ]; then
  cp "$fonts"/*.woff2 "$fonts"/OFL-*.txt "$app/Contents/Resources/Fonts/" 2>/dev/null || true
fi

# AppIcon.icns from AppIcon.svg, rendered again only when the SVG changed.
svg="$here/Resources/AppIcon.svg"
icns="$out/AppIcon.icns"
if [ ! -f "$icns" ] || [ "$svg" -nt "$icns" ]; then
  say "Rendering the icon"
  iconset="$out/AppIcon.iconset"
  rm -rf "$iconset"
  mkdir -p "$iconset"
  # sips keeps the transparent corners (Quick Look would paint them white).
  icon_ok=1
  for size in 16 32 128 256 512; do
    sips -s format png --resampleHeightWidth "$size" "$size" "$svg" --out "$iconset/icon_${size}x${size}.png" >/dev/null 2>&1 || icon_ok=0
    double=$((size * 2))
    sips -s format png --resampleHeightWidth "$double" "$double" "$svg" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null 2>&1 || icon_ok=0
  done
  if [ "$icon_ok" = 1 ]; then
    iconutil -c icns "$iconset" -o "$icns"
  else
    say "build.sh: sips could not render $svg on this macOS; the app gets the generic icon"
    rm -f "$icns"
  fi
  rm -rf "$iconset"
fi
if [ -f "$icns" ]; then cp "$icns" "$app/Contents/Resources/AppIcon.icns"; fi

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>PULSE</string>
  <key>CFBundleExecutable</key><string>PULSE</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundleIdentifier</key><string>$bundle_id</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>PULSE</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$build</string>
  <key>PulseRevision</key><string>$revision</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>LSMinimumSystemVersion</key><string>13.3</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSPrincipalClass</key><string>NSApplication</string>
  <key>NSSupportsAutomaticTermination</key><false/>
  <key>NSSupportsSuddenTermination</key><false/>
  <key>NSHumanReadableCopyright</key><string>PULSE, MIT licence.</string>
  <key>NSDownloadsFolderUsageDescription</key><string>PULSE saves what you download from the console (chat files, run transcripts) in your Downloads folder.</string>
  <key>NSAppTransportSecurity</key>
  <dict>
    <!-- The console is plain HTTP on 127.0.0.1. -->
    <key>NSAllowsLocalNetworking</key><true/>
  </dict>
</dict>
</plist>
PLIST
plutil -lint "$app/Contents/Info.plist" >/dev/null

# Ad hoc by default: no certificate, and a requirement every build meets.
if [ -n "${SIGN_IDENTITY:-}" ]; then
  codesign --force --sign "$SIGN_IDENTITY" --options runtime --timestamp "$app"
else
  codesign --force --sign - --identifier "$bundle_id" \
    --requirements "=designated => identifier \"$bundle_id\"" --options runtime "$app"
fi
codesign --verify --strict "$app"
say "Built $app"

target="$app"
if [ "$install" = 1 ]; then
  dest=/Applications
  [ -w "$dest" ] || dest="$HOME/Applications"
  mkdir -p "$dest"
  target="$dest/PULSE.app"
  if pgrep -f "$target/Contents/MacOS/PULSE" >/dev/null 2>&1; then
    say "Quitting the running PULSE.app first"
    osascript -e "tell application id \"$bundle_id\" to quit" >/dev/null 2>&1 || true
    i=0
    while pgrep -f "$target/Contents/MacOS/PULSE" >/dev/null 2>&1 && [ $i -lt 40 ]; do sleep 0.5; i=$((i + 1)); done
  fi
  case "$target" in */PULSE.app) rm -rf "$target" ;; esac
  ditto "$app" "$target"
  /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$target" >/dev/null 2>&1 || true
  say "Installed $target"
fi

if [ "$dmg" = 1 ]; then
  stage="$out/dmg"
  rm -rf "$stage" "$out/PULSE.dmg"
  mkdir -p "$stage"
  ditto "$app" "$stage/PULSE.app"
  ln -s /Applications "$stage/Applications"
  hdiutil create -volname PULSE -srcfolder "$stage" -ov -format UDZO "$out/PULSE.dmg" >/dev/null
  rm -rf "$stage"
  if [ -n "${SIGN_IDENTITY:-}" ]; then
    codesign --force --sign "$SIGN_IDENTITY" --timestamp "$out/PULSE.dmg"
  fi
  if [ -n "${NOTARY_PROFILE:-}" ]; then
    xcrun notarytool submit "$out/PULSE.dmg" --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple "$out/PULSE.dmg"
  fi
  say "Made $out/PULSE.dmg"
fi

if [ "$open_app" = 1 ]; then
  open "$target"
fi
printf '%s\n' "$target"
