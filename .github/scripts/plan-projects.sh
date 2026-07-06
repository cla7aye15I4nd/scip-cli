#!/usr/bin/env bash
set -euo pipefail

projects='[]'
while IFS= read -r project; do
  repo_url=$(jq -r .repoUrl <<< "$project")
  build=true
  if [[ "${EVENT_NAME:-}" == schedule && -s previous-site/generated/catalog.json ]]; then
    commit=$(git ls-remote "$repo_url" HEAD | awk 'NR == 1 { print $1 }')
    if jq --exit-status \
      --arg repo_url "$repo_url" \
      --arg commit "$commit" \
      '.projects[] | select(.repoUrl == $repo_url) | .commits[] | select(.commit == $commit)' \
      previous-site/generated/catalog.json > /dev/null; then
      build=false
    fi
  fi
  if [[ "$build" == true ]]; then
    projects=$(jq --compact-output --argjson project "$project" '. + [$project]' <<< "$projects")
  fi
done < <(jq --compact-output '.[]' .github/projects.json)

echo "projects=$projects" >> "$GITHUB_OUTPUT"
echo "count=$(jq 'length' <<< "$projects")" >> "$GITHUB_OUTPUT"
jq -r '.[].name' <<< "$projects"
