#!/usr/bin/env bash
set -euo pipefail

# cargo-sweep expects a project path, not a target path.
# Set CARGO_TARGET_DIR so it finds the correct target directory.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export CARGO_TARGET_DIR="${SCRIPT_DIR}/../target"

# Delete build files not touched in the last 30 days.
cargo sweep --time 30

# Clean under $CARGO_HOME (~/.cargo).
cargo cache --autoclean

# Optional: recompress git repos (slower, but can reclaim space)
# cargo cache --gc