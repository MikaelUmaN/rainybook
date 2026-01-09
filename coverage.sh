#!/usr/bin/env bash
# coverage.sh
# Local coverage report (matches CI as closely as possible).
set -euo pipefail

rm -rf coverage

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
  echo "⚠️  cargo-llvm-cov not installed"
  exit 1
fi

echo "📊 Generating coverage report..."
if command -v cargo-nextest >/dev/null 2>&1; then
  cargo llvm-cov nextest --all-features --html --output-dir coverage
else
  cargo llvm-cov --all-features --html --output-dir coverage
fi

echo "✅ Coverage written to ./coverage/index.html"
