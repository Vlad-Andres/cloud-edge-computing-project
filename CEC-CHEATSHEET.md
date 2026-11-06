# CEC cheat sheet: how it works, where it's configured, why

State: branch `exp/rest-api-bursts` (dd6c850), as deployed on 5 Nov.

- **REST API:** `52.209.7.51:3003`
- **Our Grafana:** `52.209.7.51:3001`. *Event pipeline* = `/d/event-pipeline`, *Deployment cost* = `/d/deployment-cost`.
- **Prometheus for the course Grafana:** `:3008`
- **Course Grafana:** `grafana.cec.dlandau.nl` → *group_personal*

## 1. The picture

```
Kafka topic "experiment" (16 partitions, key = experiment id)
   │                                   │
   ▼ group group16-k8s-store           ▼ group group16-k8s-notify
store-consumer (1-3 pods, KEDA)      notify-consumer (always 2)
   │ 1 SQL statement per reading       │ averages sensors in memory
   ▼                                   ▼
Postgres (Docker on group16)         notifications service (HTTPS POST)
   ▲ reads measurement_averages
REST API (2-3 pods, one per node) ◄── clients → 52.209.7.51:3003

Prometheus (5 s scrape) + Grafana on group16 · node-exporter on each node · postgres-exporter
KEDA reads the Kafka lag from Prometheus · metrics-server gives CPU to the REST HPA
```

**Nodes:**
- **group16:** control plane plus workloads plus Postgres; 2 vCPU, 3.8 GB.
- **client14:** worker; 2 vCPU, 1.9 GB.

**Two networks:**
- **Pod traffic goes over the VPC** (flannel `--iface=ens5`, 0.5 ms between the nodes, 8951-byte packets).
- **The nodes register and talk to the API server over Tailscale.**

## 2. Where each thing is configured

| What | File | Key settings | Why (one line) |
|---|---|---|---|
| Whole deploy | `kustomization.yaml`, `Makefile` | namespace `cec`; image tags (rest-api `exp-pool`); secrets from `.env` + `auth/` | one `make start` deploys everything |
| Pod network | flannel DaemonSet (`kube-flannel` ns), steps in `k8s/README.md` | `--iface=ens5` | Tailscale encrypts every packet in software; forwarded bursts were ~5x slower |
| REST Service | `k8s/rest-api.yaml` | `externalIPs: 10.0.1.151` (= 52.209.7.51), port 3003, `externalTrafficPolicy: Cluster` | group16 forwards ~half the requests to client14's pod, so both nodes' CPUs work |
| REST pods | `k8s/rest-api.yaml` | 1 per node (`topologySpreadConstraints`); `maxSurge 1 / maxUnavailable 0`; `GRANIAN_WORKERS=1`; `CACHE_TTL_MS=1000`; 100m/96Mi request, 256Mi limit; readiness `/health` 5 s × 6; `preStop sleep 5` | no downtime on rollouts; a leak can't take the node |
| REST scaling | `k8s/rest-api-hpa.yaml` | min 2, max 3, CPU 500m per pod | 2 = one restarts, the other answers; each extra pod also reads Postgres itself |
| REST code | `rest-api/app.py`, `rest-api/Dockerfile` | granian `--interface rsgi` (async); pool min 4 / max 16 (`DB_POOL_MAX`); one read per experiment shared by all requests, reused 1 s; `/metrics` | the tester sends ~70+ requests at once |
| store-consumer | `k8s/store-consumer.yaml`, `k8s/store-consumer-scaledobject.yaml` | KEDA 1-3 pods, `threshold: 3000` lag per pod; init container waits for Postgres | it waits on Postgres, so CPU stays low while it falls behind: lag is the real signal |
| store logic | `consumer/src/bin/store-consumer/store.rs` | one statement: insert the reading; if it's the last sensor, insert the average (with `out_of_range`) | the API reads finished averages; replays are safe (`ON CONFLICT DO NOTHING`) |
| notify-consumer | `k8s/notify-consumer.yaml`, `consumer/src/bin/notify-consumer/notifier.rs` | `replicas: 2`; 16 sender threads, queue of 256; sends the queue on SIGTERM | state lives in memory, so no autoscaling; a full queue backs up into Kafka, not RAM |
| Kafka client (both) | `consumer/src/kafka.rs` | `cooperative-sticky`; leave the group on SIGTERM; prefetch 16 MiB; `auto.offset.reset=latest`; auto-commit | scaling moves partitions without pausing all 16; bounded memory while catching up |
| Fresh start | `Makefile` (`kafka-reset`), `scripts/reset-kafka-offsets.py` | groups jump to the newest message when the namespace didn't exist | no replay of old runs (was ~129,000 messages at once) |
| Postgres | `Makefile` (`postgres-up`), `k8s/postgres.yaml`, `database/init.sql` | Docker `--restart unless-stopped`, volume `pgdata`; Service without selector + EndpointSlice `10.0.1.151:5432` | data survives cluster rebuilds; pods find it as `postgres:5432` |
| Schema | `database/init.sql` | `measurements` (one row per sensor); `measurement_averages` (PK `experiment_id, timestamp, measurement_id` INCLUDE `temperature, out_of_range`) | the API answers from the index alone |
| Retention | `scripts/cleanup-retention.sh` (cron), `make stop` | delete > 1 day; `make stop` truncates | the disk was 85% full on 8 Oct |
| Monitoring | `k8s/monitoring/*.yaml`, `k8s/monitoring/prometheus.yml`, `monitoring/grafana/dashboards/*.json` | scrape 5 s, retention 2 h (emptyDir); jobs consumers / rest-api / postgres / node; Prometheus also on `:3008`; Grafana hostPort 3001 on group16 | the course Grafana reads our VMs from `:3008`; KEDA needs the lag |
| Autoscalers | `make autoscalers-install` | metrics-server (`--kubelet-insecure-tls`), KEDA 2.21 | HPA needs pod CPU, KEDA needs its CRDs before the app |

