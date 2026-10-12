#!/bin/sh
set -eu

PROJECT_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
POSTGRES_CONTAINER=${POSTGRES_CONTAINER:-postgres}
POSTGRES_USER=${POSTGRES_USER:-groupdb}
POSTGRES_DB=${POSTGRES_DB:-groupdb}

cd "$PROJECT_ROOT"

sql() {
    echo "$(date -u '+%F %T') $1"
    docker exec "$POSTGRES_CONTAINER" psql \
        -U "$POSTGRES_USER" \
        -d "$POSTGRES_DB" \
        -v ON_ERROR_STOP=1 \
        -c "$1"
}

sql "DELETE FROM measurements
     WHERE timestamp < extract(epoch FROM now() - interval '1 days');"

sql "DELETE FROM experiments e
     WHERE e.terminated_at < extract(epoch FROM now() - interval '1 days')
       AND NOT EXISTS (SELECT 1 FROM measurements m WHERE m.experiment_id = e.experiment_id);"

sql "VACUUM (ANALYZE) measurements, experiments;"

sql "REINDEX TABLE CONCURRENTLY measurements;"

sql "SELECT pg_size_pretty(pg_database_size(current_database())) AS database_size;"
