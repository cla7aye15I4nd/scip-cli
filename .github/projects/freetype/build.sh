#!/usr/bin/env bash
set -euo pipefail
exec .github/scripts/build-cmake-project.sh \
  -DFT_DISABLE_BROTLI=ON \
  -DFT_DISABLE_BZIP2=ON \
  -DFT_DISABLE_HARFBUZZ=ON \
  -DFT_DISABLE_PNG=ON
