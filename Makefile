# SubReader — Unified Build Commands
#
# Usage:
#   make build-rust          Build Rust library (debug)
#   make build-rust-release  Build Rust library (release)
#   make build-app           Build macOS app (debug)
#   make build-app-release   Build macOS app (release)
#   make test-rust           Run Rust tests
#   make test-app            Run Swift tests
#   make test                Run all tests
#   make release             Full release build
#   make clean               Clean all build artifacts
#   make header              Regenerate C header only

.PHONY: all build-rust build-rust-release build-app build-app-release \
        test-rust test-app test release clean header ci

# ─── Paths ────────────────────────────────────────────────────────────────────

PROJECT_ROOT := $(shell pwd)
APPLE_APP_DIR := $(PROJECT_ROOT)/apple_app
SCRIPTS_DIR := $(APPLE_APP_DIR)/Scripts

# ─── Rust Library ─────────────────────────────────────────────────────────────

build-rust: header
	@echo "=== Building Rust library (debug) ==="
	@bash $(SCRIPTS_DIR)/build-rust.sh --debug

build-rust-release: header
	@echo "=== Building Rust library (release) ==="
	@bash $(SCRIPTS_DIR)/build-rust.sh --release

header:
	@echo "=== Generating C header ==="
	@cargo build -p ffi_c

test-rust:
	@echo "=== Running Rust tests ==="
	@cargo test --workspace

# ─── macOS App ────────────────────────────────────────────────────────────────

build-app: build-rust
	@echo "=== Building macOS app (debug) ==="
	@if [ -d "$(APPLE_APP_DIR)/SubReader.xcodeproj" ]; then \
		xcodebuild -project $(APPLE_APP_DIR)/SubReader.xcodeproj \
			-scheme SubReader -configuration Debug \
			-destination "platform=macOS" build; \
	else \
		echo "Xcode project not found. Skipping app build."; \
	fi

build-app-release: build-rust-release
	@echo "=== Building macOS app (release) ==="
	@if [ -d "$(APPLE_APP_DIR)/SubReader.xcodeproj" ]; then \
		xcodebuild -project $(APPLE_APP_DIR)/SubReader.xcodeproj \
			-scheme SubReader -configuration Release \
			-destination "platform=macOS" build; \
	else \
		echo "Xcode project not found. Skipping app build."; \
	fi

test-app: build-rust
	@echo "=== Running Swift tests ==="
	@if [ -d "$(APPLE_APP_DIR)/SubReader.xcodeproj" ]; then \
		xcodebuild -project $(APPLE_APP_DIR)/SubReader.xcodeproj \
			-scheme SubReader -configuration Debug \
			-destination "platform=macOS" test; \
	else \
		echo "Xcode project not found. Skipping app tests."; \
	fi

# ─── Combined ─────────────────────────────────────────────────────────────────

test: test-rust test-app

release: build-rust-release build-app-release
	@echo "=== Release build complete ==="

ci:
	@bash $(SCRIPTS_DIR)/ci-build.sh --release --test

clean:
	@echo "=== Cleaning build artifacts ==="
	@cargo clean
	@rm -rf $(APPLE_APP_DIR)/DerivedData
	@rm -rf $(APPLE_APP_DIR)/SubReader/Vendor/libreader_core.a
	@rm -rf $(APPLE_APP_DIR)/SubReader/Vendor/reader_core.h
	@echo "=== Clean complete ==="

all: build-app
