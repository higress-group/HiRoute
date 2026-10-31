#!/usr/bin/env bash
set -euo pipefail

payload=${1:?usage: deploy-model-feed-oss.sh VALIDATED_PAYLOAD}
[[ -f "$payload" ]] || { echo "Validated model feed missing" >&2; exit 1; }
: "${ACCESS_KEYID:?ACCESS_KEYID is required}"
: "${ACCESS_KEYSECRET:?ACCESS_KEYSECRET is required}"

# The production workflow checks the served publication and stages these exact
# bytes before acquiring credentials. Never upload a website tree or delete keys.
aliyun oss cp "$payload" oss://hiroute-ai/model-feed/v2.json --force \
  --meta "Content-Type:application/json#Cache-Control:public,max-age=300" \
  --endpoint oss-cn-hongkong.aliyuncs.com --region cn-hongkong \
  --access-key-id "$ACCESS_KEYID" --access-key-secret "$ACCESS_KEYSECRET"
