#!/usr/bin/env bash
set -euo pipefail

projects='[]'
previous_catalog="${PREVIOUS_SITE:-previous-site}/generated/catalog.json"
while IFS= read -r config; do
  project=$(ruby -ryaml -rjson -e 'data = YAML.safe_load_file(ARGV[0]); data.delete("build"); puts JSON.generate(data)' "$config")
  name=$(jq -r .name <<< "$project")
  if [[ "${config##*/}" != "$name.yaml" ]]; then
    echo "Project name $name does not match $config" >&2
    exit 1
  fi
  repo_url=$(jq -r .repoUrl <<< "$project")
  commit=$(git ls-remote "$repo_url" HEAD | awk 'NR == 1 { print $1 }')
  config_hash=$(sha256sum "$config" | awk '{ print $1 }')
  project=$(jq --compact-output \
    --arg commit "$commit" \
    --arg config_hash "$config_hash" \
    '. + {commit: $commit, configHash: $config_hash}' <<< "$project")
  build=true
  if [[ "${FORCE_REBUILD:-false}" != true && -s "$previous_catalog" ]]; then
    if jq --exit-status \
      --arg repo_url "$repo_url" \
      --arg commit "$commit" \
      '.projects[] | select(.repoUrl == $repo_url) | .commits[] | select(.commit == $commit)' \
      "$previous_catalog" > /dev/null; then
      build=false
      echo "$name is already indexed at $commit" >&2
    fi
  fi
  if [[ "$build" == true ]]; then
    projects=$(jq --compact-output --argjson project "$project" '. + [$project]' <<< "$projects")
  fi
done < <(find .github/projects -mindepth 1 -maxdepth 1 -name '*.yaml' -type f | sort)

echo "projects=$projects" >> "$GITHUB_OUTPUT"
echo "count=$(jq 'length' <<< "$projects")" >> "$GITHUB_OUTPUT"
jq -r '.[].name' <<< "$projects"
