#!/usr/bin/env bash
# outdated.sh
# Check for outdated dependencies (informational only, not part of CI).
set -euo pipefail

echo "📦 Checking for outdated dependencies..."
if command -v cargo-outdated >/dev/null 2>&1; then
  cargo outdated --workspace --depth 1
else
  echo "⚠️  cargo-outdated not installed"
  echo "Install with: cargo install cargo-outdated"
  exit 1
fi
