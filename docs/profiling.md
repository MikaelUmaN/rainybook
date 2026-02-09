# Performance Profiling

## Criterion Benchmarks

Run benchmarks to measure operation timing:

```bash
# Run all benchmarks
cargo bench

# Run specific benchmark
cargo bench benchmark_name

# Run with baseline comparison
cargo bench --bench benchmark_suite -- --save-baseline main
# After changes:
cargo bench --bench benchmark_suite -- --baseline main
```

Criterion outputs:
- Mean, median, standard deviation
- Throughput measurements
- HTML reports in `../target/criterion/`

## perf + Flamegraph

Use [`profile.sh`](../profile.sh) to build, profile, and generate flamegraphs:

```bash
./profile.sh --perf --flamegraph    # Profile steady_state + generate flamegraph
./profile.sh --perf my_binary -f    # Profile specific binary
./profile.sh --help                 # See all options
```

Open `flamegraph.svg` in a browser to visualize.

## WSL2 Installation and Limitations

### Installation

WSL2 uses a custom Microsoft kernel, so standard `linux-tools` packages won't work. Build `perf` from source:

```bash
# Clone WSL2 kernel repository
git clone https://github.com/microsoft/WSL2-Linux-Kernel --depth 1
cd WSL2-Linux-Kernel/tools/perf

# Install build dependencies
sudo apt-get update
sudo apt-get install flex bison libelf-dev libunwind-dev \
    libaudit-dev libslang2-dev libperl-dev python3-dev \
    binutils-dev liblzma-dev libzstd-dev libcap-dev \
    libdwarf-dev libdw-dev systemtap-sdt-dev libssl-dev \
    libbabeltrace-dev libiberty-dev

# Build and install
make -j$(nproc)
sudo cp perf /usr/local/bin/

# Verify
perf --version
```

### System Configuration

```bash
# Allow user-space profiling
echo -1 | sudo tee /proc/sys/kernel/perf_event_paranoid

# Optional: Enable kernel symbol resolution
echo 0 | sudo tee /proc/sys/kernel/kptr_restrict
```

### Limitations

WSL2 runs in Hyper-V and lacks hardware performance counter (PMU) support. This is an [ongoing limitation](https://github.com/microsoft/WSL/issues/8480).

**What Works:**
- ✅ CPU time sampling (cpu-clock)
- ✅ Call-graph sampling with DWARF
- ✅ Flamegraph generation
- ✅ Context switches, page faults

**What Doesn't Work:**
- ❌ Hardware performance counters (cycles, instructions)
- ❌ Cache miss analysis (L1, L2, LLC)
- ❌ Branch prediction analysis
- ❌ IPC (Instructions Per Cycle) measurement
- ❌ TLB statistics

### WSL2 Workaround

Use `cpu-clock` software event instead of default hardware events. The `profile.sh` script already uses `cpu-clock:u` for WSL2 compatibility.

### Recommendation

For hardware-level optimization (cache, branch prediction, IPC):
- Use native Linux (dual-boot, bare metal, or cloud VM with nested virtualization disabled)
- For iterative development: WSL2 with flamegraphs is sufficient for hotspot identification
- For production optimization: Validate on native Linux

## References

- [WSL Issue #8480: Hardware PMU support](https://github.com/microsoft/WSL/issues/8480)
- [Brendan Gregg's Flamegraphs](https://www.brendangregg.com/flamegraphs.html)
- [Linux perf Wiki](https://perf.wiki.kernel.org/index.php/Main_Page)
