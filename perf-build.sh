cargo perf-build --bin steady_state
#sudo perf record -F 199 -g --call-graph dwarf ../target/perf/steady_state <program-args>