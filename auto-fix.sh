#!/usr/bin/env bash
# Development helper script - Auto-fixes common code issues.
set -euo pipefail

echo "🔧 Auto-formatting code (safe - formatting only)..."
cargo fmt --all

echo "🔨 Auto-fixing Clippy lints (review changes!)..."
# Use clippy explicitly as the driver; `cargo clippy --fix` works too,
# but this is the recommended/consistent pattern for fixes.
cargo fix --clippy --allow-dirty --allow-staged --all-targets --all-features

echo "📎 Re-running Clippy (must be clean)..."
cargo clippy --all-targets --all-features -- -D warnings

# Optional: dependency cleanup (can remove deps from Cargo.toml)
if command -v cargo-machete >/dev/null 2>&1; then
  echo "🧹 Removing unused dependencies (review Cargo.toml changes!)..."
  cargo machete --fix || true
fi

echo "✅ Autofix done!"