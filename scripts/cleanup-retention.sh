#!/bin/sh
set -eu

PROJECT_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
POSTGRES_CONTAINER=${POSTGRES_CONTAINER:-postgres}
POSTGRES_USER=${POSTGRES_USER:-groupdb}
POSTGRES_DB=${POSTGRES_DB:-groupdb}

cd "$PROJECT_ROOT"

docker exec "$POSTGRES_CONTAINER" psql \
    -U "$POSTGRES_USER" \
    -d "$POSTGRES_DB" \
    -v ON_ERROR_STOP=1 \
    -c "DELETE FROM measurements
        WHERE timestamp < extract(epoch FROM now() - interval '2 days');"

docker exec "$POSTGRES_CONTAINER" psql \
    -U "$POSTGRES_USER" \
    -d "$POSTGRES_DB" \
    -v ON_ERROR_STOP=1 \
    -c "VACUUM (ANALYZE) measurements;"
