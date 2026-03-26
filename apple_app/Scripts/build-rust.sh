#!/usr/bin/env bash
# Build Rust ffi_c as a Universal macOS static library (arm64 + x86_64).
#
# Usage:
#   ./build-rust.sh           # Debug build (default)
#   ./build-rust.sh --debug   # Debug build with debug symbols
#   ./build-rust.sh --release # Release build with LTO + strip
#
# Prerequisites:
#   rustup target add aarch64-apple-darwin x86_64-apple-darwin

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CRATE_NAME="ffi_c"
LIB_NAME="libreader_core"
VENDOR_DIR="$SCRIPT_DIR/../SubReader/Vendor"

# ─── Parse arguments ─────────────────────────────────────────────────────────

BUILD_MODE="debug"
CARGO_FLAGS=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --release)
            BUILD_MODE="release"
            CARGO_FLAGS="--release"
            shift
            ;;
        --debug)
            BUILD_MODE="debug"
            shift
            ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: $0 [--debug|--release]"
            exit 1
            ;;
    esac
done

echo "=== Building Rust ffi_c for macOS ($BUILD_MODE) ==="

# ─── Targets ──────────────────────────────────────────────────────────────────

TARGETS=(
    "aarch64-apple-darwin"
    "x86_64-apple-darwin"
)

# ─── Build each target ────────────────────────────────────────────────────────

for TARGET in "${TARGETS[@]}"; do
    echo "--- Building for $TARGET ($BUILD_MODE) ---"
    cargo build --manifest-path "$PROJECT_ROOT/Cargo.toml" \
        -p "$CRATE_NAME" \
        $CARGO_FLAGS \
        --target "$TARGET"
done

# ─── Create Universal Binary ─────────────────────────────────────────────────

echo "=== Creating Universal Binary ==="

ARM64_LIB="$PROJECT_ROOT/target/aarch64-apple-darwin/$BUILD_MODE/libffi_c.a"
X86_LIB="$PROJECT_ROOT/target/x86_64-apple-darwin/$BUILD_MODE/libffi_c.a"

# Verify both libraries exist
if [[ ! -f "$ARM64_LIB" ]]; then
    echo "ERROR: arm64 library not found at $ARM64_LIB"
    exit 1
fi
if [[ ! -f "$X86_LIB" ]]; then
    echo "ERROR: x86_64 library not found at $X86_LIB"
    exit 1
fi

mkdir -p "$VENDOR_DIR"

UNIVERSAL_LIB="$VENDOR_DIR/${LIB_NAME}.a"
lipo -create "$ARM64_LIB" "$X86_LIB" -output "$UNIVERSAL_LIB"

echo "Universal library: $UNIVERSAL_LIB"
lipo -info "$UNIVERSAL_LIB"

# ─── Copy header ─────────────────────────────────────────────────────────────

HEADER_SRC="$PROJECT_ROOT/target/reader_core.h"
HEADER_DST="$VENDOR_DIR/reader_core.h"

if [[ -f "$HEADER_SRC" ]]; then
    cp "$HEADER_SRC" "$HEADER_DST"
    echo "Header copied: $HEADER_DST"
else
    echo "WARNING: reader_core.h not found at $HEADER_SRC"
    echo "Run 'cargo build -p ffi_c' first to generate the header."
fi

# ─── Summary ──────────────────────────────────────────────────────────────────

echo ""
echo "=== macOS build complete ==="
echo "  Library: $UNIVERSAL_LIB"
echo "  Header:  $HEADER_DST"
echo "  Mode:    $BUILD_MODE"
