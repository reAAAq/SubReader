#!/usr/bin/env bash
# Build Android shared libraries (.so) for ARM64 and ARM32.
#
# Prerequisites:
#   - Android NDK installed (set ANDROID_NDK_HOME environment variable)
#   - Rust targets: aarch64-linux-android, armv7-linux-androideabi
#     Install via: rustup target add aarch64-linux-android armv7-linux-androideabi
#   - cargo-ndk installed: cargo install cargo-ndk

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CRATE_NAME="ffi_c"
OUTPUT_DIR="$PROJECT_ROOT/target/android"

# Minimum Android API level
MIN_API_LEVEL="${ANDROID_MIN_API:-21}"

echo "=== Building Android shared libraries ==="

# Check for Android NDK
if [ -z "${ANDROID_NDK_HOME:-}" ]; then
    echo "WARNING: ANDROID_NDK_HOME is not set."
    echo "Attempting to use cargo-ndk which may auto-detect the NDK."
fi

# Targets and their corresponding Android ABI names
declare -A TARGET_ABI_MAP
TARGET_ABI_MAP=(
    ["aarch64-linux-android"]="arm64-v8a"
    ["armv7-linux-androideabi"]="armeabi-v7a"
)

# Build each target
for TARGET in "${!TARGET_ABI_MAP[@]}"; do
    ABI="${TARGET_ABI_MAP[$TARGET]}"
    echo "--- Building for $TARGET (ABI: $ABI) ---"

    # Use cargo-ndk if available, otherwise fall back to raw cargo
    if command -v cargo-ndk &> /dev/null; then
        cargo ndk \
            --manifest-path "$PROJECT_ROOT/Cargo.toml" \
            --target "$TARGET" \
            --platform "$MIN_API_LEVEL" \
            -- build -p "$CRATE_NAME" --release
    else
        cargo build \
            --manifest-path "$PROJECT_ROOT/Cargo.toml" \
            -p "$CRATE_NAME" \
            --release \
            --target "$TARGET"
    fi

    # Copy output to organized directory
    SO_FILE="$PROJECT_ROOT/target/$TARGET/release/libffi_c.so"
    DEST_DIR="$OUTPUT_DIR/jniLibs/$ABI"
    mkdir -p "$DEST_DIR"

    if [ -f "$SO_FILE" ]; then
        cp "$SO_FILE" "$DEST_DIR/libreader_core.so"
        echo "  -> Copied to $DEST_DIR/libreader_core.so"
    else
        echo "  WARNING: $SO_FILE not found. Build may have failed."
    fi
done

echo "=== Android build complete ==="
echo "Output directory: $OUTPUT_DIR/jniLibs/"
ls -la "$OUTPUT_DIR/jniLibs/"*/ 2>/dev/null || echo "(no output files yet)"
