# Design decisions: cheat sheet

Each decision: what we chose, the numbers behind it, and where to show it. Grafana is at `http://52.209.7.51:3001` (admin/admin). Most panels are on the *Event pipeline* dashboard; "Explore" means Grafana's Explore tab with the PromQL given. Measurements are from 2026-11-04.

## In one minute

- The course's tester sends **bursts**: ~70 requests at the same moment, all for one experiment, then quiet. It cycles ~10 min on, 3-7 min off, with ~40-80 connections open.
- Our old REST API answered a burst **4 requests at a time per pod** (Flask: 2 processes x 2 threads), so the last requests of a burst waited 25-30 ms. On top of that, half the requests were forwarded to the other node **through Tailscale**, which was ~5x slower.
- Now the API is **async**, does **one database read per burst**, and **answers on the node where the request arrives**. Bursts went from p50 69 ms / max 165 ms to p50 13 ms / max 40 ms, and capacity from ~600 to 2,000+ req/s.
- The database was never the bottleneck: 11-15 of 100 connections, 100% cache hits, 2-4 queries running at the peak.

## Results

Bursts of 70 requests at once, every second, for the largest experiment (480 points, ~23 KB per answer), 60 s each, k6 from client14 (inside AWS, like the tester). They run on top of the tester's own bursts.

| Setup | p50 | p90 | p99 | max | <= 5 ms | <= 25 ms |
|---|---|---|---|---|---|---|
| Before: Flask, requests spread over both nodes | 68.7 ms | 112.2 | 135.1 | 165.1 | 0.0% | 11.7% |
| Async + one read per burst | 42.8 | 79.6 | 142.4 | 186.7 | 0.8% | 37.2% |
| + answer on the arrival node | 12.9 | 19.0 | 29.0 | 37.8 | 1.9% | 97.9% |
| Final: 2 pods x 1 process on group16 | 13.5 | 18.8 | 29.5 | 39.9 | 0.4% | 98.2% |
| Final, client sends `Accept-Encoding: gzip` | 11.3 | 15.3 | 20.2 | 24.6 | 0.6% | 100% |

Steady load over 20 experiments (k6 `rest-load.js` from client14, no gzip): every step OK up to **2,000 req/s**, 0 errors, p50 1.9 ms at 400 req/s, p95 108 ms at 2,000 req/s. Before: ceiling ~600-650 req/s (k6 from a laptop), group16's CPU at 100%.

