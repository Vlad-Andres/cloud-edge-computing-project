# Scaling review (2026-10-28)

> History: this reviews the first autoscaling attempt on branch `feat/kubernetes` (up to `032ce16`). This branch redoes the scaling one step at a time from the fixed setup; see `IMPLEMENTATION-PLAN.md`.

## In short

The first autoscaling version let the cluster grow faster and further than the machines and the single Postgres can handle. During a stress test store-consumer jumped to 16 pods, the 2 GB VMs ran out of memory, group16 stopped answering, and the autoscaler froze at the maximum. The fixes cap the scaling to what is useful, make every component's memory bounded, keep room on each VM for the system, and start every cluster run at the current Kafka events.

## What we measured

| | Value | Meaning |
|---|---|---|
| store-consumer, time per event | 1.7 ms (almost all Postgres write) | one pod stores ~580 events/s |
| Live traffic | ~100 events/s | 2 store-consumers are plenty |
| store-consumer memory | 6 MiB live, **139 MiB** catching up | Kafka prefetch (64 MiB by default) |
| notify-consumer memory | 9-17 MiB live, **245-278 MiB** replaying | prefetch + one thread per notification |
| client14 kernel log | `global_oom`, killed `consumer` (245 MiB) | the **node** ran out of memory, not the pod limit |
| Memory limits on client14 | 222 % of its RAM | limits promised more than exists |
| KEDA + metrics-server actual use | ~15m CPU, ~130 MiB | cheap |

## Problems found

1. **Scaling the wrong thing.** 16 store-consumers all write into one Postgres on group16; more pods only added load.
2. **Scaling too fast.** 2 → 6 → 12 → 16 pods within a minute.
3. **Backlog replay.** After a stop or a stress test the groups resumed their old position: ~129,000 old messages at once.
4. **Memory spikes during catch-up** (prefetch, notifier threads) on VMs without swap → node-level out-of-memory kills.
5. **Requests guessed too small** (50m / 32Mi), and nothing reserved for the system: the scheduler packed 7 store-consumers next to the control plane and Postgres on group16, and the API server starved.
6. **Autoscaler blind under overload.** KEDA's metrics API runs in the overloaded cluster; the HPA froze at 16 pods for ~25 min. The fallback only covers "Prometheus unreachable".
7. **Pods stuck on a dead node.** `ip-10-0-1-72` went `NotReady`; 13 pods stayed "Terminating" and blocked clean shutdowns.
8. **CPU credits.** The VMs are burstable `t3a`; sustained load above ~20 % per CPU drains them and AWS throttles the VM.

## What changed

| Area | Change | Why |
|---|---|---|
| `k8s/store-consumer-scaledobject.yaml` | 2-**4** pods (was 16), threshold **1000** (was 500), **+1 pod per minute**, fallback **2** (was 4), removed `pollingInterval` | scale only on a real backlog, slowly, and never beyond what Postgres can take |
| `k8s/rest-api-hpa.yaml` | 2-**4** pods (was 8), CPU target **60 %** (was 70), **+1 pod per 30 s** | scale before requests queue, without filling the small VMs |
| Requests / limits (all pods, incl. monitoring) | set from measured usage; memory limits on every pod | the scheduler can only place what really fits; no pod can take a whole node |
| `consumer/src/kafka.rs` | Kafka prefetch capped at **16 MiB** | bounded memory while catching up |
| `consumer/.../notifier.rs`, `main.rs` | **16 senders + bounded queue** instead of a thread per notification; on SIGTERM it **sends the queue before exiting** | a slow service backs up into Kafka lag, not memory; no notification lost on shutdown |
| `k8s/notify-consumer.yaml` | 90 s shutdown grace period | time to send the queue even if the service is slow |
| `scripts/reset-kafka-offsets.py` + `make start` | both groups start at the **newest message** | no replay of old messages on a new run |
| `make stress-kafka` | resets its test group first | old test data isn't replayed |
| `make stop` | force-removes pods stuck on dead nodes after 100 s | `stop` never hangs |
| `scripts/reserve-node.sh` | reserves 500m CPU + 512Mi (1Gi on group16) per node | the OS, kubelet, SSH and Postgres always keep room; pods are evicted before the VM freezes |
| `k8s/README.md` | updated values, reservation step, stress tests | |

Not changed: the REST API's code (not ours), Postgres (still one container on group16), notify-consumer stays at 2 pods (its state is in memory).

## Tested

| Test | Result |
|---|---|
| Replay of 317,000 messages with a slow notifications service, old vs new notifier | **278 → 144 MiB**, **644 → 36 threads** |
| SIGTERM with 273 notifications queued | all **273 sent** before exit, exit code 0 |
| Offset reset on a throwaway group | all 16 partitions moved to the newest message |
| Manifests | server-side dry run ok; ScaledObject valid against KEDA 2.21's schema; `make -n` ok |

## Can messages be lost?

Only on purpose: `make start` skips the events produced **while the cluster was stopped**. Everything after that is processed; a restarted or rescheduled pod continues from its saved position, and the notifier sends its queue before it exits.

## What you need to do

1. **Push the new consumer images** (built on client14 as `:next`):
   ```sh
   for i in store-consumer notify-consumer; do docker tag vladandres/$i:next vladandres/$i:latest && docker push vladandres/$i:latest; done
   ```
2. **Reserve room on every node**, once, from the repo root: `sudo scripts/reserve-node.sh`
3. **The third node `ip-10-0-1-72`**: check the VM; if it's gone for good, `kubectl delete node ip-10-0-1-72`.
4. **Restart cleanly** on group16: `make stop`, then `make start`.
5. For stress tests, run the load generator from outside the cluster if you can (e.g. `hey` from a laptop).
