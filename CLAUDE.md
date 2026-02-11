# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

RainyBook is a high-performance order book library written in pure Rust. It provides Market-By-Order (MBO) and Market-By-Price (MBP) implementations for processing and maintaining order book state from market data feeds, with a focus on performance and functional programming patterns.

## Build and Test Commands

```bash
# Build the project
cargo build

# Build with release optimizations
cargo build --release

# Run tests
cargo test

# Run specific test
cargo test test_name

# Run benchmarks
cargo bench

# Run specific benchmark
cargo bench orderbook/add_order

# Run with logging
RUST_LOG=debug cargo test

# Build with optional features
cargo build --features polars_perf
cargo build --features polars_all_dtypes

# Performance profiling
cargo build --release --bin steady_state
../target/release/steady_state --duration 30

# Flamegraph generation
cargo flamegraph --bin steady_state -- --duration 30
```

## Architecture

### Core Components

The codebase is organized into three main modules under `src/orderbook/`:

1. **book.rs** - Core order book implementation
   - `OrderBook`: Market-By-Order orderbook tracking individual orders with BTreeMap-based price levels
   - `OrderLevel`: Price level tracking individual orders with HashMap for O(1) order lookup
   - `Order`: Individual order struct with id, side, price, size
   - Supports add, cancel, modify, and fill operations
   - Maintains `order_index` (HashMap) for fast order_id -> price lookup

2. **mbo.rs** - Market-By-Order message processing
   - `MboProcessor`: Processes incoming MBO messages and maintains OrderBook state
   - `MarketByOrderMessage`: Standardized MBO message format
   - `Action` enum: Add, Cancel, Modify, Fill, Clear, Trade
   - `into_mbo_messages()`: Converts Polars DataFrame to MBO messages
   - Integrates with Databento's `dbn` crate for market data ingestion

3. **mbp.rs** - Market-By-Price aggregation view
   - `MarketByPrice`: Aggregated view of order book by price level
   - `OrderLevelSummary`: Contains price, total_quantity, and order_count
   - Conversion from OrderBook to MarketByPrice
   - DataFrame export via `to_dataframe()` for Polars integration

### Key Design Patterns

- **Idempotent operations**: add_order and remove_order are idempotent with warning logs
- **Integer prices**: All prices are i64 (cents, ticks, etc.) for precision
- **BTreeMap for price levels**: Enables efficient best_bid/best_ask via next_back()/next()
- **HashMap for order lookup**: O(1) order_id lookup via order_index
- **Functional iterator chains**: Heavily uses map, filter, filter_map instead of loops
- **Error propagation**: Uses thiserror for custom error types, ? operator for propagation

### Dependencies

- **polars**: DataFrame processing and data export
- **dbn**: Databento market data format support
- **thiserror**: Error type definitions
- **num_enum**: Enum to/from integer conversions for Action and Side
- **criterion**: Benchmarking framework
- **tracing**: Structured logging
- **rand/rand_chacha/rand_distr**: Random number generation for testing and profiling

### Supporting Modules

4. **generators.rs** - Order generation for testing and profiling
   - `OrderGenerator`: Stateful generator with configurable price/quantity distributions
   - Maintains max_bid/min_ask to prevent crossed books
   - Seeded RNG for deterministic generation
   - Used by benchmarks and steady_state binary

### Binaries

1. **src/main.rs** - CLI tool for processing market data files
   - Supports .dbn, .parquet, and MBO message formats
   - File format auto-detection
   - Verbose logging option

2. **src/bin/steady_state.rs** - Performance profiling binary
   - Simulates steady-state order book with ~10 levels depth
   - Deterministic execution with seeded RNG
   - Dynamic probability adjustment to maintain depth
   - Configurable operation mix (add/cancel/fill/modify)
   - 2M+ operations per second throughput

## Performance Profiling

### Steady-State Binary

The `steady_state` binary is designed for CPU profiling with Linux perf and flamegraph generation:

**Basic Usage:**
```bash
# Run simulation for 30 seconds
../target/release/steady_state --duration 30 --target-depth 10

# With different operation mix
../target/release/steady_state \
    --duration 60 \
    --prob-add 0.40 \
    --prob-cancel 0.30 \
    --prob-fill 0.25 \
    --prob-modify 0.05
```

### Linux Perf Profiling

**CPU Profiling:**
```bash
# Record samples with call graphs
perf record -F 997 --call-graph dwarf \
    ../target/release/steady_state --duration 30

# View report
perf report --stdio

# Interactive TUI
perf report
```

**Flamegraph Generation:**
```bash
# One command (easiest)
cargo flamegraph --bin steady_state -- --duration 30

# Manual with FlameGraph scripts
perf record -F 997 --call-graph dwarf -g ../target/release/steady_state --duration 30
perf script | stackcollapse-perf.pl | flamegraph.pl > flamegraph.svg
```

**Cache and Branch Analysis:**
```bash
# Cache performance
perf stat -e cache-references,cache-misses \
    ../target/release/steady_state --duration 10

# Branch prediction
perf stat -e branches,branch-misses \
    ../target/release/steady_state --duration 10

# Detailed stats
perf stat -d ../target/release/steady_state --duration 10
```

### Profiling Workflow

1. **Baseline**: Build with release or perf profile and establish baseline metrics
2. **Profile**: Use perf to identify hotspots
3. **Visualize**: Generate flamegraphs to see call hierarchy
4. **Analyze**: Check cache misses, branch predictions, IPC
5. **Optimize**: Make targeted improvements
6. **Validate**: Compare before/after with perf stat and criterion benchmarks

**See [docs/profiling.md](docs/profiling.md) for profiling documentation.**

### Performance Monitoring

- **Criterion benchmarks** (`cargo bench`) - Precise per-operation timing with statistical analysis
- **Linux perf/flamegraph** - CPU profiling without overhead (see docs/profiling.md)
- **Tracing spans** - Optional operation-level monitoring for debugging (see steady_state.rs for examples)

### Feature Flags

- `polars_perf`: Enable polars performant mode
- `polars_all_dtypes`: Enable all polars data types

## Coding Standards

For detailed coding standards, style guidelines, and best practices, see [.github/copilot-instructions.md](.github/copilot-instructions.md).

Key highlights:
- **Read-only mode by default** - only make changes when explicitly asked
- **Functional programming** - prefer iterator methods over loops
- **Import at top** - never use inline `::` paths in type signatures
- **Version compatibility** - ensure compatibility with exact versions in Cargo.toml
- **Performance-conscious** - avoid unnecessary allocations and clones
