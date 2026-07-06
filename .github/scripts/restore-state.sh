#!/usr/bin/env bash
set -euo pipefail

destination=${1:?destination is required}
mode=${2:-site}

if [[ "${GITHUB_EVENT_NAME:-}" == pull_request && -n "${GITHUB_HEAD_REF:-}" ]]; then
  run_id=$(gh run list \
    --workflow code-browser.yml \
    --branch "$GITHUB_HEAD_REF" \
    --status success \
    --limit 1 \
    --json databaseId \
    --jq '.[0].databaseId // empty')
  if [[ -n "$run_id" ]] && gh run download "$run_id" --name code-browser-preview --dir "$destination"; then
    echo "Restored preview state from run $run_id"
    exit 0
  fi
fi

run_id=$(gh run list \
  --workflow code-browser.yml \
  --branch main \
  --status success \
  --limit 1 \
  --json databaseId \
  --jq '.[0].databaseId // empty')
if [[ -z "$run_id" ]]; then
  echo "No previous state is available"
  exit 0
fi

if [[ "$mode" == catalog ]]; then
  mkdir -p "$destination/generated"
  if gh run download "$run_id" --name code-browser-catalog --dir "$destination/generated"; then
    echo "Restored production catalog from run $run_id"
    exit 0
  fi
fi

gh run download "$run_id" --name code-browser-state --dir "$destination"
echo "Restored production state from run $run_id"
