#!/usr/bin/env bash
# Build WASM module from ffi_wasm crate.
#
# Prerequisites:
#   - wasm-pack installed: curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
#   - wasm-opt installed (part of binaryen): brew install binaryen
#   - Rust target: wasm32-unknown-unknown
#     Install via: rustup target add wasm32-unknown-unknown

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CRATE_DIR="$PROJECT_ROOT/crates/ffi_wasm"
OUTPUT_DIR="$PROJECT_ROOT/target/wasm"

echo "=== Building WASM module ==="

# Check for wasm-pack
if ! command -v wasm-pack &> /dev/null; then
    echo "ERROR: wasm-pack is not installed."
    echo "Install it via: curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh"
    exit 1
fi

# Build with wasm-pack
echo "--- Running wasm-pack build ---"
wasm-pack build "$CRATE_DIR" \
    --target web \
    --release \
    --out-dir "$OUTPUT_DIR/pkg"

echo "--- WASM build output ---"
ls -la "$OUTPUT_DIR/pkg/"

# Optimize with wasm-opt if available
WASM_FILE="$OUTPUT_DIR/pkg/ffi_wasm_bg.wasm"
if command -v wasm-opt &> /dev/null; then
    if [ -f "$WASM_FILE" ]; then
        echo "--- Optimizing WASM with wasm-opt -Oz ---"
        ORIGINAL_SIZE=$(wc -c < "$WASM_FILE")
        wasm-opt -Oz "$WASM_FILE" -o "$WASM_FILE.opt"
        mv "$WASM_FILE.opt" "$WASM_FILE"
        OPTIMIZED_SIZE=$(wc -c < "$WASM_FILE")
        echo "  Original size:  $ORIGINAL_SIZE bytes"
        echo "  Optimized size: $OPTIMIZED_SIZE bytes"
        echo "  Saved: $(( ORIGINAL_SIZE - OPTIMIZED_SIZE )) bytes"
    fi
else
    echo "WARNING: wasm-opt not found. Skipping optimization."
    echo "Install binaryen for wasm-opt: brew install binaryen"
fi

echo "=== WASM build complete ==="
echo "Output directory: $OUTPUT_DIR/pkg/"
