#!/usr/bin/env bash
set -euo pipefail

dist=${1:?usage: deploy-website-oss.sh DIST MANIFEST}
manifest=${2:?usage: deploy-website-oss.sh DIST MANIFEST}
: "${ACCESS_KEYID:?ACCESS_KEYID is required}"
: "${ACCESS_KEYSECRET:?ACCESS_KEYSECRET is required}"
[[ -f "$dist/index.html" && -f "$manifest" ]] || { echo "Website build or release manifest missing" >&2; exit 1; }
[[ ! -e "$dist/releases" ]] || { echo "Website output must not contain release packages" >&2; exit 1; }

bucket=hiroute-ai
endpoint=oss-cn-hongkong.aliyuncs.com
region=cn-hongkong
common=(--endpoint "$endpoint" --region "$region" --access-key-id "$ACCESS_KEYID" --access-key-secret "$ACCESS_KEYSECRET")

asset_list=$(mktemp)
object_headers=$(mktemp)
head_error=$(mktemp)
trap 'rm -f "$asset_list" "$object_headers" "$head_error"' EXIT
node apps/website/scripts/release-manifest.mjs artifacts "$manifest" > "$asset_list"

head_object() {
  local key=$1 status
  : > "$object_headers"
  : > "$head_error"
  if ! status=$(curl --silent --show-error --head \
    --output "$object_headers" --write-out '%{http_code}' \
    --connect-timeout 10 --max-time 30 \
    "https://$bucket.$endpoint/$key" 2> "$head_error"); then
    cat "$head_error" >&2
    echo "Could not inspect public release object: $key" >&2
    return 1
  fi
  printf '%s' "$status"
}

# A manifest entry is never exposed until its immutable object exists in OSS.
while IFS=$'\t' read -r _ key sha size; do
  status=$(head_object "$key") || exit 1
  [[ "$status" == 200 ]] || { echo "Release object is not publicly readable (HTTP $status): $key" >&2; exit 1; }
  grep -Eiq "^X-Oss-Meta-Sha256[[:blank:]]*:[[:blank:]]*$sha[[:space:]]*$" "$object_headers" || { echo "Release object metadata does not match manifest: $key" >&2; exit 1; }
  grep -Eiq "^Content-Length[[:blank:]]*:[[:blank:]]*$size[[:space:]]*$" "$object_headers" || { echo "Release object size does not match manifest: $key" >&2; exit 1; }
done < "$asset_list"

# Upload website objects without deleting or replacing the releases/ prefix.
aliyun oss cp "$dist" "oss://$bucket/" --recursive --force \
  --meta "Cache-Control:public,max-age=300" "${common[@]}"
if [[ -d "$dist/_astro" ]]; then
  aliyun oss cp "$dist/_astro" "oss://$bucket/_astro/" --recursive --force \
    --meta "Cache-Control:public,max-age=31536000,immutable" "${common[@]}"
fi
