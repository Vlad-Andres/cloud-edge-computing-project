"""Temperature Observability REST API (read-only).

Endpoints:
  GET /temperature?experiment-id=&start-time=&end-time=
  GET /temperature/out-of-range?experiment-id=
  GET /health
"""
import os

from flask import Flask, jsonify, request
from psycopg_pool import ConnectionPool

DB_URL = os.environ.get("DB_URL", "postgresql://api_reader:changeme@localhost:5432/tempobs")

app = Flask(__name__)
# Connection pool: reuse connections instead of opening one per request.
pool = ConnectionPool(DB_URL, min_size=1, max_size=2, open=False)


def get_pool():
    if pool.closed:
        pool.open()
    return pool


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

    # %s placeholders = parameterized query (no SQL injection).
    with get_pool().connection() as conn:
        rows = conn.execute(
            """SELECT timestamp, temperature FROM measurements
               WHERE experiment_id = %s AND timestamp BETWEEN %s AND %s
               ORDER BY timestamp""",
            (experiment_id, start, end),
        ).fetchall()
    return jsonify(rows_to_points(rows))


@app.get("/temperature/out-of-range")
def out_of_range():
    experiment_id = request.args.get("experiment-id")
    if not experiment_id:
        return jsonify({"error": "experiment-id is required"}), 400

    with get_pool().connection() as conn:
        rows = conn.execute(
            """SELECT timestamp, temperature FROM measurements
               WHERE experiment_id = %s AND out_of_range
               ORDER BY timestamp""",
            (experiment_id,),
        ).fetchall()
    return jsonify(rows_to_points(rows))


if __name__ == "__main__":
    # Local development only; the container uses gunicorn.
    app.run(host="0.0.0.0", port=3003, debug=True)