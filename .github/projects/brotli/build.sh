#!/usr/bin/env bash
set -euo pipefail
exec .github/scripts/build-cmake-project.sh -DBROTLI_DISABLE_TESTS=ON
