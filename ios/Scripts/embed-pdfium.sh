#!/bin/bash
# Put PDFium in the app bundle, as a framework.
#
# It is dlopen'd by path rather than linked: bblanchon publishes no static
# archive at chromium/7881, and the pin is load-bearing — pdfium-render's
# bindings are generated against that exact API surface, and a mismatch fails by
# not resolving a symbol at the first render rather than at link time.
#
# **A framework bundle, not a bare .dylib, and that is not cosmetic.** iOS does
# not permit standalone dynamic libraries in an app bundle: the App Store
# validator refuses them with
#
#   ITMS-90171: Invalid Bundle Structure — The binary file
#   'Payload/Pagify.app/Frameworks/libpdfium.dylib' is not permitted. Your app
#   can't contain standalone executables or libraries, other than a valid
#   CFBundleExecutable of supported bundle types.
#
# and it does so at *upload*, long after `archive` and `-exportArchive` have both
# reported success. Xcode's own Packaging.log mentions it only as
# "[OPTIONAL] Didn't find info dictionary for … libpdfium.dylib", which is easy
# to read as harmless and is in fact the exact thing ITMS treats as fatal.
#
# The layout below is FLAT — binary and Info.plist directly inside the .framework,
# with no Versions/A and no symlinks. A macOS-style versioned framework is itself
# a rejection on iOS.
set -euo pipefail

VENDOR="$SRCROOT/../third_party/pdfium"

case "${PLATFORM_NAME}" in
    iphonesimulator) SOURCE="$VENDOR/pdfium-ios-simulator-arm64/lib/libpdfium.dylib"; VT_PLATFORM=iossim ;;
    iphoneos)        SOURCE="$VENDOR/pdfium-ios-device-arm64/lib/libpdfium.dylib";    VT_PLATFORM=ios ;;
    *)
        echo "error: no PDFium build for ${PLATFORM_NAME}" >&2
        exit 1
        ;;
esac

if [ ! -f "$SOURCE" ]; then
    echo "error: $SOURCE is missing. Fetch it with ios/Scripts/fetch-pdfium.sh" >&2
    exit 1
fi

DESTINATION="${TARGET_BUILD_DIR}/${FRAMEWORKS_FOLDER_PATH}"
FRAMEWORK="$DESTINATION/PDFium.framework"

mkdir -p "$FRAMEWORK"
# Rebuilt rather than updated: a stale bare libpdfium.dylib left beside the
# framework from an older build is itself the rejection this script exists to
# avoid.
rm -f "$DESTINATION/libpdfium.dylib"

# Re-stamped to the app's own minimum. bblanchon's CI stamps the binary with the
# SDK it built against (minos 26.0), not a real floor: it imports only
# libSystem/CoreGraphics/CoreFoundation basics, and renders identically on the
# iOS 18.6 simulator with either stamp. Left at 26.0 under a 17.0 Info.plist,
# App Store processing refuses the framework with 90208 — after archive and
# export have both succeeded.
SOURCE_SDK=$(otool -l "$SOURCE" | awk '/LC_BUILD_VERSION/{f=1} f&&/sdk/{print $2; exit}')
vtool -set-build-version "$VT_PLATFORM" "$IPHONEOS_DEPLOYMENT_TARGET" "$SOURCE_SDK" \
    -replace -output "$FRAMEWORK/PDFium" "$SOURCE"
chmod +x "$FRAMEWORK/PDFium"

# The install name has to match where it now lives, or dyld cannot find it from
# a load command — harmless here because the engine dlopens by absolute path,
# but wrong metadata in a shipped binary is worth not having.
install_name_tool -id "@rpath/PDFium.framework/PDFium" "$FRAMEWORK/PDFium" 2>/dev/null || true

# CFBundleExecutable is what makes this a bundle rather than a directory with a
# library in it, and is precisely what the validator looks for.
cat > "$FRAMEWORK/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleExecutable</key>
	<string>PDFium</string>
	<key>CFBundleIdentifier</key>
	<string>com.hsilighting.pagify.pdfium</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>PDFium</string>
	<key>CFBundlePackageType</key>
	<string>FMWK</string>
	<key>CFBundleShortVersionString</key>
	<string>1.0</string>
	<key>CFBundleVersion</key>
	<string>1</string>
	<key>MinimumOSVersion</key>
	<string>${IPHONEOS_DEPLOYMENT_TARGET}</string>
	<key>CFBundleSupportedPlatforms</key>
	<array>
		<string>${PLATFORM_NAME}</string>
	</array>
</dict>
</plist>
PLIST

# The whole framework is signed, not just the binary inside it. A framework
# whose bundle is unsigned is refused by the device even when its executable
# carries a valid signature.
if [ "${CODE_SIGNING_ALLOWED:-NO}" = "YES" ] && [ -n "${EXPANDED_CODE_SIGN_IDENTITY:-}" ]; then
    codesign --force --sign "${EXPANDED_CODE_SIGN_IDENTITY}" \
        ${OTHER_CODE_SIGN_FLAGS:-} \
        --timestamp=none \
        "$FRAMEWORK"
fi

echo "PDFium: $(tr '\n' ' ' < "$(dirname "$(dirname "$SOURCE")")/VERSION")"
