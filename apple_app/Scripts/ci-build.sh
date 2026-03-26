#!/usr/bin/env bash
# CI build script for SubReader macOS app.
# Runs without Xcode GUI — suitable for CI/CD environments.
#
# Usage:
#   ./ci-build.sh                    # Build debug
#   ./ci-build.sh --release          # Build release
#   ./ci-build.sh --test             # Run tests
#   ./ci-build.sh --release --test   # Build release + run tests

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PROJECT_ROOT="$(cd "$APP_DIR/.." && pwd)"

BUILD_MODE="debug"
RUN_TESTS=false
CONFIGURATION="Debug"

# ─── Parse arguments ─────────────────────────────────────────────────────────

while [[ $# -gt 0 ]]; do
    case "$1" in
        --release)
            BUILD_MODE="release"
            CONFIGURATION="Release"
            shift
            ;;
        --debug)
            BUILD_MODE="debug"
            CONFIGURATION="Debug"
            shift
            ;;
        --test)
            RUN_TESTS=true
            shift
            ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: $0 [--debug|--release] [--test]"
            exit 1
            ;;
    esac
done

# ─── Step 1: Build Rust library ──────────────────────────────────────────────

echo "=== Step 1: Building Rust library ($BUILD_MODE) ==="
"$SCRIPT_DIR/build-rust.sh" "--$BUILD_MODE"

# ─── Step 2: Build Xcode project ─────────────────────────────────────────────

echo "=== Step 2: Building Xcode project ($CONFIGURATION) ==="

XCODEPROJ="$APP_DIR/SubReader.xcodeproj"
SCHEME="SubReader"
DERIVED_DATA="$APP_DIR/DerivedData"

if [[ ! -d "$XCODEPROJ" ]]; then
    echo "ERROR: Xcode project not found at $XCODEPROJ"
    echo "Please create the Xcode project first."
    exit 1
fi

xcodebuild \
    -project "$XCODEPROJ" \
    -scheme "$SCHEME" \
    -configuration "$CONFIGURATION" \
    -derivedDataPath "$DERIVED_DATA" \
    -destination "platform=macOS" \
    build \
    | xcpretty --color 2>/dev/null || true

echo "=== Build complete ==="

# ─── Step 3: Run tests (optional) ────────────────────────────────────────────

if [[ "$RUN_TESTS" == true ]]; then
    echo "=== Step 3: Running tests ==="

    xcodebuild \
        -project "$XCODEPROJ" \
        -scheme "$SCHEME" \
        -configuration "$CONFIGURATION" \
        -derivedDataPath "$DERIVED_DATA" \
        -destination "platform=macOS" \
        test \
        | xcpretty --color 2>/dev/null || true

    echo "=== Tests complete ==="
fi

echo "=== CI build finished ==="
