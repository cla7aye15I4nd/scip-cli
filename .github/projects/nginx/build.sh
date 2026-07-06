#!/usr/bin/env bash
set -euo pipefail

cd "$SOURCE_DIR"
CC=clang ./auto/configure --with-cc=clang --with-cc-opt=-O2
bear --output "$BUILD_DIR/compile_commands.json" -- make -j"$BUILD_JOBS"
