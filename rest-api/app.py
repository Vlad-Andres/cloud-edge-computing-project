"""Temperature Observability REST API (read-only).

Endpoints:
  GET /temperature?experiment-id=&start-time=&end-time=
  GET /temperature/out-of-range?experiment-id=
  GET /health
"""
import os

import orjson
from flask import Flask, Response, jsonify, request
from psycopg.conninfo import make_conninfo
from psycopg_pool import ConnectionPool

# Same variables as the shared .env used by store-consumer
DB_CONNINFO = make_conninfo(
    host=os.environ.get("POSTGRES_HOST", "postgres"),
    port=os.environ.get("POSTGRES_PORT", "5432"),
    dbname=os.environ.get("POSTGRES_DB", "groupdb"),
    user=os.environ.get("POSTGRES_USER", "groupdb"),
    password=os.environ.get("POSTGRES_PASSWORD", ""),
)

app = Flask(__name__)
# Connection pool: reuse connections instead of opening one per request.
# Autocommit: a single read needs no BEGIN and COMMIT, two round trips to the database.
pool = ConnectionPool(
    DB_CONNINFO, min_size=1, max_size=2, timeout=5, open=False, kwargs={"autocommit": True}
)


def get_pool():
    if pool.closed:
        pool.open()
    return pool


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


def points_response(extra_where, params):
    with get_pool().connection() as conn:
        # binary: the floats skip the conversion to text and back
        rows = conn.execute(
            AVERAGES_SQL.format(extra_where=extra_where), params, binary=True
        ).fetchall()
    points = [{"timestamp": ts, "temperature": temp} for ts, temp in rows]
    return Response(orjson.dumps(points), mimetype="application/json")


@app.get("/health")
def health():
    return jsonify({"status": "ok"})


@app.get("/temperature")
def temperature():
    experiment_id = request.args.get("experiment-id")
    try:
        start = float(request.args["start-time"])
        end = float(request.args["end-time"])
    except (KeyError, ValueError):
        return jsonify({"error": "start-time and end-time must be numbers"}), 400
    if not experiment_id:
        return jsonify({"error": "experiment-id is required"}), 400

    return points_response(
        "AND timestamp BETWEEN %(start)s AND %(end)s",
        {"id": experiment_id, "start": start, "end": end},
    )


@app.get("/temperature/out-of-range")
def out_of_range():
    experiment_id = request.args.get("experiment-id")
    if not experiment_id:
        return jsonify({"error": "experiment-id is required"}), 400

    return points_response("AND out_of_range", {"id": experiment_id})


if __name__ == "__main__":
    app.run(host="0.0.0.0", port=3003, debug=True)