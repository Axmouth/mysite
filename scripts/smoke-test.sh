#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT="${SMOKE_PORT:-3214}"
BASE_URL="http://127.0.0.1:${PORT}"
DATA_DIR="$(mktemp -d)"
COOKIE_JAR="$(mktemp)"
SERVER_LOG="$(mktemp)"
PNG_FILE="$(mktemp --suffix=.png)"

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]]; then
    kill "${SERVER_PID}" 2>/dev/null || true
    wait "${SERVER_PID}" 2>/dev/null || true
  fi
  rm -rf "${DATA_DIR}" "${COOKIE_JAR}" "${SERVER_LOG}" "${PNG_FILE}"
}
trap cleanup EXIT

fail() {
  printf 'smoke test failed: %s\n' "$1" >&2
  printf '%s\n' '--- server log ---' >&2
  cat "${SERVER_LOG}" >&2
  exit 1
}

assert_contains() {
  local content="$1"
  local expected="$2"
  [[ "${content}" == *"${expected}"* ]] || fail "expected response to contain: ${expected}"
}

assert_status() {
  local expected="$1"
  shift
  local actual
  actual="$(curl --silent --show-error --output /dev/null --write-out '%{http_code}' "$@")"
  [[ "${actual}" == "${expected}" ]] || fail "expected HTTP ${expected}, got ${actual}"
}

wait_for_server() {
  for _ in $(seq 1 50); do
    if curl --silent --fail "${BASE_URL}/healthz" >/dev/null 2>&1; then
      return
    fi
    sleep 0.1
  done
  fail "server did not become healthy"
}

start_server() {
  ADMIN_PASSWORD=smoke-password \
  SITE_URL="${BASE_URL}" \
  BIND_ADDRESS="127.0.0.1:${PORT}" \
  DATA_DIR="${DATA_DIR}" \
  ./target/debug/mysite >"${SERVER_LOG}" 2>&1 &
  SERVER_PID=$!
  wait_for_server
}

cd "${ROOT_DIR}"
cargo build --quiet

start_server

HOME_RESPONSE="$(curl --silent --show-error --include "${BASE_URL}/")"
assert_contains "${HOME_RESPONSE}" "HTTP/1.1 200 OK"
assert_contains "${HOME_RESPONSE}" "content-security-policy:"
assert_contains "${HOME_RESPONSE}" '<link rel="canonical"'
assert_contains "${HOME_RESPONSE}" '<meta name="description"'
assert_contains "${HOME_RESPONSE}" "/assets/theme.js?v="
assert_contains "${HOME_RESPONSE}" "/assets/style.css?v="

ROBOTS_RESPONSE="$(curl --silent --show-error "${BASE_URL}/robots.txt")"
assert_contains "${ROBOTS_RESPONSE}" "Disallow: /admin"
assert_contains "${ROBOTS_RESPONSE}" "${BASE_URL}/sitemap.xml"

SITEMAP_RESPONSE="$(curl --silent --show-error "${BASE_URL}/sitemap.xml")"
assert_contains "${SITEMAP_RESPONSE}" "${BASE_URL}/projects"

NOT_FOUND_RESPONSE="$(curl --silent --show-error --include "${BASE_URL}/missing-page")"
assert_contains "${NOT_FOUND_RESPONSE}" "HTTP/1.1 404 Not Found"
assert_contains "${NOT_FOUND_RESPONSE}" "Page not found"
assert_contains "${NOT_FOUND_RESPONSE}" "Return home"

MISSING_PROJECT_RESPONSE="$(curl --silent --show-error --include "${BASE_URL}/projects/missing-project")"
assert_contains "${MISSING_PROJECT_RESPONSE}" "HTTP/1.1 404 Not Found"
assert_contains "${MISSING_PROJECT_RESPONSE}" "Page not found"

SERVER_ERROR_RESPONSE="$(curl --silent --show-error --include "${BASE_URL}/__test/500")"
assert_contains "${SERVER_ERROR_RESPONSE}" "HTTP/1.1 500 Internal Server Error"
assert_contains "${SERVER_ERROR_RESPONSE}" "Something went wrong"
assert_contains "${SERVER_ERROR_RESPONSE}" "Please try again shortly"

