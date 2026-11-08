# consumers

Two binaries from one crate, both read Kafka and serve Prometheus metrics

- `store-consumer`: writes every event to Postgres (`POSTGRES_HOST`, `POSTGRES_PORT`, `POSTGRES_DB`, `POSTGRES_USER`, `POSTGRES_PASSWORD`)
- `notify-consumer`: sends `Stabilized` / `OutOfRange` to the notifications service (`NOTIFICATIONS_TOKEN`)

The env vars come from `.env` through the `app-env` kustomization.yaml

Building, pushing and deploying the images /k8s/README.md
