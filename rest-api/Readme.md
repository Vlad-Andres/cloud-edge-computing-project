# REST API

Small async API that reads temperature data from our Postgres database. It is an RSGI app for granian: each worker process runs one event loop, so a request waiting for Postgres doesn't hold a thread.

Requests for the same experiment that arrive together share one database read, and the rows are reused for CACHE_TTL_MS (1 s in k8s).



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
make start

building and pushing is in k8s readme.

## Metrics
Prometheus reads two sources; the Event pipeline dashboard shows them under Bottlenecks.

- `GET /metrics` on port 3003, from the app: `rest_api_db_read_seconds` (one read of an experiment's rows), `rest_api_request_duration_seconds` (the whole request in the app, per endpoint) and `rest_api_rows_lookups_total` (rows read, shared with a read in progress, or reused). 
- Port 9091 : connections, requests handled, worker restarts.

## Benchmark

`make bench-rest` measures both endpoints one request at a time and fails if a median response is above 5 ms. For the same at a different limit or against another server:

```bash
TARGET_MS=10 scripts/bench-rest.sh http://localhost:3003 <experiment-id>
```