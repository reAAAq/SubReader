#!/usr/bin/env bash
# Build iOS static libraries and create XCFramework.
#
# Prerequisites:
#   - Xcode Command Line Tools installed
#   - Rust targets: aarch64-apple-ios, aarch64-apple-ios-sim, x86_64-apple-ios
#     Install via: rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CRATE_NAME="ffi_c"
LIB_NAME="libffi_c.a"
OUTPUT_DIR="$PROJECT_ROOT/target/ios"
XCFRAMEWORK_OUTPUT="$OUTPUT_DIR/ReaderCore.xcframework"

echo "=== Building iOS static libraries ==="

# Targets
TARGETS=(
    "aarch64-apple-ios"
    "aarch64-apple-ios-sim"
    "x86_64-apple-ios"
)

# Build each target
for TARGET in "${TARGETS[@]}"; do
    echo "--- Building for $TARGET ---"
    cargo build --manifest-path "$PROJECT_ROOT/Cargo.toml" \
        -p "$CRATE_NAME" \
        --release \
        --target "$TARGET"
done

echo "=== Creating universal simulator library ==="

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Create a universal (fat) library for simulators (arm64 + x86_64)
SIMULATOR_LIBS=(
    "$PROJECT_ROOT/target/aarch64-apple-ios-sim/release/$LIB_NAME"
    "$PROJECT_ROOT/target/x86_64-apple-ios/release/$LIB_NAME"
)

UNIVERSAL_SIM_LIB="$OUTPUT_DIR/sim-universal/$LIB_NAME"
mkdir -p "$(dirname "$UNIVERSAL_SIM_LIB")"

lipo -create "${SIMULATOR_LIBS[@]}" -output "$UNIVERSAL_SIM_LIB"

echo "=== Creating XCFramework ==="

# Remove old XCFramework if it exists
rm -rf "$XCFRAMEWORK_OUTPUT"

# Create XCFramework from device lib + universal simulator lib
xcodebuild -create-xcframework \
    -library "$PROJECT_ROOT/target/aarch64-apple-ios/release/$LIB_NAME" \
    -library "$UNIVERSAL_SIM_LIB" \
    -output "$XCFRAMEWORK_OUTPUT"

echo "=== XCFramework created at: $XCFRAMEWORK_OUTPUT ==="
echo "=== iOS build complete ==="
