#!/usr/bin/env bash
# Runs all quality checks and builds (CI)
set -euo pipefail

# cargo-llvm-cov may not reliably respect .cargo/config.toml target-dir with relative paths.
# Export absolute path to ensure consistent behavior across all environments.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export CARGO_TARGET_DIR="${SCRIPT_DIR}/../target"

JOBS="${CARGO_BUILD_JOBS:-4}"

echo "🧹 Checking code formatting..."
cargo fmt --all --check

echo "📎 Running Clippy linter..."
cargo clippy --all-targets --all-features -- -D warnings

echo "🧪 Running tests (nextest if available)..."
if command -v cargo-nextest >/dev/null 2>&1; then
  cargo nextest run --no-fail-fast
else
  cargo test
fi

echo "📊 Generating test coverage report..."
rm -rf coverage
if command -v cargo-llvm-cov >/dev/null 2>&1; then
  if command -v cargo-nextest >/dev/null 2>&1; then
    cargo llvm-cov nextest --all-features --html --output-dir coverage
  else
    cargo llvm-cov --all-features --html --output-dir coverage
  fi
else
  echo "⚠️  cargo-llvm-cov not installed"
fi

echo "🔍 Dependency policy & advisories..."
if command -v cargo-deny >/dev/null 2>&1; then
  cargo deny check advisories bans licenses sources
else
  echo "⚠️  cargo-deny not installed"
fi

echo "🔍 Running security audit (advisories only)..."
if command -v cargo-audit >/dev/null 2>&1; then
  cargo audit
else
  echo "⚠️  cargo-audit not installed"
fi

echo "🧼 Checking for unused dependencies..."
if command -v cargo-machete >/dev/null 2>&1; then
  cargo machete
else
  echo "⚠️  cargo-machete not installed"
fi

echo "📦 Building debug version..."
cargo build -j "$JOBS"

echo "🚀 Building release version..."
cargo build -j "$JOBS" --release

echo "📚 Generating documentation..."
# Stable-friendly default:
cargo doc --no-deps

# If you *really* want private items, do it on nightly like:
# cargo +nightly doc -Z unstable-options --document-private-items --no-deps

echo "📊 Checking for outdated dependencies..."
if command -v cargo-outdated >/dev/null 2>&1; then
  cargo outdated --workspace
else
  echo "⚠️  cargo-outdated not installed"
fi

echo "✅ All checks passed! Ready for deployment."
