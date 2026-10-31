#!/bin/sh
# Response time of both REST endpoints, one request at a time, so it shows how long a
# request takes rather than how requests queue. Fails if a median is above TARGET_MS.
#
#   scripts/bench-rest.sh <base-url> <experiment-id>
set -eu

URL=$1
EXPERIMENT=$2
TARGET_MS=${TARGET_MS:-5}
REQUESTS=${REQUESTS:-500}
DOCKER_NETWORK=${DOCKER_NETWORK:-bridge}

failed=0

measure() {
    name=$1
    output=$(docker run --rm --network "$DOCKER_NETWORK" williamyeh/hey -n "$REQUESTS" -c 1 -q 50 "$2")
    p50=$(echo "$output" | awk '/ 50% in /{printf "%.2f", $3 * 1000}')
    p95=$(echo "$output" | awk '/ 95% in /{printf "%.2f", $3 * 1000}')
    ok=$(echo "$output" | awk '/\[200\]/{print $2}')
    printf '%-14s p50 %6s ms   p95 %6s ms   %s/%s OK\n' "$name" "$p50" "$p95" "${ok:-0}" "$REQUESTS"
    if [ "${ok:-0}" != "$REQUESTS" ] || ! awk "BEGIN {exit !($p50 <= $TARGET_MS)}"; then
        failed=1
    fi
}

measure temperature "$URL/temperature?experiment-id=$EXPERIMENT&start-time=0&end-time=9999999999"
measure out-of-range "$URL/temperature/out-of-range?experiment-id=$EXPERIMENT"

if [ "$failed" -ne 0 ]; then
    echo "FAILED: a median is above ${TARGET_MS} ms or a request failed"
    exit 1
fi
echo "OK: both medians are within ${TARGET_MS} ms"
