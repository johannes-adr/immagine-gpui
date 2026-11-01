#!/usr/bin/env bash
# Builds the app for iOS and installs it.
#   ./build-ios.sh sim            iOS Simulator (booted device, or SIM=<name/udid>)
#   ./build-ios.sh device         connected iPhone (DEVICE=<udid> to pick one), signed with your Xcode team
#   ./build-ios.sh device --log   same, then launches with the console attached (Ctrl-C to stop)
# Debug builds are fast but slow to run; add RELEASE=1 for the optimized build (default on device).
# Needs: rustup target add aarch64-apple-ios aarch64-apple-ios-sim, xcodegen (device), an Apple ID signed in to Xcode.
set -euo pipefail
cd "$(dirname "$0")"

MODE="${1:-}"; LOG=false; [[ "${2:-}" == "--log" ]] && LOG=true
TEAM="${TEAM:-ZK5Q4XC8XR}"            # Xcode "Personal Team"; change in env or ios/project.yml
BUNDLE_ID="dev.johannesandrade.immagine"
export IPHONEOS_DEPLOYMENT_TARGET=15.0

case "$MODE" in
sim)
    PROFILE=debug; FLAGS=""; [[ "${RELEASE:-0}" == 1 ]] && { PROFILE=release; FLAGS="--release"; }
    cargo build --bin immagine-desktop --target aarch64-apple-ios-sim $FLAGS
    APP=target/ios/Immagine.app
    rm -rf "$APP"; mkdir -p "$APP"
    cp "target/aarch64-apple-ios-sim/$PROFILE/immagine-desktop" "$APP/immagine"
    # same Info.plist as the device build, with the Xcode variables filled in
    sed -e 's/$(EXECUTABLE_NAME)/immagine/; s/$(PRODUCT_BUNDLE_IDENTIFIER)/dev.immagine.spike/' ios/Info.plist > "$APP/Info.plist"
    codesign --force --sign - "$APP"
    DEV="${SIM:-booted}"
    xcrun simctl install "$DEV" "$APP"
    if $LOG; then xcrun simctl launch --console-pty --terminate-running-process "$DEV" dev.immagine.spike
    else xcrun simctl launch --terminate-running-process "$DEV" dev.immagine.spike; fi
    ;;
device)
    cargo build --release --bin immagine-desktop --target aarch64-apple-ios
    if [[ -z "${DEVICE:-}" ]]; then
        xcrun devicectl list devices --json-output /tmp/immagine-devices.json >/dev/null 2>&1 || true
        DEVICE="$(python3 -I - <<'PY'
import json
try:
    d = json.load(open("/tmp/immagine-devices.json"))["result"]["devices"]
except Exception:
    d = []
for x in d:
    hw = x.get("hardwareProperties", {})
    # A paired Apple Watch is a physical device too.
    if hw.get("reality") == "physical" and hw.get("platform") == "iOS" and x.get("connectionProperties", {}).get("tunnelState") != "unavailable":
        print(hw["udid"]); break
PY
)"
    fi
    [[ -n "$DEVICE" ]] || { echo "No iPhone found (connect it, unlock it, trust this Mac)." >&2; exit 1; }
    ( cd ios && xcodegen generate -q && xcodebuild -project Immagine.xcodeproj -scheme Immagine -configuration Release \
        -destination "id=$DEVICE" -derivedDataPath ./dd -allowProvisioningUpdates DEVELOPMENT_TEAM="$TEAM" build -quiet )
    # the Rust binary replaces Xcode's placeholder after the signing step, so sign again with the same identity and entitlements
    APP=ios/dd/Build/Products/Release-iphoneos/Immagine.app
    IDENT="$(security find-identity -v -p codesigning | awk -F'"' '/Apple Development/ {print $2; exit}')"
    [[ -n "$IDENT" ]] || { echo "No 'Apple Development' signing identity (sign in to Xcode > Settings > Accounts)." >&2; exit 1; }
    codesign --force --sign "$IDENT" --entitlements ios/dd/Build/Intermediates.noindex/Immagine.build/Release-iphoneos/Immagine.build/Immagine.app.xcent --generate-entitlement-der "$APP"
    xcrun devicectl device install app --device "$DEVICE" "$APP"
    if $LOG; then xcrun devicectl device process launch --console --terminate-existing --device "$DEVICE" "$BUNDLE_ID"
    else xcrun devicectl device process launch --terminate-existing --device "$DEVICE" "$BUNDLE_ID" || echo "Installed. Open it on the phone (first time: trust the developer profile in Settings > General > VPN & Device Management)."; fi
    ;;
*)
    sed -n 2,7p "$0" | sed 's/^# \{0,1\}//'; exit 1 ;;
esac
