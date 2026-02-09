#!/usr/bin/env bash
set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Defaults
BIN="steady_state"
DO_BENCH=false
DO_PERF=false
DO_FLAMEGRAPH=false

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Performance profiling script for rainybook.

Options:
  -b, --bench       Run cargo bench
  -p, --perf [BIN]  Run perf record on binary (default: steady_state)
  -f, --flamegraph  Generate flamegraph from perf.data
  -h, --help        Show this help message

Examples:
  $(basename "$0") --bench                    # Run benchmarks only
  $(basename "$0") --perf                     # Profile steady_state
  $(basename "$0") --perf my_binary           # Profile specific binary
  $(basename "$0") --perf --flamegraph        # Profile + generate flamegraph
  $(basename "$0") -p my_binary -f            # Short form
  $(basename "$0") --flamegraph               # Generate from existing perf.data
  $(basename "$0") -b -p -f                   # Full pipeline

Execution order: bench -> perf -> flamegraph
EOF
}

info() {
    echo -e "${BLUE}==>${NC} $1"
}

success() {
    echo -e "${GREEN}==>${NC} $1"
}

warn() {
    echo -e "${YELLOW}Warning:${NC} $1"
}

error() {
    echo -e "${RED}Error:${NC} $1" >&2
}

check_tool() {
    if ! command -v "$1" &> /dev/null; then
        error "$1 is not installed or not in PATH"
        exit 1
    fi
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        -b|--bench)
            DO_BENCH=true
            shift
            ;;
        -p|--perf)
            DO_PERF=true
            shift
            # Check if next arg is a binary name (not a flag)
            if [[ $# -gt 0 && ! "$1" =~ ^- ]]; then
                BIN="$1"
                shift
            fi
            ;;
        -f|--flamegraph)
            DO_FLAMEGRAPH=true
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            error "Unknown option: $1"
            usage
            exit 1
            ;;
    esac
done

# Check if any action requested
if [[ "$DO_BENCH" == false && "$DO_PERF" == false && "$DO_FLAMEGRAPH" == false ]]; then
    error "No action specified"
    usage
    exit 1
fi

# Execute in order: bench -> perf -> flamegraph

# 1. Run benchmarks
if [[ "$DO_BENCH" == true ]]; then
    info "Running cargo bench..."
    cargo bench
    success "Benchmarks complete"
fi

# 2. Run perf record
if [[ "$DO_PERF" == true ]]; then
    check_tool perf

    info "Building $BIN with perf profile..."
    cargo build --profile perf --bin "$BIN"

    BINARY_PATH="../target/perf/$BIN"
    if [[ ! -f "$BINARY_PATH" ]]; then
        error "Binary not found: $BINARY_PATH"
        exit 1
    fi

    info "Running perf record on $BIN..."
    perf record -e cpu-clock:u -F 199 -g --call-graph fp "$BINARY_PATH"
    success "perf record complete -> perf.data"
fi

# 3. Generate flamegraph
if [[ "$DO_FLAMEGRAPH" == true ]]; then
    check_tool flamegraph

    if [[ ! -f "perf.data" ]]; then
        error "perf.data not found. Run with --perf first or ensure perf.data exists."
        exit 1
    fi

    info "Generating flamegraph from perf.data..."
    flamegraph --perfdata perf.data -o flamegraph.svg
    success "Flamegraph generated -> flamegraph.svg"
fi

success "Done!"
