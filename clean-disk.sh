#!/bin/bash

# In this project: delete build files not touched in the last 30 days.
cargo sweep --time 30

# Clean under $CARGO_HOME (e.g. ~/.cargo).
cargo cache --autoclean
