#!/usr/bin/env bash
set -euo pipefail

# Delete build files not touched in the last 30 days.
cargo sweep --time 30

# Clean under $CARGO_HOME (~/.cargo).
cargo cache --autoclean

# Optional: recompress git repos (slower, but can reclaim space)
# cargo cache --gc