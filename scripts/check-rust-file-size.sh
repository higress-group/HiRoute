#!/usr/bin/env bash
set -euo pipefail

soft_limit=700
hard_limit=1100
checked_files=0
failed=0
base=""

if (( $# > 0 )); then
  if [[ "$1" != "--base" || $# != 2 ]]; then
    printf 'usage: %s [--base COMMIT]\n' "$0" >&2
    exit 2
  fi
  base="$2"
  git rev-parse --verify "${base}^{commit}" >/dev/null
fi

rust_files() {
  if [[ -n "${base}" ]]; then
    git diff --name-only -z --diff-filter=ACMR "${base}...HEAD" -- '*.rs'
    git ls-files -z --others --exclude-standard -- '*.rs'
  else
    git ls-files -z --cached --others --exclude-standard -- '*.rs'
  fi
}

while IFS= read -r -d '' rust_file; do
  [[ -f "${rust_file}" ]] || continue
  line_count=$(awk 'END { print NR }' "${rust_file}")
  checked_files=$((checked_files + 1))

  if (( line_count > hard_limit )); then
    base_line_count=0
    if [[ -n "${base}" ]] && git cat-file -e "${base}:${rust_file}" 2>/dev/null; then
      base_line_count=$(git show "${base}:${rust_file}" | awk 'END { print NR }')
    fi
    if (( base_line_count > 0 )); then
      printf 'warning: %s is an existing oversized file at %d lines (base: %d, review responsibility boundary: %d)\n' \
        "${rust_file}" "${line_count}" "${base_line_count}" "${hard_limit}" >&2
    else
      printf 'error: %s has %d lines (base: %d, hard limit: %d)\n' \
        "${rust_file}" "${line_count}" "${base_line_count}" "${hard_limit}" >&2
      failed=1
    fi
  elif (( line_count > soft_limit )); then
    printf 'warning: %s has %d lines (soft target: %d)\n' \
      "${rust_file}" "${line_count}" "${soft_limit}" >&2
  fi
done < <(rust_files)

printf 'checked %d Rust files (soft target: %d, hard limit: %d)\n' \
  "${checked_files}" "${soft_limit}" "${hard_limit}"
exit "${failed}"
