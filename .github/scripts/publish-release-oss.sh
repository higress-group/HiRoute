#!/usr/bin/env bash
set -euo pipefail

asset_dir=${1:?usage: publish-release-oss.sh ASSET_DIR MANIFEST TAG}
manifest=${2:?usage: publish-release-oss.sh ASSET_DIR MANIFEST TAG}
tag=${3:?usage: publish-release-oss.sh ASSET_DIR MANIFEST TAG}
: "${ACCESS_KEYID:?ACCESS_KEYID is required}"
: "${ACCESS_KEYSECRET:?ACCESS_KEYSECRET is required}"

bucket=hiroute-ai
endpoint=oss-cn-hongkong.aliyuncs.com
region=cn-hongkong
common=(--endpoint "$endpoint" --region "$region" --access-key-id "$ACCESS_KEYID" --access-key-secret "$ACCESS_KEYSECRET")

asset_list=$(mktemp)
object_headers=$(mktemp)
head_error=$(mktemp)
trap 'rm -f "$asset_list" "$object_headers" "$head_error"' EXIT
node apps/website/scripts/release-manifest.mjs artifacts "$manifest" --tag "$tag" > "$asset_list"

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

# Resolve and check the entire local publication set before the first remote write.
while IFS=$'\t' read -r filename key sha size; do
  [[ -n "$filename" && -f "$asset_dir/$filename" ]] || { echo "Missing release asset: $filename" >&2; exit 1; }
done < "$asset_list"

while IFS=$'\t' read -r filename key sha size; do
  metadata="Cache-Control:public,max-age=31536000,immutable#Content-Disposition:attachment; filename=$filename#X-Oss-Meta-Sha256:$sha"
  status=$(head_object "$key") || exit 1
  case "$status" in
    200)
      grep -Eiq "^X-Oss-Meta-Sha256[[:blank:]]*:[[:blank:]]*$sha[[:space:]]*$" "$object_headers" || { echo "Existing immutable object has a different SHA256: $key" >&2; exit 1; }
      grep -Eiq "^Content-Length[[:blank:]]*:[[:blank:]]*$size[[:space:]]*$" "$object_headers" || { echo "Existing immutable object has a different size: $key" >&2; exit 1; }
      echo "Immutable release object already exists: $key"
      continue
      ;;
    404) ;;
    *)
      echo "Could not determine whether immutable release object exists (HTTP $status): $key" >&2
      exit 1
      ;;
  esac
  aliyun oss cp "$asset_dir/$filename" "oss://$bucket/$key" --meta "$metadata" "${common[@]}"
  status=$(head_object "$key") || exit 1
  [[ "$status" == 200 ]] || { echo "Uploaded object is not publicly readable (HTTP $status): $key" >&2; exit 1; }
  grep -Eiq "^X-Oss-Meta-Sha256[[:blank:]]*:[[:blank:]]*$sha[[:space:]]*$" "$object_headers" || { echo "Uploaded object lacks expected SHA256 metadata: $key" >&2; exit 1; }
  grep -Eiq "^Content-Length[[:blank:]]*:[[:blank:]]*$size[[:space:]]*$" "$object_headers" || { echo "Uploaded object size does not match: $key" >&2; exit 1; }
done < "$asset_list"
