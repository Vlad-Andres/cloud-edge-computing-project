"""Temperature Observability REST API (read-only).

Endpoints:
  GET /temperature?experiment-id=&start-time=&end-time=
  GET /temperature/out-of-range?experiment-id=
  GET /health
"""
import os

from flask import Flask, jsonify, request
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
pool = ConnectionPool(DB_CONNINFO, min_size=1, max_size=2, timeout=5, open=False)


def get_pool():
    if pool.closed:
        pool.open()
    return pool


# The measurements table has one row per sensor. One measurement = all sensors with
# the same measurement_id, and its temperature is their average.
AVERAGES_SQL = """
    SELECT MIN(m.timestamp) AS timestamp, AVG(m.temperature) AS temperature
    FROM measurements m
    WHERE m.experiment_id = %(id)s
      AND m.timestamp >= (SELECT started_at FROM experiments WHERE experiment_id = %(id)s)
      {extra_where}
    GROUP BY m.measurement_id
    HAVING COUNT(*) = (SELECT sensor_count FROM experiments WHERE experiment_id = %(id)s)
      {extra_having}
    ORDER BY timestamp
"""


def rows_to_points(rows):
    return [{"timestamp": ts, "temperature": temp} for ts, temp in rows]


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

    with get_pool().connection() as conn:
        rows = conn.execute(
            AVERAGES_SQL.format(
                extra_where="AND m.timestamp BETWEEN %(start)s AND %(end)s",
                extra_having="",
            ),
            {"id": experiment_id, "start": start, "end": end},
        ).fetchall()
    return jsonify(rows_to_points(rows))


@app.get("/temperature/out-of-range")
def out_of_range():
    experiment_id = request.args.get("experiment-id")
    if not experiment_id:
        return jsonify({"error": "experiment-id is required"}), 400

    with get_pool().connection() as conn:
        rows = conn.execute(
            AVERAGES_SQL.format(
                extra_where="",
                 extra_having="""AND AVG(m.temperature) NOT BETWEEN
                      (SELECT lower_threshold FROM experiments WHERE experiment_id = %(id)s)
                  AND (SELECT upper_threshold FROM experiments WHERE experiment_id = %(id)s)""",
            ),
            {"id": experiment_id},
        ).fetchall()
    return jsonify(rows_to_points(rows))


if __name__ == "__main__":
    app.run(host="0.0.0.0", port=3003, debug=True)