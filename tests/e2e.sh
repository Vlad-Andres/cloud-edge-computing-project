#!/usr/bin/env bash
# End-to-end recovery test.
#
# Runs several experiment producers against the group topic, optionally injects
# a failure mid-run, and checks that every experiment and measurement reached
# Postgres. Uses its own containers (e2e-postgres, e2e-store-consumer), so the
# real stack is never touched.
#
# Usage (from anywhere, on the server):
#   tests/e2e.sh baseline     # no failure
#   tests/e2e.sh kill-store   # kill the store-consumer, start it again later
#   tests/e2e.sh restart-db   # restart Postgres mid-run
#
# Needs: .env and auth/ in the project root, image store-consumer-img,
# docker network cec. Every setting below can be overridden from the env,
# e.g. WARMUP=0 tests/e2e.sh baseline
set -euo pipefail

SCENARIO=${1:-}
case "$SCENARIO" in
    baseline|kill-store|restart-db) ;;
    *) echo "Usage: $0 <baseline|kill-store|restart-db>" >&2; exit 2 ;;
esac

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

TOPIC=${TOPIC:-group16}
GROUP=${GROUP:-group16-e2e-store}
BROKERS=${BROKERS:-kafka.cec.dlandau.nl:19092,kafka.cec.dlandau.nl:29092,kafka.cec.dlandau.nl:39092}
PRODUCER_IMAGE=${PRODUCER_IMAGE:-dclandau/cec-experiment-producer:latest}
CONSUMER_IMAGE=${CONSUMER_IMAGE:-store-consumer-img}

# Load per producer: 5 sensors every 200 ms = 25 events/s, 300 samples = ~60 s
PRODUCERS=${PRODUCERS:-3}
NUM_SENSORS=${NUM_SENSORS:-5}
SAMPLE_RATE=${SAMPLE_RATE:-200}
STABILIZATION_SAMPLES=${STABILIZATION_SAMPLES:-20}
CARRY_OUT_SAMPLES=${CARRY_OUT_SAMPLES:-280}

WARMUP=${WARMUP:-20}           # s to wait for partition assignment before producing
FAIL_AT=${FAIL_AT:-20}         # s after producers start
RECOVER_AT=${RECOVER_AT:-40}   # s after producers start (kill-store only)
SETTLE=${SETTLE:-30}           # s without new rows before checking
TIMEOUT=${TIMEOUT:-600}        # s max to wait for the drain

PG=e2e-postgres
PG_VOLUME=e2e-pgdata
CONSUMER=e2e-store-consumer
RUN_ID="e2e-$(date +%s)"
TMP=$(mktemp -d)

log() { printf '[%s] %s\n' "$(date +%T)" "$*"; }
die() { log "ERROR: $*" >&2; exit 2; }

producer_names() {
    for i in $(seq 1 "$PRODUCERS"); do echo "e2e-producer-$i"; done
}

cleanup() {
    # Stop but keep the containers so `docker logs` still works after the run
    docker stop -t 2 "$CONSUMER" $(producer_names) >/dev/null 2>&1 || true
    rm -rf "$TMP"
}
trap cleanup EXIT

# --- Preflight ---------------------------------------------------------------

[ -f .env ] || die ".env missing in $ROOT"
[ -d auth ] || die "auth/ missing in $ROOT"
docker image inspect "$CONSUMER_IMAGE" >/dev/null 2>&1 \
    || die "image $CONSUMER_IMAGE missing, build it (see consumer/README.md)"
docker network inspect cec >/dev/null 2>&1 \
    || die "docker network cec missing: docker network create cec"

set -a
. ./.env
set +a
: "${POSTGRES_DB:?POSTGRES_DB missing in .env}"
: "${POSTGRES_USER:?POSTGRES_USER missing in .env}"
: "${POSTGRES_PASSWORD:?POSTGRES_PASSWORD missing in .env}"

sql() {
    docker exec "$PG" psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -At -F '|' -c "$1"
}

# --- Steps -------------------------------------------------------------------

start_db() {
    docker rm -f "$PG" >/dev/null 2>&1 || true
    docker volume rm "$PG_VOLUME" >/dev/null 2>&1 || true
    docker run -d --name "$PG" --network cec \
        -e POSTGRES_DB -e POSTGRES_USER -e POSTGRES_PASSWORD \
        -v "$ROOT/database/init.sql":/docker-entrypoint-initdb.d/init.sql:ro \
        -v "$PG_VOLUME":/var/lib/postgresql/data \
        postgres:16-alpine >/dev/null
    wait_for_db
}

wait_for_db() {
    # TCP check: the entrypoint's temporary init server only listens on the socket
    for _ in $(seq 1 60); do
        if docker exec "$PG" pg_isready -q -h 127.0.0.1 -U "$POSTGRES_USER" -d "$POSTGRES_DB"; then
            return 0
        fi
        sleep 1
    done
    die "$PG not ready after 60 s"
}

start_consumer() {
    docker rm -f "$CONSUMER" >/dev/null 2>&1 || true
    docker run -d --init --name "$CONSUMER" --network cec \
        --env-file .env -e POSTGRES_HOST="$PG" \
        -v "$ROOT/auth":/usr/src/app/auth:ro \
        "$CONSUMER_IMAGE" "$TOPIC" "$GROUP" >/dev/null
}

