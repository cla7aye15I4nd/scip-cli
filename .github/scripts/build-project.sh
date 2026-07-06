#!/usr/bin/env bash
set -euo pipefail

project=$(jq -r .name <<< "$PROJECT_JSON")
repo_url=$(jq -r .repoUrl <<< "$PROJECT_JSON")
source_dir="$RUNNER_TEMP/source"
build_dir="$RUNNER_TEMP/build"
index_path="$RUNNER_TEMP/$project.scip"
mkdir -p "$build_dir"
git clone --depth 1 "$repo_url" "$source_dir"
export SOURCE_DIR="$source_dir"
export BUILD_DIR="$build_dir"
export BUILD_JOBS=4
project_build=".github/projects/$project/build.sh"
test -x "$project_build"
"$project_build"

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
