# rainybook

High-performance order book library written in pure Rust. Provides Market-By-Order (MBO) and Market-By-Price (MBP) implementations for processing and maintaining order book state from market data feeds.

## Development

### Git worktrees
This repository is intended to be worked on using `git worktree`. Either clone `--bare` as `rainybook.git` or develop only inside `worktrees/`, never in the root clone directly.

The `.cargo/config.toml` uses `target-dir = "../target"`, so all worktrees share a single build cache at `worktrees/target/`. This keeps the build artifacts inside the repo structure.

```
rainybook/                  # root clone (don't work here)
  worktrees/
    target/                 # shared build cache
    main/                   # main branch worktree
    feature-X/
    feature-Y/
```

#### Initial setup

```bash
git clone git@github.com:MikaelUmaN/rainybook.git
cd rainybook
mkdir worktrees
git worktree add worktrees/main main
```

#### Useful commands

```bash
# List all worktrees
git worktree list

# Prune stale worktree metadata
git worktree prune
```

#### Feature development

```bash
git worktree add worktrees/feature-xyz -b feature-xyz origin/main
# Work...
# Finish feature
git push -u origin feature-xyz
git worktree remove worktrees/feature-xyz
```

#### Review/Contribute to existing branch

```bash
git fetch --all --prune
git worktree add worktrees/feature-abc origin/feature-abc
# Review or contribute
# Push changes if you made any
git push
git worktree remove worktrees/feature-abc
```

## Performance Profiling

RainyBook includes a `steady_state` binary for CPU profiling with Linux perf and flamegraph generation.

### Quick Start

```bash
# Build the profiling binary
cargo build --release --bin steady_state

# Run steady-state simulation
../target/release/steady_state --operations 10000000

# Profile with perf and generate flamegraph
cargo flamegraph --bin steady_state -- --operations 10000000
```

### Steady-State Binary

The `steady_state` binary maintains a realistic order book with ~10 levels of depth by dynamically adjusting operation probabilities:

- **Deterministic**: Seeded RNG for reproducible results
- **Configurable**: Adjust operation mix, depth, and seed
- **Fast**: 2M+ operations per second
- **Instrumented**: Optional detailed timing statistics

**Example:**
```bash
../target/release/steady_state \
    --operations 10000000 \
    --target-depth 10 \
    --seed 42 \
    --prob-add 0.45 \
    --prob-cancel 0.35 \
    --prob-fill 0.15 \
    --prob-modify 0.05
```

### Performance Analysis

For detailed performance analysis, use:

**Criterion Benchmarks** (precise per-operation timing):
```bash
cargo bench
```

**Tracing Spans** (optional debugging):
```bash
# Add spans to operations in code (see steady_state.rs for examples)
RUST_LOG=debug cargo run --bin steady_state -- --operations 10000
```

### CPU Profiling with Perf

```bash
# Record CPU samples
perf record -F 997 --call-graph dwarf \
    ../target/release/steady_state --operations 10000000

# View report
perf report

# Generate flamegraph
cargo flamegraph --bin steady_state -- --operations 10000000
```

**See [docs/perf_guide.md](docs/perf_guide.md) for comprehensive profiling guide**, including:
- Building with debug symbols
- Advanced perf commands
- Cache and branch prediction analysis
- Interpreting flamegraphs
- Performance optimization workflow

### Benchmarks

Run Criterion benchmarks:

```bash
# All benchmarks
cargo bench

# Specific benchmark
cargo bench orderbook/add_order
```

Benchmarks cover:
- Add order (empty and populated book)
- Remove order
- Modify order
- Fill order
- Best bid/ask queries
- Top N levels
