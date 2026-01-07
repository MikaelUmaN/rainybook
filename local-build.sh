#!/bin/bash
# Local lightweight build script.
# Use this during development for rapid iteration
set -e

echo "🔍 Quick check..."
cargo check

echo "📎 Clippy (warnings only)..."
cargo clippy --all-targets --all-features

echo "🧪 Running tests..."
cargo test --quiet

echo "✅ Quick checks passed!"