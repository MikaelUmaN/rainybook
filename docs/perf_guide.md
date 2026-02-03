# Performance Profiling with Linux Perf and Flamegraphs

This guide explains how to use Linux `perf` and flamegraphs to profile the RainyBook order book implementation using the `steady_state` binary.

## Table of Contents

- [Overview](#overview)
- [Prerequisites](#prerequisites)
- [WSL2 Limitations and Workarounds](#wsl2-limitations-and-workarounds)
- [Building for Profiling](#building-for-profiling)
- [Running the Steady-State Binary](#running-the-steady-state-binary)
- [Optional: Operation-Level Monitoring with Tracing](#optional-operation-level-monitoring-with-tracing)
- [CPU Profiling with Perf](#cpu-profiling-with-perf)
- [Generating Flamegraphs](#generating-flamegraphs)
- [Interpreting Results](#interpreting-results)
- [Advanced Profiling](#advanced-profiling)
- [Troubleshooting](#troubleshooting)
- [Performance Optimization Workflow](#performance-optimization-workflow)

## Overview

The `steady_state` binary simulates realistic order book activity by maintaining approximately 10 levels of bid/ask depth through dynamic probability adjustment. This creates a stable, representative workload perfect for performance profiling.

**Key Features:**
- Deterministic execution with seeded RNG
- Configurable operation mix (add/cancel/fill/modify)
- Dynamic depth maintenance
- 2M+ operations per second throughput

## Prerequisites

### Required Tools

```bash
# Linux perf tools
sudo apt-get install linux-tools-common linux-tools-generic linux-tools-`uname -r`

# For flamegraph generation (optional but recommended)
cargo install flamegraph
```

### System Configuration

Enable user-space profiling (required for non-root users):

```bash
# Temporary (until reboot)
sudo sysctl kernel.perf_event_paranoid=1

# Permanent
echo 'kernel.perf_event_paranoid=1' | sudo tee -a /etc/sysctl.conf
sudo sysctl -p
```

## WSL2 Limitations and Workarounds

**⚠️ IMPORTANT: If you're running on WSL2, read this section carefully before proceeding.**

WSL2 has significant limitations for performance profiling due to its virtualized nature. As of January 2026, hardware PMU support remains an [open issue with no official resolution](https://github.com/microsoft/WSL/issues/8480).

### The Root Cause

WSL2 runs inside Hyper-V, which *can* expose hardware performance counters (`arch_perfmon` CPU feature) but **HCS (Host Compute Service) and WSL2 do not enable this**. This affects Intel Alderlake and newer CPUs particularly severely ([Issue #12836](https://github.com/microsoft/WSL/issues/12836)).

Some users report partial success with specific kernel versions (e.g., kernel 6.8.0-62-generic on WSL 2.5.9), but results are inconsistent across systems.

### What Does NOT Work

**Hardware Performance Counters** - The critical limitation:

- ❌ CPU cycles / instructions
- ❌ Cache references/misses (L1, L2, LLC)
- ❌ Branch predictions/mispredictions
- ❌ IPC (Instructions Per Cycle)
- ❌ TLB loads/misses
- ❌ Memory loads/stores

**Why this matters:** You cannot use `perf stat` to measure cache efficiency, branch prediction accuracy, or IPC. Most of the [Advanced Profiling](#advanced-profiling) section won't work.

### What DOES Work

**Software events** work reliably and are sufficient for hotspot identification:

- ✅ **cpu-clock** - Software-based CPU time sampling
- ✅ **task-clock** - Time spent on CPU
- ✅ **Call-graph sampling** - Function call hierarchy via DWARF
- ✅ **Flamegraph generation** - Visual hot path analysis
- ✅ **Context switches** - OS scheduling events
- ✅ **Page faults** - Memory allocation patterns

**Bottom line:** You can identify which functions consume CPU time and generate useful flamegraphs, but cannot measure hardware-level efficiency.

### Installation on WSL2

Standard `linux-tools` packages won't work because WSL2 uses a custom Microsoft kernel. You must compile `perf` from source:

```bash
# Clone the WSL2 kernel repository (match your kernel version if possible)
git clone https://github.com/microsoft/WSL2-Linux-Kernel --depth 1
cd WSL2-Linux-Kernel/tools/perf

# Install build dependencies (comprehensive list for proper stack traces)
sudo apt-get update
sudo apt-get install flex bison libelf-dev libunwind-dev \
    libaudit-dev libslang2-dev libperl-dev python3-dev \
    binutils-dev liblzma-dev libzstd-dev libcap-dev \
    libdwarf-dev libdw-dev systemtap-sdt-dev libssl-dev \
    libbabeltrace-dev libiberty-dev

# Build perf
make -j$(nproc)

# Install to system path
sudo cp perf /usr/local/bin/

# Verify installation
perf --version
```

### System Configuration for WSL2

```bash
# Required: Allow user-space profiling
echo -1 | sudo tee /proc/sys/kernel/perf_event_paranoid

# Optional: Enable kernel symbol resolution
echo 0 | sudo tee /proc/sys/kernel/kptr_restrict
```

### Flamegraph Generation on WSL2 - What Actually Works

The key insight: **use `cpu-clock` software event** instead of the default `cycles` hardware event.

**Method 1: cargo flamegraph with explicit event (Recommended)**

```bash
# Install flamegraph
cargo install flamegraph

# CRITICAL: Use -e cpu-clock to avoid hardware counter errors
CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
    -e cpu-clock \
    --bin steady_state \
    -- --operations 10000000
```

Without `-e cpu-clock`, you'll see: `perf_event_open(...) failed with error 22 (Invalid argument)`.

**Method 2: Manual perf + FlameGraph scripts**

```bash
# Record with cpu-clock software event and DWARF call graphs
perf record -e cpu-clock -F 997 --call-graph dwarf,16384 \
    ../target/release/steady_state --operations 10000000

# Generate flamegraph
perf script | stackcollapse-perf.pl | flamegraph.pl > flamegraph.svg

# View in browser
firefox flamegraph.svg
```

**Method 3: Native perf flamegraph (if supported)**

```bash
# Some perf versions support native flamegraph output
perf record -e cpu-clock -F 997 --call-graph dwarf \
    ../target/release/steady_state --operations 10000000
perf script report flamegraph
```

### What You CAN Achieve on WSL2

| Task | Status | How |
|------|--------|-----|
| Identify hot functions | ✅ Works | `cargo flamegraph -e cpu-clock` |
| Find call hierarchy | ✅ Works | Flamegraph + DWARF call graphs |
| Compare before/after | ✅ Works | Multiple flamegraphs |
| Operation-level timing | ✅ Works | `cargo bench` with criterion |
| Cache miss analysis | ❌ Broken | Need native Linux |
| Branch prediction analysis | ❌ Broken | Need native Linux |
| IPC measurement | ❌ Broken | Need native Linux |

### What perf stat Shows on WSL2

```bash
$ perf stat ../target/release/steady_state --operations 1000000

 Performance counter stats for '../target/release/steady_state':

         847.32 msec task-clock                #    0.998 CPUs utilized
             12      context-switches          #   14.162 /sec
              0      cpu-migrations            #    0.000 /sec
            892      page-faults               #    1.053 K/sec
   <not supported>   cycles
   <not supported>   instructions
   <not supported>   branches
   <not supported>   branch-misses
```

Only software events produce values; hardware counters show `<not supported>`.

### WSL2 Profiling Strategy

**For iterative development:**
1. Use `cargo flamegraph -e cpu-clock` to identify hotspots
2. Use `cargo bench` for precise timing comparisons
3. Focus on algorithmic improvements visible in flamegraphs

**For hardware-level optimization:**
1. Develop and iterate on WSL2 with flamegraphs
2. Move to native Linux for cache/branch analysis
3. Validate final optimizations on bare metal

### Recommended Approach for WSL2 Users

1. **Hotspot identification**: `cargo flamegraph -e cpu-clock` - **WORKS**
2. **Operation-level timing**: `cargo bench` with criterion - **WORKS**
3. **Call graph analysis**: `perf record -e cpu-clock --call-graph dwarf` - **WORKS**
4. **Hardware metrics**: Native Linux, dual-boot, or bare metal - **Required**

### Alternative: Dual-Boot or Native Linux

If you need hardware performance counters for serious optimization work:
- Dual-boot your machine with Linux
- Use a native Linux machine
- Use a cloud VM with nested virtualization disabled (AWS metal instances)
- Use a Hyper-V VM (ironically, Hyper-V VMs can access PMU counters that WSL2 cannot)

The virtualization overhead in WSL2 isn't just a measurement limitation - it also affects actual performance characteristics. For production-grade optimization, you'll eventually need bare metal.

### References

- [WSL Issue #8480: State of hardware performance monitoring](https://github.com/microsoft/WSL/issues/8480) - Current status tracker
- [WSL Issue #4678: Support hardware performance counters](https://github.com/microsoft/WSL/issues/4678) - Original feature request (2019)
- [WSL Issue #12836: Enabling hardware counter profiling](https://github.com/microsoft/WSL/issues/12836) - Recent discussion (2025)
- [flamegraph-rs Issue #82: WSL compatibility](https://github.com/flamegraph-rs/flamegraph/issues/82) - cargo flamegraph workarounds
- [How to Perform Perf Profiling in WSL2](https://scicoding.com/how-to-perform-perf-profiling-in-wsl2/)
- [Install perf on WSL 2](https://gist.github.com/abel0b/b1881e41b9e1c4b16d84e5e083c38a13)
- [Brendan Gregg's CPU Flame Graphs](https://www.brendangregg.com/FlameGraphs/cpuflamegraphs.html)

## Building for Profiling

### Option 1: Custom Profile (Recommended)

Add to `Cargo.toml`:

```toml
[profile.perf]
inherits = "release"
debug = true      # Full debug info for symbol resolution
strip = false     # Keep all symbols
```

Build:

```bash
cargo build --profile=perf --bin steady_state
```

Binary location: `../target/perf/steady_state`

### Option 2: Release with Debug Symbols

```bash
# Using RUSTFLAGS
RUSTFLAGS="-C debuginfo=2" cargo build --release --bin steady_state
```

Binary location: `../target/release/steady_state`

## Running the Steady-State Binary

### Basic Usage

```bash
../target/release/steady_state \
    --operations 1000000 \
    --target-depth 10 \
    --seed 42
```

### Command-Line Options

| Option | Default | Description |
|--------|---------|-------------|
| `--operations` | 1000000 | Total operations to execute |
| `--target-depth` | 10 | Target price levels per side |
| `--seed` | 42 | Random seed for determinism |
| `--prob-add` | 0.45 | Add operation probability |
| `--prob-cancel` | 0.35 | Cancel operation probability |
| `--prob-fill` | 0.15 | Fill operation probability |
| `--prob-modify` | 0.05 | Modify operation probability |
| `--report-interval` | 100000 | Progress report frequency |

### Example Configurations

**High-Frequency Trading (more cancels/modifies):**
```bash
../target/release/steady_state \
    --operations 10000000 \
    --prob-add 0.30 \
    --prob-cancel 0.50 \
    --prob-fill 0.10 \
    --prob-modify 0.10
```

**Execution-Heavy (more fills):**
```bash
../target/release/steady_state \
    --operations 10000000 \
    --prob-add 0.40 \
    --prob-cancel 0.25 \
    --prob-fill 0.30 \
    --prob-modify 0.05
```

**Deep Book:**
```bash
../target/release/steady_state \
    --operations 10000000 \
    --target-depth 50
```

## Optional: Operation-Level Monitoring with Tracing

For operation-level timing during debugging, you can add `tracing` spans to the code. **However, for performance analysis, use the following tools instead:**

- **Criterion benchmarks** (`cargo bench`) - Provides precise per-operation timing with statistical analysis
- **perf/flamegraph** (below) - CPU-level profiling without instrumentation overhead
- **Tracing spans** (this section) - Optional for debugging specific operations

### When to Use Tracing Spans

Use tracing spans when:
- Debugging performance issues in specific code paths
- Understanding operation sequencing and timing
- Investigating latency outliers

**Do NOT use** tracing spans for:
- CPU profiling (use perf instead - see below)
- Benchmark comparisons (use criterion instead)
- Production performance measurement (adds overhead)

### Adding Tracing Spans

The `steady_state` binary uses `tracing` for logging. You can add spans for timing specific operations:

**Example - Function instrumentation:**
```rust
use tracing::instrument;

#[instrument(skip_all)]
fn execute_add_order(book: &mut OrderBook, order: Order) {
    book.add_order(order);
}
```

**Example - Inline spans:**
```rust
let _span = tracing::debug_span!("add_order", order_id = order.order_id).entered();
book.add_order(order);
```

**Running with tracing:**
```bash
# Show debug spans
RUST_LOG=debug cargo run --bin steady_state -- --operations 10000

# Show trace-level spans (very verbose)
RUST_LOG=trace cargo run --bin steady_state -- --operations 1000
```

**Important:** Tracing spans add overhead. For realistic CPU profiling, use perf without tracing (see next section).

## CPU Profiling with Perf

### Basic CPU Profiling

Record CPU samples with call graphs:

```bash
perf record -F 997 --call-graph dwarf \
    ../target/release/steady_state --operations 10000000
```

View report:

```bash
# Text report
perf report --stdio | less

# Interactive TUI
perf report
```

### Sampling Frequency

- `-F 997`: Sample at 997 Hz (default, ~1ms resolution)
- `-F 99`: Lower frequency, smaller `perf.data` file
- `-F 4999`: Higher frequency, more accurate hotspot detection

**Note:** Use prime numbers to avoid aliasing with periodic workloads.

### Call Graph Methods

```bash
# DWARF (Recommended for Rust)
perf record -F 997 --call-graph dwarf ...

# Frame pointers (not reliable in Rust)
perf record -F 997 --call-graph fp ...
```

Always use `--call-graph dwarf` for Rust programs - frame pointers are not always available.

### Quick Statistics with perf stat

```bash
perf stat ../target/release/steady_state --operations 1000000
```

Output includes:
- Total instructions executed
- CPU cycles consumed
- Cache references and misses
- Branch predictions and mispredictions
- IPC (instructions per cycle)

Example:

```bash
perf stat -d ../target/release/steady_state --operations 1000000
```

The `-d` flag adds detailed cache and TLB statistics.

## Generating Flamegraphs

### Using cargo-flamegraph (Easiest)

```bash
# Install once
cargo install flamegraph

# Generate flamegraph
cargo flamegraph --bin steady_state -- --operations 10000000

# Opens flamegraph.svg in browser automatically
```

### Using FlameGraph Scripts

```bash
# 1. Record with perf
perf record -F 997 --call-graph dwarf -g \
    ../target/release/steady_state --operations 10000000

# 2. Generate flamegraph
perf script | stackcollapse-perf.pl | flamegraph.pl > flamegraph.svg

# 3. View in browser
firefox flamegraph.svg
```

### Flamegraph Interpretation

**Wide Bars = Hotspots:**
- Width represents total CPU time spent in that function
- Clicking a function zooms into its call tree
- Search box (Ctrl+F) to find specific functions

**Expected Hotspots:**
- `BTreeMap::insert` / `BTreeMap::remove` - Price level operations
- `HashMap::get` / `HashMap::insert` - Order lookups
- `ChaCha8Rng` / `Normal::sample` / `Exp::sample` - Random generation
- `OrderBook::add_order` / `remove_order` / `fill_order` / `modify_order`

**Colors:**
- Colors are random for visual distinction (not meaningful)
- Frames are sorted alphabetically at each level

## Interpreting Results

### What to Look For

#### 1. CPU Hotspots

Identify functions consuming the most CPU time:

```bash
perf report --stdio | head -40
```

Look for:
- Functions with >5% of total samples
- Unexpected functions in hot path
- Library code dominating (potential optimization target)

**Example Analysis:**
```
  25.3%  BTreeMap::insert     # Expected - core operation
  18.7%  HashMap::get         # Expected - order lookups
  12.4%  ChaCha8Rng::next_u64 # May be optimizable if too high
   8.9%  OrderGenerator::next_order
   6.2%  BTreeMap::remove
```

If RNG dominates (>30%), consider:
- Batch generation of random values
- Simpler RNG for non-cryptographic needs
- Reducing unnecessary randomization

#### 2. Cache Performance

```bash
perf stat -e cache-references,cache-misses,L1-dcache-load-misses \
    ../target/release/steady_state --operations 1000000
```

**Good Cache Performance:**
- Cache miss rate < 5%
- L1 data cache miss rate < 3%

**Poor Cache Performance:**
- Cache miss rate > 10%
- Indicates poor data locality

**For Order Books:**
- HashMap order lookups: expect high cache hit rate
- BTreeMap traversals: expect more cache misses (tree structure)

#### 3. Branch Prediction

```bash
perf stat -e branches,branch-misses \
    ../target/release/steady_state --operations 1000000
```

**Good Branch Prediction:**
- Branch miss rate < 5%

**High Misprediction:**
- Match statements with unpredictable patterns
- Random control flow
- Complex conditional logic

**For Order Books:**
- `match order.side` should predict well (roughly 50/50)
- Option handling (`Some`/`None`) may mispredict
- Action selection may mispredict due to dynamic probabilities

#### 4. Instructions Per Cycle (IPC)

```bash
perf stat ../target/release/steady_state --operations 1000000
```

Look for the IPC metric:

```
2.45 insn per cycle
```

**IPC Interpretation:**
- IPC < 1.0: CPU stalled (memory, cache misses, branch mispredictions)
- IPC 1.0-2.0: Typical for complex code with pointer chasing
- IPC 2.0-4.0: Good IPC for compute-intensive code
- IPC > 4.0: Excellent IPC (rare, mostly SIMD or simple loops)

**For Order Books:**
- Expect IPC around 1.5-2.5 (pointer chasing, tree traversals)
- Lower IPC indicates memory/cache bottleneck

## Advanced Profiling

### Event-Based Profiling

#### Cache Miss Profiling

```bash
# L1 data cache misses
perf record -e L1-dcache-load-misses --call-graph dwarf \
    ../target/release/steady_state --operations 10000000

# Last-level cache (LLC) misses
perf record -e LLC-load-misses --call-graph dwarf \
    ../target/release/steady_state --operations 10000000
```

#### Branch Misprediction Profiling

```bash
perf record -e branch-misses --call-graph dwarf \
    ../target/release/steady_state --operations 10000000
```

#### Memory Access Profiling

```bash
perf mem record ../target/release/steady_state --operations 1000000
perf mem report --stdio
```

Shows memory access patterns and latency.

### Multi-Event Recording

Record multiple events simultaneously:

```bash
perf record \
    -e cycles:u,instructions:u,cache-misses:u,branch-misses:u \
    -F 997 --call-graph dwarf \
    ../target/release/steady_state --operations 10000000

# Analyze specific event
perf report --stdio -e cache-misses
```

### Annotated Source Code

View assembly with sample counts:

```bash
perf annotate --stdio OrderBook::add_order
```

Shows:
- Assembly instructions
- Sample counts per instruction
- Source lines (if debug info available)

Useful for micro-optimization of hot functions.

### Comparing Configurations

Use `perf stat` with multiple runs:

```bash
# Baseline
perf stat -r 10 ../target/release/steady_state \
    --operations 1000000 --target-depth 10 > baseline.txt

# Alternative (deeper book)
perf stat -r 10 ../target/release/steady_state \
    --operations 1000000 --target-depth 50 > deep_book.txt

# Compare results
diff baseline.txt deep_book.txt
```

## Troubleshooting

### Problem: No Function Names in Perf Report

**Symptoms:**
```
  45.2%  [unknown]
  23.1%  0x000055a4b2c41234
```

**Solutions:**
1. Ensure debug symbols enabled: `RUSTFLAGS="-C debuginfo=2" cargo build --release`
2. Verify binary not stripped: `file ../target/release/steady_state`
3. Check symbol table: `nm ../target/release/steady_state | grep OrderBook`
4. Use `--call-graph dwarf` (not `fp`)

### Problem: Permission Denied

**Symptoms:**
```
Error: perf_event_open(...) failed: Permission denied
```

**Solution:**
```bash
# Temporary
sudo sysctl kernel.perf_event_paranoid=1

# Or run with sudo (not recommended)
sudo perf record ...
```

### Problem: Flamegraph Shows Only Hex Addresses

**Symptoms:**
SVG shows addresses like `0x55a4b2c41234` instead of function names.

**Solutions:**
1. Rebuild with debug symbols
2. Regenerate flamegraph after fixing symbols
3. Ensure perf.data matches current binary (rebuild if changed)

### Problem: Huge perf.data File

**Symptoms:**
`perf.data` file is multi-gigabyte.

**Solutions:**
1. Reduce sampling frequency: `-F 99` instead of `-F 997`
2. Reduce operation count: `--operations 1000000` instead of `--operations 10000000`
3. Profile user-space only: add `:u` to events (`-e cycles:u`)

### Problem: Inconsistent Results

**Symptoms:**
Flamegraph or perf report shows different hotspots on repeated runs.

**Solutions:**
1. Increase operation count for stable sampling
2. Use higher sampling frequency (`-F 4999`)
3. Disable CPU frequency scaling:
```bash
sudo cpupower frequency-set --governor performance
```
4. Pin to specific CPU core:
```bash
taskset -c 0 ../target/release/steady_state ...
```

## Performance Optimization Workflow

### 1. Baseline Measurement

```bash
# Record baseline
perf stat -r 5 ../target/release/steady_state --operations 10000000 > baseline.txt

# Generate baseline flamegraph
cargo flamegraph --bin steady_state -- --operations 10000000
mv flamegraph.svg flamegraph_baseline.svg
```

### 2. Identify Hotspots

```bash
perf report --stdio | head -50 > hotspots.txt
```

Questions to ask:
- Which functions consume >10% CPU?
- Are hotspots expected (core logic) or unexpected (overhead)?
- Is RNG consuming >20% CPU?
- Are there many cache misses?

### 3. Hypothesize Optimizations

Based on hotspots:
- **BTreeMap dominates**: Consider alternative data structures (skip list, B-tree variants)
- **HashMap lookups high**: Check load factor, consider FxHash or AHash
- **RNG dominates**: Batch generate random values, use simpler RNG
- **Cache misses high**: Improve data layout, reduce pointer chasing
- **Allocations visible**: Use object pooling or arena allocators

### 4. Implement and Measure

```bash
# After code changes
cargo build --release --bin steady_state

# Measure again
perf stat -r 5 ../target/release/steady_state --operations 10000000 > optimized.txt

# Compare
diff baseline.txt optimized.txt
```

### 5. Validate Improvement

```bash
# Generate comparison flamegraph
cargo flamegraph --bin steady_state -- --operations 10000000
mv flamegraph.svg flamegraph_optimized.svg

# Visual comparison in browser
firefox flamegraph_baseline.svg flamegraph_optimized.svg
```

### 6. A/B Testing

```bash
# Automated comparison
for config in baseline optimized; do
    echo "Testing $config..."
    perf stat -r 10 ../target/release/steady_state_$config \
        --operations 5000000 2>&1 | tee perf_$config.txt
done
```

## Example Session

Here's a complete performance profiling session:

```bash
# 1. Build with debug symbols
cargo build --release --bin steady_state

# 2. Quick baseline
perf stat ../target/release/steady_state --operations 1000000

# 3. Detailed CPU profiling
perf record -F 997 --call-graph dwarf \
    ../target/release/steady_state --operations 10000000

# 4. Analyze results
perf report --stdio | tee cpu_report.txt | head -50

# 5. Generate flamegraph
cargo flamegraph --bin steady_state -- --operations 10000000
# Opens flamegraph.svg

# 6. Cache analysis
perf stat -e cache-references,cache-misses,L1-dcache-load-misses \
    ../target/release/steady_state --operations 10000000

# 7. Branch prediction analysis
perf stat -e branches,branch-misses \
    ../target/release/steady_state --operations 10000000

# 8. Compare different depths
for depth in 5 10 20 50; do
    echo "=== Depth $depth ===" | tee -a comparison.txt
    perf stat -r 5 ../target/release/steady_state \
        --operations 1000000 --target-depth $depth 2>&1 | \
        tee -a comparison.txt
done
```

## Performance Targets

Based on the RainyBook implementation:

| Metric | Target | Excellent |
|--------|--------|-----------|
| Throughput | >500K ops/sec | >1M ops/sec |
| Add latency | <500ns mean | <300ns mean |
| Cancel latency | <500ns mean | <300ns mean |
| Fill latency | <500ns mean | <300ns mean |
| Cache miss rate | <10% | <5% |
| Branch miss rate | <10% | <5% |
| IPC | >1.5 | >2.0 |

## Further Reading

- [Linux perf Wiki](https://perf.wiki.kernel.org/index.php/Main_Page)
- [Brendan Gregg's Perf Examples](http://www.brendangregg.com/perf.html)
- [Flamegraph](http://www.brendangregg.com/flamegraphs.html)
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [Intel VTune Profiler](https://www.intel.com/content/www/us/en/developer/tools/oneapi/vtune-profiler.html) (alternative GUI profiler)

## Summary

Performance profiling workflow:
1. **Build** with debug symbols (`cargo build --release`)
2. **Run** steady_state binary with appropriate operation count
3. **Profile** with `perf record --call-graph dwarf`
4. **Visualize** with flamegraphs (`cargo flamegraph`)
5. **Analyze** hotspots, cache misses, branch predictions
6. **Optimize** based on findings
7. **Validate** improvement with perf stat comparisons

The steady_state binary provides a deterministic, representative workload ideal for identifying performance bottlenecks and validating optimizations in the RainyBook order book implementation.
