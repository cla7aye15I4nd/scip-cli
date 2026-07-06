#!/usr/bin/env bash
set -euo pipefail

site=${1:?site directory is required}
incoming=${2:?incoming directory is required}
mkdir -p "$site/assets" "$site/generated"
cp assets/index.html "$site/index.html"
cp assets/index.html "$site/404.html"
cp assets/app.js "$site/assets/app.js"
cp assets/style.css "$site/assets/style.css"
printf '/* /index.html 200\n' > "$site/_redirects"

catalog="$site/generated/catalog.json"
if [[ ! -s "$catalog" ]]; then
  printf '{"version":1,"projects":[]}' > "$catalog"
fi

if [[ -d "$incoming" ]]; then
  while IFS= read -r fragment_catalog; do
    fragment_dir=${fragment_catalog%/generated/catalog.json}
    project=$(jq --compact-output '.projects[0]' "$fragment_catalog")
    slug=$(jq -r .slug <<< "$project")
    commit=$(jq -r '.commits[0].commit' <<< "$project")
    mkdir -p "$site/generated/$slug"
    cp -a "$fragment_dir/generated/$slug/$commit" "$site/generated/$slug/$commit"
    jq --argjson new "$project" '
      (.projects // []) as $projects |
      ($projects | map(select(.slug == $new.slug)) | .[0]) as $old |
      ($new.commits + ($old.commits // [])) as $candidates |
      (reduce $candidates[] as $item ([];
        if any(.[]; .commit == $item.commit) then . else . + [$item] end
      ) | .[:2]) as $commits |
      .version = 1 |
      .projects = (($projects | map(select(.slug != $new.slug))) + [$new | .commits = $commits] | sort_by(.slug))
    ' "$catalog" > "$catalog.tmp"
    mv "$catalog.tmp" "$catalog"
  done < <(find "$incoming" -path '*/generated/catalog.json' -type f | sort)
fi

while IFS= read -r project_dir; do
  slug=${project_dir##*/}
  while IFS= read -r commit_dir; do
    commit=${commit_dir##*/}
    if ! jq --exit-status --arg slug "$slug" --arg commit "$commit" \
      '.projects[] | select(.slug == $slug) | .commits[] | select(.commit == $commit)' \
      "$catalog" > /dev/null; then
      rm -rf "$commit_dir"
    fi
  done < <(find "$project_dir" -mindepth 1 -maxdepth 1 -type d ! -name '.*')
done < <(find "$site/generated" -mindepth 1 -maxdepth 1 -type d ! -name '.*')
