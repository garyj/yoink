#!/usr/bin/env bash
set -euo pipefail

versions=$(cargo metadata --locked --no-deps --format-version 1 | jq -r '.packages[].version' | sort -u)
if [[ $(wc -l <<<"$versions") -ne 1 ]]; then
  echo "Every Cargo package must have the same version, found: ${versions//$'\n'/ }" >&2
  exit 1
fi
version=$versions
if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Use a stable major.minor.patch version" >&2
  exit 1
fi
if [[ ${GITHUB_REF_TYPE:-} == tag && ${GITHUB_REF_NAME:-} != "v$version" ]]; then
  echo "Release tag must match the package version" >&2
  exit 1
fi
echo "$version"