start_producers() {
    for i in $(seq 1 "$PRODUCERS"); do
        cat > "$TMP/p$i.json" <<EOF
[
  {
    "start_time": 0,
    "researcher": "$RUN_ID-p$i@test.invalid",
    "num_sensors": $NUM_SENSORS,
    "sample_rate": $SAMPLE_RATE,
    "start_temperature": 20,
    "stabilization_samples": $STABILIZATION_SAMPLES,
    "carry_out_samples": $CARRY_OUT_SAMPLES,
    "temp_range": {
      "lower_threshold": 18,
      "upper_threshold": 22
    }
  }
]
EOF
        docker rm -f "e2e-producer-$i" >/dev/null 2>&1 || true
        docker run -d --init --platform linux/amd64 --name "e2e-producer-$i" \
            -v "$ROOT/auth":/experiment-producer/auth:ro \
            -v "$TMP/p$i.json":/config.json:ro \
            "$PRODUCER_IMAGE" \
            --brokers "$BROKERS" --topic "$TOPIC" --config-file /config.json >/dev/null
    done
}

sleep_until() {
    local target=$((T0 + $1)) now
    now=$(date +%s)
    if (( target > now )); then sleep $((target - now)); fi
}

inject_failure() {
    case "$SCENARIO" in
        baseline) ;;
        kill-store)
            sleep_until "$FAIL_AT"
            log "FAILURE: docker kill $CONSUMER"
            docker kill "$CONSUMER" >/dev/null || log "WARNING: $CONSUMER was not running"
            sleep_until "$RECOVER_AT"
            log "RECOVER: starting $CONSUMER again (same group $GROUP)"
            start_consumer
            ;;
        restart-db)
            sleep_until "$FAIL_AT"
            log "FAILURE: docker restart $PG"
            docker restart "$PG" >/dev/null
            wait_for_db
            log "RECOVER: $PG is back"
            ;;
    esac
}

wait_for_producers() {
    local codes
    codes=$(docker wait $(producer_names) | tr '\n' ' ')
    log "producers exited with codes: $codes"
    for code in $codes; do
        [ "$code" = 0 ] || log "WARNING: a producer failed, see docker logs e2e-producer-<n>"
    done
}

run_measurements() {
    sql "SELECT count(*) FROM measurements m JOIN experiments e USING (experiment_id)
         WHERE e.researcher LIKE '$RUN_ID-%'" 2>/dev/null || echo "?"
}

wait_for_drain() {
    local start last="" stable_since now count
    start=$(date +%s)
    stable_since=$start
    while true; do
        count=$(run_measurements)
        now=$(date +%s)
        if [ "$count" != "$last" ]; then
            log "measurements stored so far: $count"
            last=$count
            stable_since=$now
        elif (( now - stable_since >= SETTLE )); then
            log "no new measurements for ${SETTLE} s, checking"
            return 0
        fi
        if (( now - start >= TIMEOUT )); then
            log "WARNING: drain timeout after ${TIMEOUT} s, checking anyway"
            return 0
        fi
        sleep 5
    done
}

check() {
    local expected=$((STABILIZATION_SAMPLES + CARRY_OUT_SAMPLES)) failed=0
    local researcher row phase stages count result

    echo
    printf '%-34s %-13s %-7s %-17s %s\n' RESEARCHER PHASE STAGES "MEASUREMENTS" RESULT
    for i in $(seq 1 "$PRODUCERS"); do
        researcher="$RUN_ID-p$i@test.invalid"
        row=$(sql "SELECT e.phase,
                          e.stabilizing_at IS NOT NULL AND e.started_at IS NOT NULL
                              AND e.terminated_at IS NOT NULL,
                          (SELECT count(*) FROM measurements m
                           WHERE m.experiment_id = e.experiment_id)
                   FROM experiments e WHERE e.researcher = '$researcher'" || true)
        if [ -z "$row" ]; then
            phase=missing stages=- count=0
        else
            IFS='|' read -r phase stages count <<< "$row"
        fi
        if [ "$phase" = terminated ] && [ "$stages" = t ] && [ "$count" = "$expected" ]; then
            result=PASS
        else
            result=FAIL
            failed=1
        fi
        printf '%-34s %-13s %-7s %-17s %s\n' \
            "$researcher" "$phase" "$stages" "$count/$expected" "$result"
    done
    echo

    if (( failed )); then
        log "RESULT: FAIL ($SCENARIO, run $RUN_ID)"
        log "inspect: docker logs $CONSUMER | grep -i error ; docker exec -it $PG psql -U $POSTGRES_USER -d $POSTGRES_DB"
        return 1
    fi
    log "RESULT: PASS ($SCENARIO, run $RUN_ID)"
}

# --- Run ---------------------------------------------------------------------

log "run $RUN_ID, scenario $SCENARIO, $PRODUCERS producers on topic $TOPIC"
log "starting fresh $PG"
start_db
log "starting $CONSUMER (group $GROUP), waiting ${WARMUP} s for partition assignment"
start_consumer
sleep "$WARMUP"

log "starting $PRODUCERS producers"
start_producers
T0=$(date +%s)

inject_failure
wait_for_producers
wait_for_drain
check
