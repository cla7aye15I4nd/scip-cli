#!/usr/bin/env bash
set -euo pipefail
exec .github/scripts/build-cmake-project.sh -DWITH_TESTS=OFF
