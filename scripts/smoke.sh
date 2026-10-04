#!/usr/bin/env bash
#
# The checks only a running database and a real socket can make.
#
# This exists because two defects of the same kind reached a review: the column
# holding an event is `jsonb`, the Rust side holds that JSON as a string, and
# neither the write nor the claim said so. The write failed with a parameter
# serialization error and the claim would have panicked on its first row, and
# nothing in the gate noticed, because a unit test of the mapping never touches
# the statement and mutation analysis deliberately skips the crates that do.
#
# So the SQL is exercised here, against PostgreSQL, through the API. Every
# assertion below is one a type checker cannot make.
#
# It is not part of scripts/verify.sh. That script runs inside a container, and a
# gate that starts containers from inside one needs the daemon socket mounted
# into it, which is a larger grant than a verification should ask for. Run this
# beside it.
set -euo pipefail

readonly REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPOSITORY_ROOT}"

# A project of its own, so this cannot touch a stack somebody is already running,
# and ports that are overridable because 5432 and 8080 are the two most likely to
# be taken on a developer's machine.
readonly PROJECT='rust-standards-smoke'
readonly API_PORT="${SMOKE_API_PORT:-18080}"
readonly DB_PORT="${SMOKE_DB_PORT:-15432}"
readonly BASE_URL="http://localhost:${API_PORT}"

# A token for a stack that lives for the length of this script.
export API_TOKEN='a-smoke-test-token'
export SMOKE_API_PORT="${API_PORT}"
export SMOKE_DB_PORT="${DB_PORT}"

readonly STARTUP_ATTEMPTS=30
readonly SECONDS_BETWEEN_ATTEMPTS=2
# The relay drains on a timer, so a published row cannot be asserted sooner.
readonly RELAY_INTERVAL_SECONDS=5
readonly RELAY_GRACE_SECONDS=4

readonly OK=200
readonly CREATED=201
readonly NOT_ALLOWED=405
readonly UNPROCESSABLE=422

readonly PORTS_OVERRIDE="$(mktemp)"
trap 'docker compose -p "${PROJECT}" down --volumes --remove-orphans > /dev/null 2>&1; rm -f "${PORTS_OVERRIDE}"' EXIT

# `!override` rather than a second ports list: compose merges lists, so without
# it the API would publish the default port as well and collide with whatever
# holds it.
cat > "${PORTS_OVERRIDE}" << YAML
services:
  db:
    ports: !override
      - "${DB_PORT}:5432"
  api:
    ports: !override
      - "${API_PORT}:8080"
YAML

compose() {
  docker compose -p "${PROJECT}" -f compose.yaml -f "${PORTS_OVERRIDE}" "$@"
}

in_the_database() {
  compose exec -T db psql -U standards -d standards -tAc "$1"
}

status_of() {
  curl -s -o /dev/null -w '%{http_code}' "$@"
}

expect() {
  local expected="$1" actual="$2" what="$3"
  if [ "${expected}" != "${actual}" ]; then
    echo "FAIL ${what}: expected ${expected}, got ${actual}" >&2
    exit 1
  fi
  echo "ok   ${what}"
}

announce() {
  printf '\n=== %s ===\n' "$1"
}

announce 'the stack'
compose up --build --detach

for attempt in $(seq "${STARTUP_ATTEMPTS}"); do
  if [ "$(status_of "${BASE_URL}/health")" = "${OK}" ]; then
    break
  fi
  if [ "${attempt}" -eq "${STARTUP_ATTEMPTS}" ]; then
    echo 'the API never answered its liveness probe' >&2
    compose logs api >&2
    exit 1
  fi
  sleep "${SECONDS_BETWEEN_ATTEMPTS}"
done

announce 'the edge'
expect "${OK}" "$(status_of "${BASE_URL}/health")" 'liveness answers a read'
expect "${NOT_ALLOWED}" "$(status_of -X POST "${BASE_URL}/health")" \
  'liveness refuses a write'
expect "${NOT_ALLOWED}" "$(status_of -X POST "${BASE_URL}/health/ready")" \
  'readiness refuses a write'
expect "${OK}" "$(status_of "${BASE_URL}/health/ready")" \
  'readiness answers while the database is up'
expect "${NOT_ALLOWED}" \
  "$(status_of -X PATCH -H "Authorization: Bearer ${API_TOKEN}" "${BASE_URL}/api/tasks")" \
  'the task collection refuses PATCH rather than reading the path as an identifier'
expect "${UNPROCESSABLE}" \
  "$(status_of -X POST -H "Authorization: Bearer ${API_TOKEN}" \
    -H 'content-type: application/json' -d '{}' "${BASE_URL}/api/signups")" \
  'an empty form is answered per field rather than as an unreadable body'

announce 'the stores'
expect "${CREATED}" \
  "$(status_of -X POST -H "Authorization: Bearer ${API_TOKEN}" \
    -H 'content-type: application/json' \
    -d '{"fullName":"Ada Lovelace","email":"ada@example.com","plan":"Growth","seats":3,"acceptTerms":true}' \
    "${BASE_URL}/api/signups")" \
  'a valid signup is recorded, which writes an event beside it'
expect "${CREATED}" \
  "$(status_of -X POST -H "Authorization: Bearer ${API_TOKEN}" \
    -H 'content-type: application/json' -d '{"title":"Read the ADR"}' \
    "${BASE_URL}/api/tasks")" \
  'a task is created'

readonly FIRST_TASK="$(in_the_database 'SELECT min(id) FROM tasks')"
expect "${OK}" \
  "$(status_of -X PATCH -H "Authorization: Bearer ${API_TOKEN}" \
    -H 'content-type: application/json' -d '{"completed":true}' \
    "${BASE_URL}/api/tasks/${FIRST_TASK}")" \
  'completing a task writes its event through the jsonb column'

announce 'the relay'
# An event whose type this build cannot resolve, and one whose type it can read
# with a body it cannot, both older than everything valid so they sit at the head
# of the ordered batch. A relay that let either block the queue would leave the
# valid rows below unpublished forever.
in_the_database "INSERT INTO outbox (event_id, type, payload, occurred_at) VALUES
  (gen_random_uuid(), 'AccountClosed', '{\"anything\":1}'::jsonb, now() - interval '2 hours'),
  (gen_random_uuid(), 'TaskCompleted', '{}'::jsonb, now() - interval '1 hour')" > /dev/null

sleep $((RELAY_INTERVAL_SECONDS + RELAY_GRACE_SECONDS))

expect '2' \
  "$(in_the_database "SELECT count(*) FROM outbox WHERE published_at IS NOT NULL")" \
  'both real events published, from behind two undeliverable rows'
expect '1' \
  "$(in_the_database "SELECT count(*) FROM outbox
      WHERE type = 'AccountClosed' AND published_at IS NULL AND quarantined_at IS NULL")" \
  'a type this build cannot resolve waits, unacknowledged and unquarantined'
expect '1' \
  "$(in_the_database "SELECT count(*) FROM outbox
      WHERE quarantined_at IS NOT NULL AND published_at IS NULL")" \
  'a body nothing can read is quarantined rather than acknowledged'

announce 'every smoke check passed'
