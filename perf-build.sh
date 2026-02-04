cargo perf-build --bin steady_state
#perf record -e cpu-clock:u -F 199 -g --call-graph dwarf ../target/perf/steady_state <program-args>
#flamegraph --perfdata perf.data -o flamegraph.svg