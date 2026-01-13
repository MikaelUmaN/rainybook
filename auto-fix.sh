#!/usr/bin/env bash
# Development helper script - Auto-fixes common code issues.
set -euo pipefail

echo "🔧 Auto-formatting code (safe - formatting only)..."
cargo fmt --all

echo "🔨 Auto-fixing Clippy lints (review changes!)..."
cargo clippy --fix --allow-dirty --allow-staged --all-targets --all-features

echo "📎 Re-running Clippy (must be clean)..."
cargo clippy --all-targets --all-features -- -D warnings

# Optional: dependency cleanup (can remove deps from Cargo.toml)
if command -v cargo-machete >/dev/null 2>&1; then
  echo "🧹 Removing unused dependencies (review Cargo.toml changes!)..."
  cargo machete --fix || true
fi

# Fix vulnerable dependencies
if command -v cargo-audit >/dev/null 2>&1; then
  if cargo audit fix --help >/dev/null 2>&1; then
    echo "🔐 Fixing vulnerable dependencies (review Cargo.toml changes!)..."
    cargo audit fix || true
  else
    echo "ℹ️  cargo-audit installed without 'fix' feature - skipping vulnerability fixes"
    echo "   Install with: cargo install cargo-audit --locked --features=fix"
  fi
else
  echo "ℹ️  cargo-audit not installed - skipping vulnerability fixes"
fi

echo "✅ Autofix done!"