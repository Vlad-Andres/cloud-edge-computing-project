# database
Postgres database for the Temperature Observability microservice. `init.sql` defines three tables:

- `experiments`: config + lifecycle phase per experiment
- `measurements`: one row per sensor reading
- `measurement_averages`: what the REST API reads from, one row per measurement `store-consumer` writes it when the last sensor of a measurement reports

## run
make postgres-up



## synchronous_commit

`make start` sets `synchronous_commit = off` for the database. To undo it, reset it and restart store-consumer.

```bash
docker exec postgres psql -U groupdb -d groupdb -c 'ALTER DATABASE groupdb RESET synchronous_commit'
kubectl -n cec rollout restart deploy/store-consumer
```

## .env

```
POSTGRES_HOST=postgres
POSTGRES_PORT=5432
POSTGRES_DB=groupdb
POSTGRES_USER=groupdb
POSTGRES_PASSWORD=admin
```


## How to check the size that db is occupying
```sh
docker system df -v | grep -A 2 pgdata
```
