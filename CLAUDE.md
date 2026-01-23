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

## Coding Standards

For detailed coding standards, style guidelines, and best practices, see [.github/copilot-instructions.md](.github/copilot-instructions.md).

Key highlights:
- **Read-only mode by default** - only make changes when explicitly asked
- **Functional programming** - prefer iterator methods over loops
- **Import at top** - never use inline `::` paths in type signatures
- **Version compatibility** - ensure compatibility with exact versions in Cargo.toml
- **Performance-conscious** - avoid unnecessary allocations and clones