## 3. One measurement, end to end

1. The producer writes `SensorTemperatureMeasured` to Kafka. The key is the experiment, so one experiment always lands in one partition, in order.
2. **store-consumer:** one SQL statement per reading. It inserts into `measurements`. If that sensor was the last one for its `measurement_id`, it also inserts the average into `measurement_averages`.
3. **notify-consumer:** sums the sensors in memory, and when all have reported, compares the average with the thresholds:
   - **Stabilized:** during the stabilizing phase, the first average back in range.
   - **OutOfRange:** while carrying out, the first average that leaves the range.
   The notification goes into the queue, and one of the 16 senders POSTs it.
4. **REST GET:**
   - The request goes to the Service at 52.209.7.51:3003, then to a pod on either node.
   - The pod gets the experiment's rows: it reads them, joins a read already in progress, or reuses rows read under 1 s ago.
   - It cuts out the `start-time`…`end-time` window with a binary search, and sends the JSON (the full answer is built once and shared).

## 4. Numbers to know

| Fact | Number |
|---|---|
| Target test (k6 from client14, 175 experiments) | **1,500 req/s with 80-100 in flight: 0% late, 0 errors**, slowest batch 991 ms; group16 77% CPU, client14 67% (including k6) |
| Before (all REST pods on group16) | stopped at ~1,000 req/s, group16's CPU full |
| Async REST, bursts of 70 | median 69 → 13 ms |
| Averages written by the consumer | API median 10.3 → 2.7 ms; Postgres CPU 89% → 13% at 200 req/s; most it handled 160 → 777 req/s |
| One Postgres read | 0.25-0.3 ms; database ~26 MB, 100% cache hits; ~17-35 of 100 connections in use |
| store-consumer | 1.7 ms per event ≈ 580 events/s per pod (course load ≈ 200/s) |
| Kafka handover on scale-down | 0.4 s instead of 10 s |
| Notifier during a replay | 644 → 36 threads, 278 → 144 MiB |
| Node to node | VPC 0.48 ms round trip; forwarded bursts over Tailscale were 76 ms against 15 ms local |
| Memory | REST pod ~60-85 MiB; store ~7 MiB; notify 8-17 MiB; control plane ~640 MiB (API server 452) |
| gzip | the tester never asks for it (0 of ~4,180 of its requests), so we don't compress |

## 5. Grafana: which panel answers what

