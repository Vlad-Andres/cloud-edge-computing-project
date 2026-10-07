# Kubernetes

Runs the whole project on the cluster. Everything lives in namespace `cec` and is deployed with Kustomize (built into `kubectl`) from the repo root:

```sh
kubectl apply -k .
```

`kustomization.yaml` in the repo root lists the manifests in this folder and builds ConfigMaps/Secrets from files that already exist: `database/init.sql`, `monitoring/grafana/...`, `.env` and `auth/`. Changing one of those files and re-applying restarts the pods that use it.

| Component | Kind | Where | Reachable at |
|---|---|---|---|
| Postgres | StatefulSet + Service | control-plane node, data in `/var/lib/cec/pgdata` | `postgres:5432` (in cluster) |
| store-consumer | Deployment, 4 replicas | spread over the nodes | metrics `:9091` per pod |
| notify-consumer | Deployment, 1 replica | any node | metrics `:9091` |
| REST API | Deployment + Service | control-plane node | `http://52.209.7.51:3003` |
| Retention | CronJob, daily 03:00 | any node | |
| Prometheus | Deployment + Service | any node | `kubectl port-forward` (below) |
| Grafana | Deployment + Service | control-plane node | `http://52.209.7.51:3000` (`admin`/`admin`) |
| postgres-exporter | Deployment + Service | any node | `postgres-exporter:9187` (in cluster) |

## Requirements

On the machine where you run `kubectl` (the control-plane node), in the repo root:

- `auth/` with `ca.crt` and `kafka.keystore.pkcs12`
- `.env` with `NOTIFICATIONS_TOKEN` and `POSTGRES_DB`, `POSTGRES_USER`, `POSTGRES_PASSWORD`. `POSTGRES_HOST`/`POSTGRES_PORT` are ignored in the cluster: the pods always use `postgres:5432`.

Stop the docker containers first (`store-consumer`, `notify-consumer`, `postgres`, monitoring). Otherwise they share the Kafka consumer groups with the pods and take partitions away from them.

## Build and push the images

The cluster pulls from Docker Hub. Build for `linux/amd64` (the VMs), also when building on a Mac:

```sh
docker login

docker buildx build --platform linux/amd64 --build-arg BIN=store-consumer \
  -t vladandres/store-consumer:latest --push consumer

docker buildx build --platform linux/amd64 --build-arg BIN=notify-consumer \
  -t vladandres/notify-consumer:latest --push consumer
```

The REST API code is on the `rest-api` branch:

```sh
git worktree add /tmp/rest-api origin/rest-api
docker buildx build --platform linux/amd64 \
  -t vladandres/rest-api:latest --push /tmp/rest-api/rest-api
git worktree remove /tmp/rest-api
```

## Deploy

```sh
kubectl kustomize . > /dev/null   # check it renders
kubectl apply -k .
kubectl -n cec get pods -o wide
```

The store-consumer pods wait for Postgres (init container `wait-for-postgres`) before starting.

After pushing new images (tag `latest` is always pulled):

```sh
kubectl -n cec rollout restart deployment store-consumer notify-consumer rest-api
```

## Check

```sh
# Where the 4 store-consumers ended up
kubectl -n cec get pods -l app=store-consumer -o wide

kubectl -n cec logs -f deployment/notify-consumer
kubectl -n cec logs -f -l app=store-consumer --prefix

curl http://52.209.7.51:3003/health

# Database
kubectl -n cec exec -it postgres-0 -- sh -c 'psql -U "$POSTGRES_USER" -d "$POSTGRES_DB"'

# Prometheus UI on http://localhost:9090 of the machine running this
kubectl -n cec port-forward svc/prometheus 9090:9090
```

## Retention

Runs every day at 03:00 and deletes measurements older than one day, then `VACUUM (ANALYZE)`. Run it now:

```sh
kubectl -n cec create job --from=cronjob/retention retention-manual
kubectl -n cec logs -f job/retention-manual
kubectl -n cec delete job retention-manual
```

## Tear down

```sh
kubectl delete -k .
```

The Postgres data stays in `/var/lib/cec/pgdata` on the control-plane node. Delete that directory for a clean database; `init.sql` only runs when it is empty.

## Notes

- Postgres, the REST API and Grafana are pinned with the `node-role.kubernetes.io/control-plane` label, so this assumes the control-plane node is the one with IP `52.209.7.51` (`group16`).
- The REST API on the `rest-api` branch expects a different `measurements` schema (`sensor_id`, one row per sensor) than `database/init.sql` on `main`. The pod starts, but its queries may fail until the two match.
