#!/usr/bin/env bash
set -euo pipefail

cmake -S "$SOURCE_DIR" -B "$BUILD_DIR" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_EXPORT_COMPILE_COMMANDS=ON \
  -DCMAKE_C_COMPILER=clang \
  -DCMAKE_CXX_COMPILER=clang++ \
  -DBUILD_TESTING=OFF \
  "$@"
cmake --build "$BUILD_DIR" --parallel "$BUILD_JOBS"
