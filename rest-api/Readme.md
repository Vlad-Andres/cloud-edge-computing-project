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

## How to run it

```bash
docker build -t rest-api .
docker run -d --name rest-api -p 3003:3003 -e DB_URL="postgresql://USER:PASSWORD@HOST:5432/tempobs" rest-api
```

Then open `http://localhost:3003/health`. It should show `{"status":"ok"}`.

## Files

- `app.py`: the API code
- `requirements.txt`: Python packages
- `Dockerfile`: builds the container (uses gunicorn as the server)

## What it needs from the DB

A table called `measurements` with the columns `experiment_id`, `timestamp`, `temperature` and `out_of_range`.