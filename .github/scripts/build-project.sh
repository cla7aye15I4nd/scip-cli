#!/usr/bin/env bash
set -euo pipefail

project=$(jq -r .name <<< "$PROJECT_JSON")
repo_url=$(jq -r .repoUrl <<< "$PROJECT_JSON")
source_dir="$RUNNER_TEMP/source"
build_dir="$RUNNER_TEMP/build"
index_path="$RUNNER_TEMP/$project.scip"
mkdir -p "$build_dir"
git clone --depth 1 "$repo_url" "$source_dir"

if [[ "$project" == nginx ]]; then
  (
    cd "$source_dir"
    CC=clang ./auto/configure --with-cc=clang --with-cc-opt=-O2
    bear --output "$build_dir/compile_commands.json" -- make -j4
  )
else
  cmake_args=(
    -DCMAKE_BUILD_TYPE=Release
    -DCMAKE_EXPORT_COMPILE_COMMANDS=ON
    -DCMAKE_C_COMPILER=clang
    -DCMAKE_CXX_COMPILER=clang++
    -DBUILD_TESTING=OFF
  )
  case "$project" in
    brotli) cmake_args+=(-DBROTLI_DISABLE_TESTS=ON) ;;
    freetype) cmake_args+=(-DFT_DISABLE_BROTLI=ON -DFT_DISABLE_BZIP2=ON -DFT_DISABLE_HARFBUZZ=ON -DFT_DISABLE_PNG=ON) ;;
    harfbuzz) cmake_args+=(-DHB_BUILD_SUBSET=OFF) ;;
    libjpeg-turbo) cmake_args+=(-DWITH_TESTS=OFF) ;;
    libtiff) cmake_args+=(-Dtiff-tests=OFF -Dtiff-tools=OFF -Dtiff-contrib=OFF -Dtiff-docs=OFF) ;;
    libwebp) cmake_args+=(-DWEBP_BUILD_EXTRAS=OFF -DWEBP_BUILD_ANIM_UTILS=OFF -DWEBP_BUILD_CWEBP=OFF -DWEBP_BUILD_DWEBP=OFF -DWEBP_BUILD_GIF2WEBP=OFF -DWEBP_BUILD_IMG2WEBP=OFF -DWEBP_BUILD_VWEBP=OFF -DWEBP_BUILD_WEBPINFO=OFF -DWEBP_BUILD_WEBPMUX=OFF) ;;
    opus) cmake_args+=(-DOPUS_BUILD_TESTING=OFF -DOPUS_BUILD_PROGRAMS=OFF) ;;
  esac
  cmake -S "$source_dir" -B "$build_dir" -G Ninja "${cmake_args[@]}"
  cmake --build "$build_dir" --parallel 4
fi

(
  cd "$source_dir"
  "$RUNNER_TEMP/scip-clang" \
    --compdb-path="$build_dir/compile_commands.json" \
    --index-output-path="$index_path" \
    --jobs=4 \
    --no-progress-report
)
test "$(stat -c %s "$index_path")" -ge 1024

commit=$(git -C "$source_dir" rev-parse HEAD)
bin/scip-cli "$index_path" \
  --source-root "$source_dir" \
  --repo-url "$repo_url" \
  --commit "$commit" \
  --output-dir fragment \
  --title "$project"
