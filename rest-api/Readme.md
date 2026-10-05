# REST API

Small Flask API that reads temperature data from our Postgres database.
It runs in Docker on port **3003**. Read-only: it never writes to the DB.

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
- `Dockerfile`: builds the container (uses gunicorn as the server)

## What it needs from the DB

- `measurements`: one row per sensor reading, with `measurement_id`, `sensor_id`, `experiment_id`, `timestamp`, `temperature`
- `experiments`: `sensor_count`, `lower_threshold`, `upper_threshold`, `started_at`