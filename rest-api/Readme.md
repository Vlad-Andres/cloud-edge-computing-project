# REST API

Small async API that reads temperature data from our Postgres database. It is an RSGI app for granian: each worker process runs one event loop, so a request waiting for Postgres doesn't hold a thread.
It runs in Docker on port **3003**. Read-only: it never writes to the DB.

Clients ask in bursts (the course's tester sends ~70 requests at once, all for one experiment), so one read serves a whole burst: requests that arrive while an experiment's rows are being read wait for that read, and the rows are reused for `CACHE_TTL_MS` (200 ms). Both endpoints are answered from the same rows.

| Setting | Default | |
|---|---|---|
| `GRANIAN_WORKERS` | 2 (Kubernetes: 1 per pod) | worker processes |
| `CACHE_TTL_MS` | 200 | how long rows are reused; answers are at most this much older than the database; 0 = only share reads in progress |
| `GZIP` | 0 (Kubernetes: 1) | compress answers for clients that send `Accept-Encoding: gzip` |

## Endpoints

- `GET /health`: checks that the API is running
- `GET /temperature?experiment-id=X&start-time=A&end-time=B`: measurements between A and B
- `GET /temperature/out-of-range?experiment-id=X`: measurements that were out of range

Example response:
```json
[{"timestamp": 1691419390.94, "temperature": 25.4}]
```

The temperature is the average of all sensors for the same `measurement_id`. Only measurements from after the experiment started, where all sensors have reported, are returned.

## How to run it

Postgres has to be running first (see `database/README.md`). Run from the project root, using the same `.env` and `cec` network as the other containers:

```bash
docker build -t rest-api-img rest-api
docker run -d --name rest-api --network cec --env-file .env -p 3003:3003 rest-api-img
```

Then open `http://localhost:3003/health`. It should show `{"status":"ok"}`.

## Files

- `app.py`: the API code
- `requirements.txt`: Python packages
- `Dockerfile`: builds the container (granian with `--interface rsgi`)

## What it needs from the DB

- `measurement_averages`: one row per complete measurement, with `experiment_id`, `timestamp`, `temperature`, `out_of_range`
- `experiments`: `started_at`

## Metrics

Prometheus reads two sources; the *Event pipeline* dashboard shows them under *Bottlenecks*:

- `GET /metrics` on port 3003, from the app: `rest_api_db_read_seconds` (one read of an experiment's rows), `rest_api_request_duration_seconds` (the whole request in the app, per endpoint), `rest_api_rows_lookups_total` (rows read, shared with a read in progress, or reused) and `rest_api_accepts_gzip_total`. The buckets include the default ones (5 ms ... 2.5 s), so they compare with a histogram measured by a client.
- Port 9091, from granian: connections, requests handled, worker restarts.

## Benchmark

`make bench-rest` measures both endpoints one request at a time and fails if a median response is above 5 ms. For the same at a different limit or against another server:

```bash
TARGET_MS=10 scripts/bench-rest.sh http://localhost:3003 <experiment-id>
```