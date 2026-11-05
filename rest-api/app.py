"""Temperature Observability REST API (read-only).

Endpoints:
  GET /temperature?experiment-id=&start-time=&end-time=
  GET /temperature/out-of-range?experiment-id=
  GET /health
  GET /metrics (Prometheus)

An RSGI app for granian (--interface rsgi): each worker process runs an event loop,
so a request waiting for Postgres doesn't hold a thread and a burst is served at once.
"""
import asyncio
import gzip
import math
import os
import sys
import time
import traceback
from bisect import bisect_left, bisect_right
from urllib.parse import parse_qs

import orjson
from prometheus_client import (
    CONTENT_TYPE_LATEST,
    REGISTRY,
    CollectorRegistry,
    Counter,
    Histogram,
    generate_latest,
    multiprocess,
)
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

# Connection pool: reuse connections instead of opening one per request.
# Autocommit: a single read needs no BEGIN and COMMIT, two round trips to the database.
pool = AsyncConnectionPool(
    DB_CONNINFO, min_size=1, max_size=4, timeout=5, open=False, kwargs={"autocommit": True}
)
pool_lock = asyncio.Lock()


async def get_pool():
    if pool.closed:
        async with pool_lock:
            if pool.closed:
                await pool.open()
    return pool


# Every granian worker is its own process: each writes its values to
# PROMETHEUS_MULTIPROC_DIR (set in the Dockerfile) and /metrics adds them up.
# Same buckets as store-consumer's DB histogram; they include the default ones
# (5 ms ... 2.5 s), so the counts compare with a client-side histogram.
BUCKETS = (0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0)
DB_READ = Histogram(
    "rest_api_db_read_seconds",
    "Taking a pooled connection, running the query and fetching the rows",
    ["endpoint"],
    buckets=BUCKETS,
)
REQUEST = Histogram(
    "rest_api_request_duration_seconds",
    "Time in the app per request, from the request to the finished JSON body",
    ["endpoint"],
    buckets=BUCKETS,
)
ROWS = Counter(
    "rest_api_rows_lookups_total",
    "Data requests by where their rows came from: read (from Postgres), "
    "shared (another request's read in progress) or cached",
    ["source"],
)
ACCEPTS_GZIP = Counter(
    "rest_api_accepts_gzip_total",
    "Data requests by whether the client accepts gzip (Accept-Encoding)",
    ["gzip"],
)

# GZIP=1: compress responses for clients that accept it, a ~23 KB answer becomes
# ~7 KB. The full answers are compressed once per read, like their JSON.
GZIP = os.environ.get("GZIP", "0") == "1"
GZIP_MIN_BYTES = 1400  # smaller bodies fit in one packet anyway


def accepts_gzip(header):
    for part in header.split(","):
        name, _, params = part.partition(";")
        if name.strip().lower() in ("gzip", "*"):
            q = params.strip().lower()
            return not (q.startswith("q=") and float(q[2:] or 0) == 0)
    return False


def maybe_compress(plain, gzip_ok):
    """(body, gzipped): compressed when the client accepts it and it is worth it."""
    if gzip_ok and len(plain) >= GZIP_MIN_BYTES:
        return gzip.compress(plain, compresslevel=1, mtime=0), True
    return plain, False

# One measurement = all sensors with the same measurement_id, and its temperature is
# their average. store-consumer stores that average when the last sensor reports, so
# reading an experiment is one index scan in time order. Both endpoints are answered
# from these rows.
ROWS_SQL = """
    SELECT timestamp, temperature, out_of_range
    FROM measurement_averages
    WHERE experiment_id = %(id)s
      AND timestamp >= (SELECT started_at FROM experiments WHERE experiment_id = %(id)s)
    ORDER BY timestamp
"""

# Clients ask in bursts (the course's tester sends ~70 requests at once, all for one
# experiment), so one read serves the whole burst: requests that arrive while a read
# is running wait for it, and its rows are reused for CACHE_TTL_MS. Answers are at
# most that much older than the database; 0 turns the reuse off (sharing stays).
CACHE_TTL = float(os.environ.get("CACHE_TTL_MS", "200")) / 1000
MAX_CACHED = 256
cached = {}  # experiment_id -> Rows
reading = {}  # experiment_id -> Future of the read in progress


class Rows:
    __slots__ = ("read_at", "timestamps", "points", "bodies", "out_of_range_json")

    def __init__(self, rows):
        self.read_at = time.monotonic()
        self.timestamps = [ts for ts, _, _ in rows]
        self.points = [{"timestamp": ts, "temperature": temp} for ts, temp, _ in rows]
        # name -> (plain body, (body, gzipped) for clients that accept gzip), built
        # once for the answers every request of a burst shares
        self.bodies = {}
        self.out_of_range_json = orjson.dumps(
            [p for p, (_, _, out) in zip(self.points, rows) if out]
        )

    def shared(self, name, gzip_ok, make):
        if name not in self.bodies:
            plain = make()
            self.bodies[name] = (plain, maybe_compress(plain, True))
        plain, compressed = self.bodies[name]
        return compressed if gzip_ok else (plain, False)

    def between(self, start, end, gzip_ok):
        lo = bisect_left(self.timestamps, start)
        hi = bisect_right(self.timestamps, end)
        if lo == 0 and hi == len(self.points):
            return self.shared("all", gzip_ok, lambda: orjson.dumps(self.points))
        return maybe_compress(orjson.dumps(self.points[lo:hi]), gzip_ok)

    def out_of_range(self, gzip_ok):
        return self.shared("out_of_range", gzip_ok, lambda: self.out_of_range_json)


