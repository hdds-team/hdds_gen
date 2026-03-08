# SPDX-License-Identifier: MIT

# Makefile for IDL Parser (Rust)
# Provides convenient shortcuts for common tasks

.PHONY: all build release test test-all test-prod test-roundtrip clean dist install run help fmt clippy doc
.PHONY: validate-ci

# Default target
all: build

# Build the project (debug mode)
build:
	@echo "🔨 Building IDL parser (debug mode)..."
	cargo build

# Build release version (optimized)
release:
	@echo "🚀 Building IDL parser (release mode)..."
	cargo build --release

# Run unit tests
test:
	@echo "🧪 Running unit tests..."
	cargo test

# Run all production IDL file tests (50 files)
test-all: build
	@echo "🔥 Running production test suite (50 files)..."
	@./test_all_idl.sh

# Alias for test-all
test-prod: test-all

# Cross-language CDR2 roundtrip test (Python, C, C++)
test-roundtrip: release
	@echo "🔄 Running codegen roundtrip test..."
	@./scripts/test-codegen-roundtrip.sh

# Run tests with coverage (requires cargo-tarpaulin)
test-coverage:
	@echo "📊 Running tests with coverage..."
	@if command -v cargo-tarpaulin > /dev/null; then \
		cargo tarpaulin --out Html --output-dir coverage; \
		echo "Coverage report generated in coverage/"; \
	else \
		echo "❌ cargo-tarpaulin not installed. Run: cargo install cargo-tarpaulin"; \
	fi

# Clean build artifacts
clean:
	@echo "🧹 Cleaning build artifacts..."
	cargo clean
	@rm -rf dist/
	@rm -rf coverage/
	@rm -rf fuzz/target/
	@rm -f hddsgen-*.tar.gz
	@rm -f hddsgen-*.zip
	@echo "✅ Clean complete"

# Create distribution package
dist: release
	@echo "📦 Creating distribution package..."
	@mkdir -p dist
	@cp target/release/hddsgen dist/
	@cp README.md dist/
	@cp -r examples dist/
	@tar -czf hddsgen-$(shell cargo pkgid | cut -d'#' -f2).tar.gz dist/
	@echo "✅ Distribution created: hddsgen-$(shell cargo pkgid | cut -d'#' -f2).tar.gz"

# Create source distribution (without binaries)
dist-src:
	@echo "📦 Creating source distribution..."
	@mkdir -p dist-src
	@cp -r src dist-src/
	@cp -r examples dist-src/
	@cp -r tests dist-src/
	@cp Cargo.toml dist-src/
	@cp Cargo.lock dist-src/
	@cp Makefile dist-src/
	@cp README.md dist-src/
	@cp test_all_idl.sh dist-src/
	@tar -czf hddsgen-$(shell cargo pkgid | cut -d'#' -f2)-src.tar.gz dist-src/
	@rm -rf dist-src/
	@echo "✅ Source distribution created: hddsgen-$(shell cargo pkgid | cut -d'#' -f2)-src.tar.gz"

# Install the binary to ~/.cargo/bin (or system-wide with sudo)
install: release
	@echo "📥 Installing hddsgen binary..."
	cargo install --path .
	@echo "✅ Installed to ~/.cargo/bin/hddsgen"

# Uninstall the binary
uninstall:
	@echo "🗑️  Uninstalling hddsgen..."
	cargo uninstall hddsgen
	@echo "✅ Uninstalled"

# Run with example file
run: build
	@echo "▶️  Running parser on HelloWorld.idl..."
	cargo run --bin hddsgen -- gen cpp examples/HelloWorld.idl

# Run on advanced example
run-advanced: build
	@echo "▶️  Running parser on advanced.idl..."
	cargo run --bin hddsgen -- gen cpp examples/advanced.idl

# Format code with rustfmt
fmt:
	@echo "🎨 Formatting code..."
	cargo fmt

# Check code formatting
fmt-check:
	@echo "🔍 Checking code formatting..."
	cargo fmt -- --check

# Run clippy linter
clippy:
	@echo "🔍 Running clippy linter..."
	cargo clippy -- -D warnings

# Fix clippy warnings automatically where possible
clippy-fix:
	@echo "🔧 Fixing clippy warnings..."
	cargo clippy --fix

# Generate documentation
doc:
	@echo "📚 Generating documentation..."
	cargo doc --no-deps --open

# Generate documentation without opening browser
doc-quiet:
	@echo "📚 Generating documentation..."
	cargo doc --no-deps

# Check for outdated dependencies
outdated:
	@echo "🔍 Checking for outdated dependencies..."
	@if command -v cargo-outdated > /dev/null; then \
		cargo outdated; \
	else \
		echo "❌ cargo-outdated not installed. Run: cargo install cargo-outdated"; \
	fi

# Update dependencies
update:
	@echo "⬆️  Updating dependencies..."
	cargo update

# Audit dependencies for security vulnerabilities
audit:
	@echo "🔒 Auditing dependencies..."
	@if command -v cargo-audit > /dev/null; then \
		cargo audit; \
	else \
		echo "❌ cargo-audit not installed. Run: cargo install cargo-audit"; \
	fi

