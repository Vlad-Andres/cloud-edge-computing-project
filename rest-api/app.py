"""Temperature Observability REST API (read-only, async).

Endpoints:
  GET /temperature?experiment-id=&start-time=&end-time=
  GET /temperature/out-of-range?experiment-id=
  GET /health
"""
import os
from contextlib import asynccontextmanager

import orjson
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse, Response
from psycopg.conninfo import make_conninfo
from psycopg_pool import AsyncConnectionPool

# Same variables as the shared .env used by store-consumer
DB_CONNINFO = make_conninfo(
    host=os.environ.get("POSTGRES_HOST", "postgres"),
    port=os.environ.get("POSTGRES_PORT", "5432"),
    dbname=os.environ.get("POSTGRES_DB", "groupdb"),
    user=os.environ.get("POSTGRES_USER", "groupdb"),
    password=os.environ.get("POSTGRES_PASSWORD", ""),
)

# Connections per worker process. Total = pods x workers x this; must stay below
# Postgres' max_connections (100) together with store-consumer: 4 x 2 x 10 = 80.
POOL_MAX_SIZE = int(os.environ.get("DB_POOL_MAX_SIZE", "10"))

# Connection pool: reuse connections instead of opening one per request.
# Autocommit: a single read needs no BEGIN and COMMIT, two round trips to the database.
pool = AsyncConnectionPool(
    DB_CONNINFO,
    min_size=1,
    max_size=POOL_MAX_SIZE,
    timeout=5,
    open=False,
    kwargs={"autocommit": True},
)


@asynccontextmanager
async def lifespan(_app):
    await pool.open(wait=False)   # when the worker starts
    yield                         # the app runs here
    await pool.close()            # when the worker stops (e.g. SIGTERM from Kubernetes)


app = FastAPI(lifespan=lifespan, docs_url=None, redoc_url=None, openapi_url=None)


def error(message, status=400):
    # Same error format as the Flask version: {"error": "..."}
    return JSONResponse({"error": message}, status_code=status)


# One measurement = all sensors with the same measurement_id, and its temperature is
# their average. store-consumer stores that average when the last sensor reports, so
# reading an experiment is one index scan in time order.
AVERAGES_SQL = """
    SELECT timestamp, temperature
    FROM measurement_averages
    WHERE experiment_id = %(id)s
      AND timestamp >= (SELECT started_at FROM experiments WHERE experiment_id = %(id)s)
      {extra_where}
    ORDER BY timestamp
"""


async def points_response(extra_where, params):
    try:
        async with pool.connection() as conn:          # wait for a free connection
            # binary: the floats skip the conversion to text and back
            cur = await conn.execute(                  # wait for Postgres
                AVERAGES_SQL.format(extra_where=extra_where), params, binary=True
            )
            rows = await cur.fetchall()                # wait for the rows
    except Exception:  # pool timeout or database down
        return error("database unavailable", 503)
    points = [{"timestamp": ts, "temperature": temp} for ts, temp in rows]
    return Response(orjson.dumps(points), media_type="application/json")


@app.get("/health")
async def health():
    return {"status": "ok"}


@app.get("/temperature")
async def temperature(request: Request):
    args = request.query_params
    experiment_id = args.get("experiment-id")
    try:
        start = float(args["start-time"])
        end = float(args["end-time"])
    except (KeyError, ValueError):
        return error("start-time and end-time must be numbers")
    if not experiment_id:
        return error("experiment-id is required")

    return await points_response(
        "AND timestamp BETWEEN %(start)s AND %(end)s",
        {"id": experiment_id, "start": start, "end": end},
    )


@app.get("/temperature/out-of-range")
async def out_of_range(request: Request):
    experiment_id = request.query_params.get("experiment-id")
    if not experiment_id:
        return error("experiment-id is required")

    return await points_response("AND out_of_range", {"id": experiment_id})


# Local run without Docker:
#   granian --interface asgi --host 0.0.0.0 --port 3003 app:app