Show it: *Event pipeline* -> *Bottlenecks* -> "REST API: response time histogram (inside the pods)" (same buckets as the course's histogram) and "REST API: DB read vs whole request".

## Decisions

### 1. Async server instead of Flask with threads
- **Claim:** the old API queued bursts in front of a fixed number of threads.
- **Numbers:** a Flask pod works on 4 requests at a time; ~23 requests per pod per burst, ~3 ms each, so the last waited ~25-30 ms. A packet capture at the pod: 22-24 requests arrived within 15-53 ms, answered between 8 and 32 ms after arrival, while Flask itself took ~3 ms.
- **Now:** granian's RSGI interface, one event loop per process, async Postgres pool (`rest-api/app.py`). A request waiting for Postgres holds no thread.
- **If asked "why not just more threads?":** Python runs one thread at a time per process (the GIL), and group16 has 2 CPUs. More threads would wait for the CPU instead of in granian's queue.

### 2. One database read per burst
- **Claim:** a burst asks ~70 times for the same experiment, so one read can answer all of them.
- **How:** requests that arrive while an experiment is being read wait for that read ("shared"); its rows are reused for 200 ms ("reused"). Both endpoints come from the same rows.
- **Numbers:** at 2,000 req/s over 20 experiments only 12.6% of requests read Postgres (24,786 of 196,524 in 6 min).
- **Correctness:** 171 of 171 data answers byte-identical to the old API (all 20 experiments, both endpoints, inclusive time bounds, reversed/infinite/NaN windows, unknown experiment, missing/bad parameters).
- **Freshness:** an answer is at most 200 ms older than the database (`CACHE_TTL_MS`). With 1 s reuse bursts got faster still (p50 10.5 ms, 8% <= 5 ms), but we kept 200 ms so answers stay current.
- Show it: *Bottlenecks* -> "REST API: where each request's rows came from" (read / shared / reused per second).

### 3. Answer on the node where the request arrives
- **Claim:** the cluster network between our two VMs runs through Tailscale (flannel `--iface=tailscale0`, nodes registered with 100.x addresses), and crossing it is expensive.
- **Numbers, same burst test:**
  - client14's pod, from client14 (no crossing): p50 14.6 ms
  - group16's pod, from client14 (through Tailscale): p50 75.8 ms, max 228 ms
  - both pods on client14, so every request crosses twice: p50 147 ms; `tailscaled` used 23% CPU
  - after the change: p50 12.9 ms
- **How:** the Service has `externalTrafficPolicy: Local` and only group16's address (52.209.7.51, where the tester connects). The pods are pinned to group16.
- **If asked "isn't everything on one node a single point of failure?":** Postgres runs on group16. Without group16 there is no data to serve anyway.
- **If asked "why not fix Tailscale instead?":** moving flannel to the VPC interface means re-registering the nodes and the control plane's address. That's too risky right before the assignment.

### 4. Two pods with one process each, rollouts without a gap
- **Numbers:** 2 pods x 2 processes on group16's 2 CPUs: p50 16-18 ms. 2 x 1: p50 13.5 ms, the same as a single pod with 2 processes.
- **Why 2 pods:** one keeps answering while the other restarts.
- **Why `maxUnavailable: 0`:** with local routing, a request reaching a node without a ready pod is dropped. A new pod must be ready before an old one stops.

### 5. The database is not the bottleneck (so: no Redis, no more connections)
- **Numbers:**
  - 11-15 of 100 Postgres connections in use, even at 2,000 req/s; 2-4 queries running at the old peak
  - buffer cache hit ratio 100%: the whole database is 28 MB, and Postgres has 128 MB of cache
  - a read executes in 0.25-0.3 ms; the time went to Python and the network, not Postgres
- **Redis:** would add a service on the same two small VMs to do what the in-process sharing already does (one read per burst). It doesn't remove the network or the HTTP layer.
- **More connections:** each pool is capped at 4 per process; connections were never waiting.
- Show it: *Event pipeline* -> *PostgreSQL* -> "Connections by state" and "Buffer cache hit ratio".

### 6. Gzip for clients that ask for it
- **Numbers:** a full answer shrinks from 23,560 to 6,763 bytes. With gzip, bursts p50 11.3 vs 14.8 ms, p99 20 vs 77 ms.
- **The tester doesn't ask for gzip:** 0 of ~4,180 of its requests sent `Accept-Encoding: gzip`, so it gets plain JSON, exactly as before. Gzip is on (`GZIP=1`) for other clients (browsers, most HTTP libraries).
- Show it: the "accepts gzip" line in "REST API: where each request's rows came from".

### 7. Health checks that survive overload
- **Claim:** under overload Kubernetes almost removed REST pods from service.
- **Evidence:** "Readiness probe failed: context deadline exceeded" events during the old ramp. `/health` waits in the same line as the other requests, and the default timeout is 1 s. After 3 failures a pod is taken out of service and the others get its traffic.
- **Now:** `timeoutSeconds: 5`, `failureThreshold: 6`.

### 8. Memory requests and limits on the REST pods
- **Numbers:** ~70-85 MiB per pod. Request 96 MiB, limit 256 MiB.
- **Why:** before, no pod in `k8s/` had requests or limits. client14 dropped to ~255 MiB available during one experiment (the kubelet starts evicting pods below 100 MiB). With a limit, a leak kills only its own container, not the VM.

### 9. Autoscaler: 2-3 pods on CPU
- **Why max 3:** all pods are on group16; a 4th would only share the same 2 CPUs.
- **Why on CPU:** load shows up as CPU long before memory. At 2,000 req/s group16 reached 99% CPU in 15 s windows.

## What the numbers can't get below

70 tiny `/health` requests at once (16 bytes, no app logic) already take p50 4.9 ms, p99 26 ms through this path (AWS public IP, kube-proxy, granian). That's the floor for the network path, not for our code. Full answers add ~8 ms for the 70 x 23 KB per burst.

## Reading Grafana correctly

- **CPU usage % (Deployment cost)** uses 1-minute windows. Bursts and 30 s test steps get averaged with idle time: group16 at 97-100% in 15 s windows showed as ~60% there. Use a short time range, or Explore: `100 * sum by (instance) (rate(node_cpu_seconds_total{mode!="idle"}[15s])) / 2`.
- **Times inside the pods** don't include the network or granian. A client always sees more.
- **Useful Explore queries:**
  - tester activity: `sum(granian_connections_active{job="rest-api"})`, `sum(rate(granian_requests_handled{job="rest-api"}[1m]))`
  - gzip clients: `sum by (gzip) (increase(rest_api_accepts_gzip_total[5m]))`
  - Postgres reads per request: `sum(rate(rest_api_rows_lookups_total{source="read"}[1m])) / sum(rate(rest_api_rows_lookups_total[1m]))`

## Reproduce

- Bursts like the tester: `EXPERIMENT=<id> k6 run rest-burst.js` (70 at once, every second).
- Steady ramp over many experiments: `EXPERIMENTS=<id,...> k6 run rest-load.js`. Add `make stress-kafka PROFILE=over-100` on group16 to include the write load.
- Run from inside AWS to see what the tester sees. From a laptop, every number includes ~20-25 ms of internet round trip.
