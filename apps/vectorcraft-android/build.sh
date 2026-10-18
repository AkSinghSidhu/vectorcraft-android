#!/usr/bin/env bash
# Build VectorCraft's Android APK (arm64-v8a): the Rust library with cargo-ndk, then the APK with
# Gradle. Needs ANDROID_HOME (SDK with platform 35 and build-tools), ANDROID_NDK_HOME (r27 or
# newer), JDK 17, `rustup target add aarch64-linux-android` and `cargo install cargo-ndk`.
#
#   apps/vectorcraft-android/build.sh            # release APK
#   PROFILE=dev apps/vectorcraft-android/build.sh  # unoptimised library, faster to build
#   RELEASE_BUILD=3 apps/vectorcraft-android/build.sh  # the 3rd Android release of this version
#
# The APK lands in target/android/vectorcraft-<version>[-<build>]-arm64-v8a.apk.
# Optional: CRAFT_FONTS_DIR=<craft-fonts checkout> bundles its fonts into the app.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
PROFILE="${PROFILE:-release}"
API="${ANDROID_API:-26}"
RELEASE="${RELEASE_BUILD:-0}"

: "${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
: "${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to the Android NDK (r27 or newer)}"
command -v cargo-ndk >/dev/null || { echo "error: cargo-ndk not found (cargo install cargo-ndk)"; exit 1; }

cd "$ROOT"

# Keep this machine's folders (and the user name in them) out of the library: rustc records
# source paths for panic messages. Map the Cargo home, the workspace and the target directory to
# neutral prefixes. Cargo splits RUSTFLAGS on spaces, so a remap whose path contains one (e.g. a
# user's "Documents and Settings") would arrive at rustc as two broken arguments; skip those.
remap() {
    if [[ "$1" == *" "* ]] || [[ "$2" == *" "* ]]; then return 0; fi
    printf ' --remap-path-prefix=%s=%s' "$1" "$2"
    if command -v cygpath >/dev/null; then printf ' --remap-path-prefix=%s=%s' "$(cygpath -w "$1")" "$2"; fi
}

export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="$(remap "$HOME/.cargo" /cargo)$(remap "$ROOT" /vectorcraft)$(remap "$ROOT/target" /target)"

cargo ndk -t arm64-v8a -P "$API" -o "$HERE/android/app/src/main/jniLibs" \
    build -p vectorcraft-android --profile "$PROFILE"

echo "packaging the APK (gradle)…"
(cd "$HERE/android" && chmod +x ./gradlew 2>/dev/null; ./gradlew --no-daemon -q assembleRelease) || {
    echo "gradle failed; try: (cd $HERE/android && ./gradlew --no-daemon assembleRelease)"; exit 1; }

version="$(sed -n 's/^version = "\(.*\)".*/\1/p' "$ROOT/Cargo.toml" | head -1)"
mkdir -p "$ROOT/target/android"
if [[ "$RELEASE" > 0 ]]; then
    out="$ROOT/target/android/vectorcraft-$version-$RELEASE-arm64-v8a.apk"
else
    out="$ROOT/target/android/vectorcraft-$version-arm64-v8a.apk"
fi
cp "$HERE/android/app/build/outputs/apk/release/app-release.apk" "$out"
echo "wrote $out"
