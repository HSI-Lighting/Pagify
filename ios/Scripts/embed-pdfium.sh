#!/bin/bash
# Put PDFium in the app bundle.
#
# It is dlopen'd by path rather than linked: bblanchon publishes no static
# archive at chromium/7881, and the pin is load-bearing — pdfium-render's
# bindings are generated against that exact API surface, and a mismatch fails by
# not resolving a symbol at the first render rather than at link time.
set -euo pipefail

VENDOR="$SRCROOT/../third_party/pdfium"

case "${PLATFORM_NAME}" in
    iphonesimulator) SOURCE="$VENDOR/pdfium-ios-simulator-arm64/lib/libpdfium.dylib" ;;
    iphoneos)        SOURCE="$VENDOR/pdfium-ios-device-arm64/lib/libpdfium.dylib" ;;
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
mkdir -p "$DESTINATION"
cp -f "$SOURCE" "$DESTINATION/libpdfium.dylib"

# A dylib inside a signed bundle has to carry the app's own signature, or the
# device refuses to map it. The simulator does not care, which is exactly why
# this is easy to leave out and only discover on hardware.
if [ "${CODE_SIGNING_ALLOWED:-NO}" = "YES" ] && [ -n "${EXPANDED_CODE_SIGN_IDENTITY:-}" ]; then
    codesign --force --sign "${EXPANDED_CODE_SIGN_IDENTITY}" \
        ${OTHER_CODE_SIGN_FLAGS:-} \
        --timestamp=none \
        "$DESTINATION/libpdfium.dylib"
fi

echo "PDFium: $(tr '\n' ' ' < "$(dirname "$(dirname "$SOURCE")")/VERSION")"
