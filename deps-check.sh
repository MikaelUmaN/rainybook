#!/usr/bin/env bash
# deps-check.sh
# Dependency hygiene + supply-chain checks.
set -euo pipefail

echo "🔍 Checking for unused dependencies..."
if command -v cargo-machete >/dev/null 2>&1; then
  cargo machete
else
  echo "⚠️  cargo-machete not installed"
fi

echo "🛡️  Checking dependency policy (licenses/bans/advisories/sources)..."
if command -v cargo-deny >/dev/null 2>&1; then
  cargo deny check advisories bans licenses sources
else
  echo "⚠️  cargo-deny not installed"
fi

echo "🔐 Running security advisories audit..."
if command -v cargo-audit >/dev/null 2>&1; then
  cargo audit
else
  echo "⚠️  cargo-audit not installed"
fi

echo "📦 Checking for outdated dependencies..."
if command -v cargo-outdated >/dev/null 2>&1; then
  cargo outdated --workspace
else
  echo "⚠️  cargo-outdated not installed"
fi

echo "✅ Dependency checks complete."