assert_status 403 \
  --cookie 'admin_session=fake' \
  --header 'Origin: https://attacker.example' \
  --data 'markdown=blocked' \
  "${BASE_URL}/admin/home"

assert_status 303 \
  --cookie-jar "${COOKIE_JAR}" \
  --data 'password=smoke-password' \
  "${BASE_URL}/admin/login"

assert_status 303 \
  --cookie "${COOKIE_JAR}" \
  --header "Origin: ${BASE_URL}" \
  --data-urlencode 'site_title=Smoke Site' \
  --data-urlencode 'home_seo_title=Smoke Home SEO Title' \
  --data-urlencode 'author_name=Smoke Author' \
  --data-urlencode 'site_description=Smoke description.' \
  --data-urlencode 'social_image=' \
  --data-urlencode 'copyright_claim=Copyright Smoke.' \
  "${BASE_URL}/admin/settings"

HOME_RESPONSE="$(curl --silent --show-error "${BASE_URL}/")"
assert_contains "${HOME_RESPONSE}" "<title>Smoke Home SEO Title</title>"

PROJECTS_RESPONSE="$(curl --silent --show-error "${BASE_URL}/projects")"
assert_contains "${PROJECTS_RESPONSE}" "<title>Projects | Smoke Site</title>"

assert_status 303 \
  --cookie "${COOKIE_JAR}" \
  --header "Origin: ${BASE_URL}" \
  --data-urlencode 'title=Smoke Project' \
  --data-urlencode 'slug=smoke-project' \
  --data-urlencode 'summary=Smoke summary' \
  --data-urlencode 'body=**Smoke body**' \
  --data-urlencode 'image_path=' \
  --data-urlencode 'published=on' \
  --data-urlencode 'featured=on' \
  "${BASE_URL}/admin/projects/new"

PROJECT_RESPONSE="$(curl --silent --show-error "${BASE_URL}/projects/smoke-project")"
assert_contains "${PROJECT_RESPONSE}" "<strong>Smoke body</strong>"
assert_contains "${PROJECT_RESPONSE}" "Smoke summary"
PROJECTS_RESPONSE="$(curl --silent --show-error "${BASE_URL}/projects")"
assert_contains "${PROJECTS_RESPONSE}" "Featured"

printf '\211PNG\r\n\032\n' >"${PNG_FILE}"
UPLOAD_RESPONSE="$(curl --silent --show-error \
  --cookie "${COOKIE_JAR}" \
  --header "Origin: ${BASE_URL}" \
  --form "image=@${PNG_FILE};type=image/png" \
  "${BASE_URL}/admin/projects/1/images")"
assert_contains "${UPLOAD_RESPONSE}" '"filePath":"/uploads/'
UPLOAD_PATH="$(printf '%s' "${UPLOAD_RESPONSE}" | sed -n 's/.*"filePath":"\([^"]*\)".*/\1/p')"
UPLOAD_FILE="${DATA_DIR}${UPLOAD_PATH}"
[[ -f "${UPLOAD_FILE}" ]] || fail "registered project upload was not written"

printf 'untracked' >"${DATA_DIR}/uploads/untracked.txt"
kill "${SERVER_PID}"
wait "${SERVER_PID}" 2>/dev/null || true
unset SERVER_PID
start_server
[[ ! -e "${DATA_DIR}/uploads/untracked.txt" ]] || fail "startup cleanup kept an untracked file"
[[ -f "${UPLOAD_FILE}" ]] || fail "startup cleanup removed a registered file"

rm -f "${COOKIE_JAR}"
assert_status 303 \
  --cookie-jar "${COOKIE_JAR}" \
  --data 'password=smoke-password' \
  "${BASE_URL}/admin/login"

assert_status 303 \
  --cookie "${COOKIE_JAR}" \
  --header "Origin: ${BASE_URL}" \
  --data '' \
  "${BASE_URL}/admin/projects/1/delete"
[[ ! -e "${UPLOAD_FILE}" ]] || fail "project deletion kept an owned image"

assert_status 200 "${BASE_URL}/assets/vendor/easymde/easymde.min.js"

for _ in $(seq 1 5); do
  assert_status 401 --data 'password=incorrect' "${BASE_URL}/admin/login"
done
assert_status 429 --data 'password=incorrect' "${BASE_URL}/admin/login"

printf 'smoke tests passed\n'
