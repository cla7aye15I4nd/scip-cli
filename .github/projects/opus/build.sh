#!/usr/bin/env bash
set -euo pipefail
exec .github/scripts/build-cmake-project.sh \
  -DOPUS_BUILD_TESTING=OFF \
  -DOPUS_BUILD_PROGRAMS=OFF
