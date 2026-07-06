#!/usr/bin/env bash
set -euo pipefail
exec .github/scripts/build-cmake-project.sh \
  -Dtiff-tests=OFF \
  -Dtiff-tools=OFF \
  -Dtiff-contrib=OFF \
  -Dtiff-docs=OFF
