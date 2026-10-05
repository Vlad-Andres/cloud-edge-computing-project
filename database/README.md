# database

Postgres database for the Temperature Observability microservice. `init.sql` defines two tables:

- `experiments`: config + lifecycle phase per experiment. Probably won't be read by the REST API — this exists so `store-consumer` can recover an experiment's state after a restart instead of relying only on in-memory tracking. Basically added recovery method for data.
- `measurements`: what the REST API reads from (`experiment_id`, `timestamp`, `temperature`, `out_of_range`), one row per averaged sensor reading during the carry-out phase.

## Run

```bash
docker network create cec   # if not already created

docker run \
    --rm -d \
    --name postgres \
    --network cec \
    -e POSTGRES_DB=groupdb \
    -e POSTGRES_USER=groupdb \
    -e POSTGRES_PASSWORD=admin \
    -v "$(pwd)/init.sql":/docker-entrypoint-initdb.d/init.sql \
    -v pgdata:/var/lib/postgresql/data \
    -p 5432:5432 \
    postgres:16-alpine
```

- `POSTGRES_DB` / `POSTGRES_USER` / `POSTGRES_PASSWORD` read by the image's entrypoint on first boot to create the database and role. 
- `init.sql` only runs once, against an empty data directory. After changing the schema, `docker rm -f postgres && docker volume rm pgdata` before re-running the command above, or apply the change manually with `psql`.
- `pgdata` is a Docker-managed named volume, so data survives container restarts - `docker volume inspect pgdata`.
- `--network cec` must match the network of `store-consumer` and later the REST API so they can reach this container by name (`POSTGRES_HOST=postgres`).

```bash
docker exec -it postgres psql -U groupdb -d groupdb -c '\dt'
```

```bash
docker stop postgres
```

## .env

```
POSTGRES_HOST=postgres
POSTGRES_PORT=5432
POSTGRES_DB=groupdb
POSTGRES_USER=groupdb
POSTGRES_PASSWORD=admin
```