async def read_rows(experiment_id):
    with DB_READ.labels("experiment").time():
        async with (await get_pool()).connection() as conn:
            # binary: the floats skip the conversion to text and back
            cur = await conn.execute(ROWS_SQL, {"id": experiment_id}, binary=True)
            return Rows(await cur.fetchall())


async def experiment_rows(experiment_id):
    rows = cached.get(experiment_id)
    if rows is not None and time.monotonic() - rows.read_at < CACHE_TTL:
        ROWS.labels("cached").inc()
        return rows
    future = reading.get(experiment_id)
    if future is not None:
        ROWS.labels("shared").inc()
        return await future
    ROWS.labels("read").inc()
    future = asyncio.get_running_loop().create_future()
    reading[experiment_id] = future
    try:
        rows = await read_rows(experiment_id)
        future.set_result(rows)
    except Exception as error:
        future.set_exception(error)
        # Marks the error as seen when no other request was waiting for it
        future.exception()
        raise
    finally:
        del reading[experiment_id]
    if CACHE_TTL > 0:
        if len(cached) >= MAX_CACHED:
            cached.clear()
        cached[experiment_id] = rows
    return rows


def number(value):
    # NaN as in Postgres, where it sorts above every number
    parsed = float(value)
    return math.inf if math.isnan(parsed) else parsed


JSON = [("content-type", "application/json")]
JSON_VARY = JSON + [("vary", "accept-encoding")]
JSON_GZIP = JSON_VARY + [("content-encoding", "gzip")]


def json_error(proto, status, message):
    proto.response_bytes(status, JSON, orjson.dumps({"error": message}) + b"\n")


def send_points(proto, body, gzipped):
    if gzipped:
        proto.response_bytes(200, JSON_GZIP, body)
    else:
        proto.response_bytes(200, JSON_VARY if GZIP else JSON, body)


async def temperature(proto, args, gzip_ok):
    experiment_id = args.get("experiment-id", [None])[0]
    try:
        start = number(args["start-time"][0])
        end = number(args["end-time"][0])
    except (KeyError, ValueError):
        return json_error(proto, 400, "start-time and end-time must be numbers")
    if not experiment_id:
        return json_error(proto, 400, "experiment-id is required")
    rows = await experiment_rows(experiment_id)
    send_points(proto, *rows.between(start, end, gzip_ok))


async def out_of_range(proto, args, gzip_ok):
    experiment_id = args.get("experiment-id", [None])[0]
    if not experiment_id:
        return json_error(proto, 400, "experiment-id is required")
    rows = await experiment_rows(experiment_id)
    send_points(proto, *rows.out_of_range(gzip_ok))


DATA_ENDPOINTS = {"/temperature": temperature, "/temperature/out-of-range": out_of_range}


def metrics(proto):
    registry = REGISTRY
    # Without the directory (a single process) there is only this process
    if "PROMETHEUS_MULTIPROC_DIR" in os.environ:
        registry = CollectorRegistry()
        multiprocess.MultiProcessCollector(registry)
    proto.response_bytes(200, [("content-type", CONTENT_TYPE_LATEST)], generate_latest(registry))


async def app(scope, proto):
    if scope.proto != "http":
        return
    started = time.perf_counter()
    path = scope.path
    try:
        if scope.method not in ("GET", "HEAD"):
            proto.response_bytes(405, JSON, b'{"error":"method not allowed"}\n')
        elif path in DATA_ENDPOINTS:
            wants_gzip = accepts_gzip(scope.headers.get("accept-encoding", ""))
            ACCEPTS_GZIP.labels("yes" if wants_gzip else "no").inc()
            await DATA_ENDPOINTS[path](
                proto, parse_qs(scope.query_string, keep_blank_values=True), GZIP and wants_gzip
            )
            name = "temperature" if path == "/temperature" else "out_of_range"
            REQUEST.labels(name).observe(time.perf_counter() - started)
        elif path == "/health":
            proto.response_bytes(200, JSON, b'{"status":"ok"}\n')
        elif path == "/metrics":
            metrics(proto)
        else:
            proto.response_bytes(404, JSON, b'{"error":"not found"}\n')
    except Exception:
        traceback.print_exc(file=sys.stderr)
        proto.response_bytes(500, JSON, b'{"error":"internal error"}\n')
