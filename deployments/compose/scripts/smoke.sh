#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${BASE_URL:-http://127.0.0.1:8001}"
USER_ID="${USER_ID:-smoke-u1}"

json_post() {
  local path="$1"
  local body="$2"
  curl -fsS -X POST "${BASE_URL}${path}" \
    -H 'Content-Type: application/json' \
    -d "$body"
}

echo "[1/7] health"
curl -fsS "${BASE_URL}/health" >/dev/null

echo "[2/7] add"
ADD_RES=$(json_post "/product/add" "{\"user_id\":\"${USER_ID}\",\"mem_cube_id\":\"${USER_ID}\",\"memory_content\":\"I like strawberry\",\"async_mode\":\"sync\"}")
ADD_CODE=$(echo "$ADD_RES" | jq -r '.code')
[[ "$ADD_CODE" == "200" ]]
MID=$(echo "$ADD_RES" | jq -r '.data[0].id')
[[ -n "$MID" && "$MID" != "null" ]]

echo "[3/7] search"
SEARCH_RES=$(json_post "/product/search" "{\"query\":\"What do I like\",\"user_id\":\"${USER_ID}\",\"mem_cube_id\":\"${USER_ID}\",\"top_k\":5}")
SEARCH_CODE=$(echo "$SEARCH_RES" | jq -r '.code')
[[ "$SEARCH_CODE" == "200" ]]

echo "[4/7] update"
UPD_RES=$(json_post "/product/update_memory" "{\"memory_id\":\"${MID}\",\"user_id\":\"${USER_ID}\",\"memory\":\"I like strawberry and peach\"}")
UPD_CODE=$(echo "$UPD_RES" | jq -r '.code')
[[ "$UPD_CODE" == "200" ]]

echo "[5/7] get"
GET_RES=$(json_post "/product/get_memory" "{\"memory_id\":\"${MID}\",\"user_id\":\"${USER_ID}\"}")
GET_CODE=$(echo "$GET_RES" | jq -r '.code')
GET_TEXT=$(echo "$GET_RES" | jq -r '.data.memory')
[[ "$GET_CODE" == "200" ]]
[[ "$GET_TEXT" == "I like strawberry and peach" ]]

echo "[6/7] soft delete"
DEL_RES=$(json_post "/product/delete_memory" "{\"memory_id\":\"${MID}\",\"user_id\":\"${USER_ID}\",\"soft\":true}")
DEL_CODE=$(echo "$DEL_RES" | jq -r '.code')
[[ "$DEL_CODE" == "200" ]]

GET_DEL_RES=$(json_post "/product/get_memory" "{\"memory_id\":\"${MID}\",\"user_id\":\"${USER_ID}\"}")
GET_DEL_CODE=$(echo "$GET_DEL_RES" | jq -r '.code')
[[ "$GET_DEL_CODE" == "404" ]]

echo "[7/7] audit"
AUDIT_RES=$(curl -fsS "${BASE_URL}/product/audit/list?user_id=${USER_ID}&limit=10")
AUDIT_CODE=$(echo "$AUDIT_RES" | jq -r '.code')
AUDIT_COUNT=$(echo "$AUDIT_RES" | jq -r '.data | length')
[[ "$AUDIT_CODE" == "200" ]]
[[ "$AUDIT_COUNT" -ge "3" ]]

echo "smoke test passed"
