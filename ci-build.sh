#!/bin/bash
# Runs all quality checks and builds

set -e  # Exit on first error
set -u  # Exit on undefined variables

echo "🧹 Checking code formatting..."
cargo fmt --all --check

echo "📎 Running Clippy linter..."
cargo clippy --all-targets --all-features -- -D warnings

echo "🧪 Running tests..."
cargo test

echo "📊 Generating test coverage report..."
if command -v cargo-llvm-cov &> /dev/null; then
    cargo llvm-cov --html --output-dir coverage
else
    echo "⚠️  cargo-llvm-cov not installed, run: cargo install cargo-llvm-cov"
fi

echo "�🔍 Running security audit..."
if command -v cargo-audit &> /dev/null; then
    cargo audit
else
    echo "⚠️  cargo-audit not installed, run: cargo install cargo-audit"
fi

echo "🔧 Building debug version..."
cargo build -j 4

echo "🚀 Building release version..."
cargo build -j 4 --release

echo "📚 Generating documentation..."
cargo doc --document-private-items

echo "📊 Checking for outdated dependencies..."
if command -v cargo-outdated &> /dev/null; then
    cargo outdated
else
    echo "⚠️  cargo-outdated not installed, run: cargo install cargo-outdated"
fi

echo "✅ All checks passed! Ready for deployment."
