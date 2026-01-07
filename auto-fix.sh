#!/bin/bash

# Development helper script - Auto-fixes common code issues.
set -e

echo "🔧 Auto-formatting code (safe - formatting only)..."
cargo fmt --all

echo "🔨 Auto-fixing Clippy warnings (mostly safe - review changes!)..."
cargo clippy --fix --allow-dirty --allow-staged --all-targets --all-features