# Benchmark (requires criterion benchmarks to be defined)
bench:
	@echo "⚡ Running benchmarks..."
	cargo bench

# Check project for errors without building
check:
	@echo "🔍 Checking project..."
	cargo check

# Run in release mode (optimized)
run-release: release
	@echo "▶️  Running parser (release mode) on HelloWorld.idl..."
	./target/release/hddsgen gen cpp examples/HelloWorld.idl

# Quick test: build + run unit tests + run production tests
quick-test: build test test-all
	@echo "✅ All tests passed!"

# Full validation: format check + clippy + tests + production tests
validate: fmt-check clippy test test-all
	@echo "✅ Full validation passed!"

# CI validation: wraps core checks used in CI
validate-ci:
	@set -e; \
	echo "[fmt-check]"; \
	make fmt-check; \
	echo "[clippy]"; \
	make clippy; \
	echo "[tests]"; \
	cargo test --all --locked; \
	echo "[examples check]"; \
	for f in examples/*.idl; do \
	  echo "Check $$f"; \
	  cargo run --quiet --bin hddsgen -- check "$$f"; \
	done; \
	echo "[canonical fmt strict]"; \
	for f in examples/canonical/*.idl; do \
	  echo "Fmt-check (strict) $$f"; \
	  tmp=$$(mktemp); \
	  cargo run --quiet --bin hddsgen -- fmt "$$f" > "$$tmp"; \
	  norm1=$$(mktemp); norm2=$$(mktemp); \
	  awk '{ if ($$0 ~ /[^[:space:]]/) last=NR; lines[NR]=$$0 } END { for (i=1;i<=last;i++) print lines[i] }' "$$f" > "$$norm1"; \
	  awk '{ if ($$0 ~ /[^[:space:]]/) last=NR; lines[NR]=$$0 } END { for (i=1;i<=last;i++) print lines[i] }' "$$tmp" > "$$norm2"; \
	  diff -u "$$norm1" "$$norm2"; \
	  rm -f "$$tmp" "$$norm1" "$$norm2"; \
	done; \
	echo "[invalid suite expect failures]"; \
	if ls examples/invalid/*.idl >/dev/null 2>&1; then \
	  for f in examples/invalid/*.idl; do \
	    echo "Invalid $$f (expect fail)"; \
	    if cargo run --quiet --bin hddsgen -- check "$$f"; then \
	      echo "Expected failure but passed: $$f"; exit 1; \
	    fi; \
	  done; \
	fi; \
		echo "[codegen smoke C/C++/Rust/Python]"; \
		tmp=$$(mktemp); \
		cargo run --quiet --bin hddsgen -- gen cpp examples/canonical/Advanced.idl -o "$$tmp.hpp"; test -s "$$tmp.hpp"; \
		cargo run --quiet --bin hddsgen -- gen rust examples/canonical/Advanced.idl -o "$$tmp.rs"; test -s "$$tmp.rs"; \
		cargo run --quiet --bin hddsgen -- gen c   examples/HelloWorld.idl -o "$$tmp.h"; test -s "$$tmp.h"; \
		printf '%s\n' '#include <stdint.h>' '#include "gen.h"' 'int main(void){return 0;}' > main.c; \
		cp "$$tmp.h" gen.h; \
		if command -v clang >/dev/null; then clang -std=c11 -Wall -Wextra -c main.c; else gcc -std=c11 -Wall -Wextra -c main.c; fi; \
		printf '%s\n' '#include <cstdint>' '#include "gen.hpp"' 'int main(){return 0;}' > main.cpp; \
		cp "$$tmp.hpp" gen.hpp; \
		if command -v clang++ >/dev/null; then clang++ -std=c++17 -Wall -Wextra -c main.cpp; else g++ -std=c++17 -Wall -Wextra -c main.cpp; fi; \
		rm -f "$$tmp.hpp" "$$tmp.rs" "$$tmp.h" gen.h gen.hpp main.c main.cpp main.o; \
	cargo run --quiet --bin hddsgen -- gen python examples/canonical/Advanced.idl -o "$$tmp.py"; python3 -m py_compile "$$tmp.py"; rm -f "$$tmp.py" "$$tmp.pyc" 2>/dev/null || true; \
	echo "[include fixtures]"; \
	cargo run --quiet --bin hddsgen -- check examples/include/quoted/main.idl; \
	cargo run --quiet --bin hddsgen -- check -I examples/include/angled/inc examples/include/angled/main.idl; \
	cargo run --quiet --bin hddsgen -- check examples/include/cycle/a.idl; \
	echo "[preprocessor macros]"; \
	for f in examples/macros/*.idl; do cargo run --quiet --bin hddsgen -- check "$$f"; done; \
	echo "[interfaces feature]"; \
	cargo build --quiet --features interfaces; \
	tmpi=$$(mktemp); cargo run --quiet --features interfaces --bin hddsgen -- fmt examples/interfaces/Simple.idl > "$$tmpi"; test -s "$$tmpi"; rm -f "$$tmpi"; \
	cargo test --features interfaces --all --locked; \
	echo "✅ validate-ci completed"

# Development setup: install useful tools
dev-setup:
	@echo "🔧 Setting up development environment..."
	@echo "Installing useful cargo tools..."
	@cargo install cargo-watch 2>/dev/null || true
	@cargo install cargo-tarpaulin 2>/dev/null || true
	@cargo install cargo-outdated 2>/dev/null || true
	@cargo install cargo-audit 2>/dev/null || true
	@echo "✅ Development setup complete"

# Watch for changes and rebuild automatically (requires cargo-watch)
watch:
	@echo "👀 Watching for changes..."
	@if command -v cargo-watch > /dev/null; then \
		cargo watch -x build -x test; \
	else \
		echo "❌ cargo-watch not installed. Run: cargo install cargo-watch"; \
	fi

# Watch and run tests on changes
watch-test:
	@echo "👀 Watching for changes (running tests)..."
	@if command -v cargo-watch > /dev/null; then \
		cargo watch -x test; \
	else \
		echo "❌ cargo-watch not installed. Run: cargo install cargo-watch"; \
	fi

# Generate and view flamegraph (requires flamegraph tools)
flamegraph:
	@echo "🔥 Generating flamegraph..."
	@if command -v cargo-flamegraph > /dev/null; then \
		cargo flamegraph --bin hddsgen -- examples/HelloWorld.idl --output-cpp; \
	else \
		echo "❌ cargo-flamegraph not installed. Run: cargo install flamegraph"; \
	fi

# Show project statistics (lines of code, etc.)
stats:
	@echo "📊 Project Statistics"
	@echo "====================="
	@echo "Lines of Rust code:"
	@find src -name "*.rs" -exec wc -l {} + | tail -1
	@echo ""
	@echo "Number of test files:"
	@find tests -name "*.rs" 2>/dev/null | wc -l
	@echo ""
	@echo "Number of example IDL files:"
	@find examples -name "*.idl" 2>/dev/null | wc -l
	@echo ""
	@echo "Dependencies:"
	@cargo tree --depth 1 | grep -v "├──" | grep -v "└──" | wc -l

# Show version
version:
	@echo "IDL Parser version: $(shell cargo pkgid | cut -d'#' -f2)"

# Help target - shows all available commands
help:
	@echo "IDL Parser - Available Make Targets"
	@echo "===================================="
	@echo ""
	@echo "Building:"
	@echo "  make build          - Build project (debug mode)"
	@echo "  make release        - Build project (release mode, optimized)"
	@echo "  make check          - Check for errors without building"
	@echo ""
	@echo "Testing:"
	@echo "  make test           - Run unit tests"
	@echo "  make test-all       - Run production test suite (50 files)"
	@echo "  make test-prod      - Alias for test-all"
	@echo "  make test-coverage  - Run tests with coverage report"
	@echo "  make quick-test     - Build + unit tests + production tests"
	@echo "  make validate       - Full validation (fmt + clippy + tests)"
	@echo ""
	@echo "Running:"
	@echo "  make run            - Run parser on HelloWorld.idl"
	@echo "  make run-advanced   - Run parser on advanced.idl"
	@echo "  make run-release    - Run parser (release mode)"
	@echo ""
	@echo "Code Quality:"
	@echo "  make fmt            - Format code with rustfmt"
	@echo "  make fmt-check      - Check code formatting"
	@echo "  make clippy         - Run clippy linter"
	@echo "  make clippy-fix     - Auto-fix clippy warnings"
	@echo ""
	@echo "Documentation:"
	@echo "  make doc            - Generate and open documentation"
	@echo "  make doc-quiet      - Generate documentation (no browser)"
	@echo ""
	@echo "Distribution:"
	@echo "  make dist           - Create distribution package (.tar.gz)"
	@echo "  make dist-src       - Create source distribution"
	@echo "  make clean          - Remove build artifacts"
	@echo ""
	@echo "Installation:"
	@echo "  make install        - Install binary to ~/.cargo/bin"
	@echo "  make uninstall      - Remove installed binary"
	@echo ""
	@echo "Dependencies:"
	@echo "  make update         - Update dependencies"
	@echo "  make outdated       - Check for outdated dependencies"
	@echo "  make audit          - Audit dependencies for vulnerabilities"
	@echo ""
	@echo "Development:"
	@echo "  make dev-setup      - Install development tools"
	@echo "  make watch          - Watch for changes and rebuild"
	@echo "  make watch-test     - Watch for changes and run tests"
	@echo "  make bench          - Run benchmarks"
	@echo "  make flamegraph     - Generate performance flamegraph"
	@echo ""
	@echo "Information:"
	@echo "  make stats          - Show project statistics"
	@echo "  make version        - Show version"
	@echo "  make help           - Show this help message"
	@echo ""
	@echo "Common workflows:"
	@echo "  make                - Same as 'make build'"
	@echo "  make && make test-all - Build and test everything"
	@echo "  make validate       - Full pre-commit validation"
	@echo "  make dist           - Create release package"