| Question | Where |
|---|---|
| Are we keeping up with Kafka? | *Event pipeline → Overview*: "Backlog (lag)", "End-to-end delay p95", "Time to catch up" |
| Why did store-consumer scale? | *Bottlenecks*: "Capacity vs incoming load", "Where the consumer spends its time"; *Deployment cost → What drives the cost*: "Load and consumer pods" |
| Is the database the bottleneck? | *PostgreSQL*: "Connections by state", "Buffer cache hit ratio"; *Bottlenecks*: "DB request latency by operation (p95)", "REST API: DB read vs whole request" |
| How fast is the API? | *Bottlenecks*: "REST API: response time histogram (inside the pods)" (same buckets as the course's) |
| How much does reuse save? | *Bottlenecks*: "REST API: where each request's rows came from" (read / shared / reused) |
| Are notifications on time? | *Events & notifications*: "Notification service latency (p95)"; *Notify-consumer*: "Notification latency breakdown (p95)"; course Grafana: notifications > 10 s |
| What does it cost? | *Deployment cost*: "CPU usage %", "CPU / Memory per component" ("other" = Kubernetes, Postgres, OS) |

The CPU panels average over 1 minute and hide bursts. To see short peaks, use Explore: `100 * sum by (instance) (rate(node_cpu_seconds_total{mode!="idle"}[15s])) / 2`.

## 6. Likely questions, short answers

- **Why Kubernetes?** The assignment asks to scale with the load. Pods restart by themselves and rollouts cause no downtime.
- **Why does store scale on lag and not CPU?** It waits on Postgres, so its CPU stays low even while it falls behind.
- **Why at most 3 store pods?** They all write into one Postgres.
- **Why isn't notify autoscaled?** It holds experiment state in memory, and a rebalance would hand experiments to a pod that never saw their configuration. It uses ~5m CPU.
- **Why exactly 2 notify pods?** If one dies, only half the experiments are affected.
- **Why two consumer groups?** Each group gets every event. Storing may lag; notifying must stay under 10 s and never waits on the database.
- **Delivery guarantee?** Replays are harmless: inserts are idempotent (`ON CONFLICT DO NOTHING`), and the average is written in the same statement as the last reading. But offsets are auto-committed once an event is read, so an event whose handling fails is skipped (see Known limits).
- **Why the 1 s cache, and isn't the data stale?** Answers are at most 1 s old, and that halved the Postgres reads under a 100-experiment load.
- **Why 16 database connections per pod?** A burst over many experiments starts ~20 reads at once; with 4 connections they queued in 5 rounds.
- **What if client14 dies?** After ~40 s Kubernetes marks the node NotReady, and the Service stops sending requests to its pod; until then about half the requests time out. group16's REST pod keeps answering, and the consumers are rescheduled.
- **What if group16 dies?** Everything stops: the control plane, Postgres and the public IP all live there. We accept that for a 2-VM setup.
- **How does a rollout avoid errors?** A new pod has to be ready before an old one stops (`maxUnavailable: 0`). The old pod then waits 5 s (`preStop`) while kube-proxy stops sending it requests.
- **Why is Postgres outside Kubernetes?** Its volume survives cluster rebuilds, it needs no storage setup in kubeadm, and there's one database.
- **Why did you skip Redis?** The database isn't the bottleneck (0.3 ms queries, 100% cache hits), and the in-process sharing already does what a cache would.
- **Why not CPU limits?** Throttling adds latency. Memory limits and requests protect the nodes.

**Known limits, if asked:**
- **Database errors:** if a write fails, store-consumer logs it and moves on, and its database connection doesn't reconnect after a Postgres restart, so the pod has to be restarted.
- **Hardware:** the VMs are burstable `t3a` instances, so sustained high CPU drains their credits.

## 7. Commands

```bash
make start                  # Postgres up, autoscalers, kubectl apply -k ., wait for rollouts
make status / make watch    # pods, HPA, ScaledObject, kubectl top
make stop                   # delete namespace cec, truncate + stop Postgres
kubectl -n cec get hpa,scaledobject
kubectl -n cec logs -f deploy/notify-consumer
~/stress-test/run.sh        # on client14: 500/1000/1500 req/s, 80-100 in flight, all experiments
```
