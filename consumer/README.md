# kafka-consumer

Two consumers, two images:

- `store-consumer-img`: writes every event to the database
- `notify-consumer-img`: sends `Stabilized` / `OutOfRange` to the notifications service

## Requirements

- Docker
- `auth/` in the project root: the group credentials folder (`ca.crt`, `kafka.keystore.pkcs12`, `token`)
- `.env` in the project root, see [.env](#env)


## How partitioning works

```
topic "experiment"       P0   P1   P2   P3
                          │    │    │    │
group "store"  (2 pods):  A ◄─P0,P1     B ◄─P2,P3      → work split
group "notify" (1 pod):   C ◄─P0,P1,P2,P3              → full copy
```
___
## Build

```bash
docker build --build-arg BIN=store-consumer -t store-consumer-img consumer
```

```bash
docker build --build-arg BIN=notify-consumer -t notify-consumer-img consumer
```

## Run

```bash
docker run \
    --rm \
    -d \
    --init \
    --name store-consumer \
    --env-file .env \
    -v "$(pwd)/auth":/usr/src/app/auth \
    store-consumer-img "group16" "<consumer-group>"
```

```bash
docker run \
    --rm \
    -d \
    --init \
    --name notify-consumer \
    --env-file .env \
    -v "$(pwd)/auth":/usr/src/app/auth \
    notify-consumer-img "group16" "<consumer-group>"
```

| Argument | Value |
|---|---|
| `<topic>` | `experiment` (assignment, read-only) or your group topic |
| `<consumer-group>` | one per consumer type, shared by its containers, e.g. `group16-store`, `group16-notify` |
| `-b, --brokers <list>` | optional, default `kafka.cec.dlandau.nl:19092,kafka.cec.dlandau.nl:29092,kafka.cec.dlandau.nl:39092` |
| `--token <token>` | notify-consumer only, default `NOTIFICATIONS_TOKEN` from `.env` |
| `--notifications-url <url>` | notify-consumer only, default `https://notifications.cec.dlandau.nl/api/notify` |

```bash
docker logs -f store-consumer
```

```bash
docker logs -f notify-consumer
```

- store-consumer prints every event (until the TODOs in `store.rs` write to the database)
- notify-consumer prints `OutOfRange <measurement_id> -> 200 OK <latency in s>` per notification

```bash
docker stop store-consumer notify-consumer
```

## .env

Hidden file in the project root (`ls -a`), passed to both containers with `--env-file .env`. Create it with the token from `auth/token`:

```bash
printf 'NOTIFICATIONS_TOKEN=%s\n\nPOSTGRES_HOST=\nPOSTGRES_PORT=\nPOSTGRES_DB=\nPOSTGRES_USER=\nPOSTGRES_PASSWORD=\n' "$(cat auth/token)" > .env
```

| Variable | Used by | Value |
|---|---|---|
| `NOTIFICATIONS_TOKEN` | notify-consumer | contents of `auth/token` |
| `POSTGRES_*` | store-consumer | database connection — schema is set up (see [`../database/README.md`](../database/README.md)), store-consumer doesn't read these yet |

- Keep `.env` and `auth/` out of git: they hold the token and the group keys.
- Postgres in a container on the same host: `POSTGRES_HOST` is the DB container name, not `localhost`, and both containers need the same network:

```bash
docker network create cec
```

Then add `--network cec` to both `docker run` commands.



## Test data with fake producer

```bash
docker run \
    --rm \
    -it \
    --platform linux/amd64 \
    --name producer \
    -v "$(pwd)/auth":/experiment-producer/auth \
    -v "$(pwd)/loads/1.json":/config.json \
    dclandau/cec-experiment-producer \
    -b kafka.cec.dlandau.nl:19092 --config-file /config.json --topic "<group-topic>"
```

- Writes only to your group topic (`experiment` is read-only).
- Only for store-consumer: notifications for fake data can be rejected by the notifications service and count as errors in Grafana.


## Test with real producer

```bash
docker run \
    --rm \
    -d \
    --init \
    --network cec \
    --name store-consumer \
    --env-file .env \
    -v "$(pwd)/auth":/usr/src/app/auth \
    store-consumer-img "experiment" "cg-1"
```

```bash
docker run \
    --rm \
    -d \
    --init \
    --name notify-consumer \
    --env-file .env \
    -v "$(pwd)/auth":/usr/src/app/auth \
    notify-consumer-img "experiment" "notify-consumer"
```
