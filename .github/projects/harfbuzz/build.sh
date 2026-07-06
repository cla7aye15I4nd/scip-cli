#!/usr/bin/env bash
set -euo pipefail
exec .github/scripts/build-cmake-project.sh -DHB_BUILD_SUBSET=OFF
