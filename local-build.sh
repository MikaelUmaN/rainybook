#!/usr/bin/env bash
# Local lightweight build script.
# Use this during development for rapid iteration
set -euo pipefail

echo "🔍 Quick check..."
cargo check

echo "🎨 Format check..."
cargo fmt --all -- --check

echo "📎 Clippy..."
cargo clippy --all-targets --all-features -- -D warnings

echo "🧪 Running tests..."
if command -v cargo-nextest >/dev/null 2>&1; then
  cargo nextest run --no-fail-fast
else
  cargo test --quiet
fi

echo "✅ Quick checks passed!"