# Kubernetes

Runs the whole project on the cluster. Everything lives in namespace `cec` and is deployed with Kustomize (built into `kubectl`) from the repo root:

```sh
kubectl apply -k .
```

`kustomization.yaml` in the repo root lists the manifests in this folder and builds ConfigMaps/Secrets from files that already exist: `monitoring/grafana/...`, `.env` and `auth/`. 

The consumers use their own consumer groups (`group16-k8s-store`, `group16-k8s-notify`), so they run next to the docker setup on group16 without taking its partitions.

The REST API Service listens on port 3003 of both nodes' private IPs (`externalIPs`: `10.0.1.151` behind `52.209.7.51`, `10.0.1.230` behind `18.203.181.249`), and kube-proxy spreads the connections over both replicas, whichever address they arrive at. Nothing else may use port 3003 on those nodes (e.g. the docker REST API on group16).

Prometheus is also exposed on port 3008 of both nodes (Service `prometheus-external`, same mechanism), as the demo instructions ask: the course's Grafana reads the VMs' CPU and memory from the `node` job, which scrapes one node-exporter pod per node.

## Setting up

### 1. Cluster

kubeadm cluster (v1.34) over Tailscale, Flannel as CNI. All nodes advertise their Tailscale IP, so cluster traffic goes over the VPN.

| Node                       | Role                                | Tailscale IP   | Public IP      |
|----------------------------|-------------------------------------|----------------|----------------|
| `ip-10-0-1-151` (group16)  | control plane (also schedules pods) | 100.122.68.0   | 52.209.7.51    |
| `ip-10-0-1-230` (client14) | worker                              | 100.125.52.112 | 18.203.181.249 |

To add a new machine as a worker, follow [how-to-join-cluster.md](how-to-join-cluster.md).

#### Prerequisites (every node)

Kernel modules and sysctls Kubernetes/Flannel need, persistent across reboots:

```sh
printf 'overlay\nbr_netfilter\n' | sudo tee /etc/modules-load.d/k8s.conf
printf 'net.bridge.bridge-nf-call-iptables = 1\nnet.bridge.bridge-nf-call-ip6tables = 1\nnet.ipv4.ip_forward = 1\n' | sudo tee /etc/sysctl.d/k8s.conf
sudo modprobe overlay
sudo modprobe br_netfilter
sudo sysctl --system
```

Once the node is in the cluster, reserve CPU and memory for everything that isn't a pod (OS, kubelet, SSH, and Postgres on group16), so pods can never take the whole VM. From the repo root:

```sh
sudo scripts/reserve-node.sh   # 500m CPU; 1Gi memory on group16, 512Mi on the 2 GB nodes
```

#### Reset a node (any node leaving an existing cluster)

```sh
sudo kubeadm reset -f --cri-socket unix:///run/containerd/containerd.sock
```

`kubeadm reset` doesn't clean CNI config, network interfaces or kubeconfig:

```sh
sudo rm -f /etc/cni/net.d/10-flannel.conflist /run/flannel/subnet.env
sudo ip link delete cni0
sudo ip link delete flannel.1
rm -rf $HOME/.kube
```

Don't flush iptables. That would also wipe Docker's and Tailscale's rules; kube-proxy rewrites its own chains.

#### Control plane (`ip-10-0-1-151`)

Config file. `taints: []` lets the control plane run pods too:

```sh
cat <<'EOF' > kubeadm-init.yaml
apiVersion: kubeadm.k8s.io/v1beta4
kind: InitConfiguration
localAPIEndpoint:
  advertiseAddress: 100.122.68.0
  bindPort: 6443
nodeRegistration:
  criSocket: unix:///run/containerd/containerd.sock
  kubeletExtraArgs:
    - name: node-ip
      value: 100.122.68.0
  taints: []
---
apiVersion: kubeadm.k8s.io/v1beta4
kind: ClusterConfiguration
kubernetesVersion: v1.34.12
controlPlaneEndpoint: 100.122.68.0:6443
apiServer:
  certSANs:
    - 100.122.68.0
    - 10.0.1.151
    - ip-10-0-1-151
networking:
  podSubnet: 10.244.0.0/16
  serviceSubnet: 10.96.0.0/12
---
apiVersion: kubelet.config.k8s.io/v1beta1
kind: KubeletConfiguration
cgroupDriver: systemd
EOF
```

```sh
sudo kubeadm init --config kubeadm-init.yaml
```

```sh
mkdir -p $HOME/.kube
sudo cp /etc/kubernetes/admin.conf $HOME/.kube/config
sudo chown $(id -u):$(id -g) $HOME/.kube/config
```

Flannel, pinned to the `tailscale0` interface:

```sh
curl -sSL -o kube-flannel.yml https://github.com/flannel-io/flannel/releases/latest/download/kube-flannel.yml
sed -i 's/^\(\s*\)- --kube-subnet-mgr$/&\n\1- --iface=tailscale0/' kube-flannel.yml
kubectl apply -f kube-flannel.yml
kubectl -n kube-flannel rollout status ds/kube-flannel-ds
```

#### Worker (`ip-10-0-1-230`)

On the worker, make kubelet advertise the Tailscale IP:

```sh
echo 'KUBELET_EXTRA_ARGS=--node-ip=100.125.52.112' | sudo tee /etc/default/kubelet
```

On the control plane, print a join command (the token is valid for 24h):

```sh
kubeadm token create --print-join-command
```

On the worker, run the printed command with `sudo`:

```sh
sudo kubeadm join 100.122.68.0:6443 --token <token> --discovery-token-ca-cert-hash sha256:<hash>
```

#### Verify

```sh
kubectl get nodes -o wide
kubectl get node ip-10-0-1-151 -o jsonpath='{.spec.taints}'
```

Both nodes `Ready` with their Tailscale IPs as `INTERNAL-IP`; the second command prints nothing (no taints).

- A `kube-flannel-ds` pod failing with `Failed to check br_netfilter`: the prerequisites were skipped on that node.
- Pods stuck with `"cni0" already has an IP address different from ...`: the node kept a stale subnet from an old cluster. On that node `sudo ip link delete cni0`, then delete the stuck pods.

### 2. Credentials

On the machine you deploy from (client14: it has this repo and `~/.kube/config`), in the repo root (both are gitignored):

- `auth/` with the group's `ca.crt` and `kafka.keystore.pkcs12`
- `.env` with the credentials of group16's docker Postgres. `NOTIFICATIONS_TOKEN` is the contents of the group's `auth/token`; `POSTGRES_HOST`/`POSTGRES_PORT` are ignored in the cluster (the pods always use `postgres:5432`).

```
NOTIFICATIONS_TOKEN=<contents of auth/token>

POSTGRES_HOST=postgres
POSTGRES_PORT=5432
POSTGRES_DB=<same as group16's docker .env>
POSTGRES_USER=<same as group16's docker .env>
POSTGRES_PASSWORD=<same as group16's docker .env>
```

After changing `.env`, `kubectl apply -k .` restarts the pods that use it.

### 3. Build and push the images

The cluster pulls from Docker Hub (`vladandres/*`). client14 has 2 GB of RAM, which isn't enough to compile the consumers, so add swap for the build and remove it afterwards (kubelet refuses to start with swap on):

```sh
sudo fallocate -l 2G /swapfile-build
sudo chmod 600 /swapfile-build
sudo mkswap /swapfile-build
sudo swapon /swapfile-build
```

```sh
DOCKER_BUILDKIT=1 docker build --build-arg BIN=store-consumer -t vladandres/store-consumer:latest consumer
DOCKER_BUILDKIT=1 docker build --build-arg BIN=notify-consumer -t vladandres/notify-consumer:latest consumer
docker build -t vladandres/rest-api:latest rest-api
```

```sh
sudo swapoff /swapfile-build
sudo rm /swapfile-build
```

```sh
docker login -u vladandres
docker push vladandres/store-consumer:latest
docker push vladandres/notify-consumer:latest
docker push vladandres/rest-api:latest
```

On a Mac, build for the VMs with `docker buildx build --platform linux/amd64 ... --push` instead.

### 4. Autoscalers (once per cluster)

metrics-server provides pod CPU for the REST API's HPA. `--kubelet-insecure-tls` because kubeadm's kubelets use self-signed serving certificates:

```sh
kubectl apply -f https://github.com/kubernetes-sigs/metrics-server/releases/download/v0.9.0/components.yaml
kubectl -n kube-system patch deployment metrics-server --type=json \
  -p '[{"op":"add","path":"/spec/template/spec/containers/0/args/-","value":"--kubelet-insecure-tls"}]'
kubectl top nodes   # works after ~1 min
```

KEDA scales store-consumer on Kafka lag. Install it before deploying: `kubectl apply -k .` fails on the ScaledObject while KEDA's CRDs are missing.

```sh
kubectl apply --server-side -f https://github.com/kedacore/keda/releases/download/v2.21.0/keda-2.21.0.yaml
kubectl -n keda rollout status deploy/keda-operator
```

### 5. Deploy

```sh
kubectl kustomize . > /dev/null   # check it renders
kubectl apply -k .
kubectl -n cec get pods -o wide
```

The store-consumer pods wait for Postgres (init container `wait-for-postgres`) before starting.

Consumers built before the `cooperative-sticky` change can't share a consumer group with newer ones. If old store-consumer pods are still running when the new image arrives, stop them all first and wait out their 45 s session timeout (they leave the group without saying so):

```sh
kubectl -n cec scale deploy/store-consumer --replicas=0
kubectl -n cec wait --for=delete pod -l app=store-consumer --timeout=90s
sleep 45
kubectl -n cec scale deploy/store-consumer --replicas=2   # KEDA takes over from here
```

notify-consumer uses `Recreate`, so it never mixes old and new pods; at most it logs consumer errors for those 45 s until the old members expire.

After pushing new images (tag `latest` is always pulled):

```sh
kubectl -n cec rollout restart deployment store-consumer notify-consumer rest-api
```

## Autoscaling

| Component | Replicas | Scaled by |
|---|---|---|
| store-consumer | 2–4 | KEDA, on Kafka lag (`k8s/store-consumer-scaledobject.yaml`): pods = ceil(lag / 1000), at most +1 pod per minute |
| rest-api | 2–4 | HPA, on CPU, target 60 % of the 200m request (`k8s/rest-api-hpa.yaml`), at most +1 pod per 30 s |
| notify-consumer | fixed 2 | not scaled: it keeps each experiment's state in memory, and every rebalance moves partitions to a pod that never saw their experiments' configuration, so their notifications would be lost |

- The ranges are deliberately small. One store-consumer stores ~580 events/s (1.7 ms per event, almost all of it the Postgres write) and live traffic is ~100/s; extra pods only help to work off a backlog, and they all write into the one Postgres on group16. Scaling up to 16 overloaded the 2 GB VMs and group16's CPU credits (see `k8s/SCALING-REVIEW.md`).
- If KEDA can't read the lag, it falls back to 2 pods, not more.
- `make start` moves both consumer groups to the newest message first: a new run starts at the current events instead of replaying everything since the last run.
- The consumers fetch at most 16 MiB ahead from Kafka, and notify-consumer sends with 16 workers and a bounded queue (backpressure into Kafka lag instead of memory); on SIGTERM it sends what's queued before exiting.
- `replicas` is deliberately missing from the store-consumer and rest-api Deployments; with it, every `kubectl apply -k .` would reset what the autoscaler chose.
- Consumers use `cooperative-sticky` assignment and leave the group on SIGTERM, so a scale-up or scale-down only moves the partitions that change owner, right away.
- Requests and memory limits on every pod are based on measured usage; there are no CPU limits, they would only throttle.

Watch it:

```sh
kubectl -n cec get hpa,scaledobject -w
kubectl -n cec top pods
```

Pod count over time, for the report (Prometheus/Grafana): `count(up{job="store-consumer"} == 1)`.

Stress tests (from the repo root on group16, after `make start`; `make watch` in a second terminal):

```sh
make stress-rest    # hey against the REST API: DURATION=60s CONCURRENCY=50
make stress-kafka   # PROFILE=parallel (32 experiments, 640 events/s) on the group16 topic, threshold 100
```

`stress-kafka` restores the normal setup afterwards, also on errors and Ctrl+C. Run load generators from outside the cluster if you can (e.g. `hey` from a laptop); on group16 they take CPU from the system they measure.

During the demo the Docker consumers on group16 must be stopped; otherwise every event is written twice and every notification sent twice.

## Check

```sh
# Where the store-consumers ended up
kubectl -n cec get pods -l app=store-consumer -o wide

kubectl -n cec logs -f deployment/notify-consumer
kubectl -n cec logs -f -l app=store-consumer --prefix

curl http://52.209.7.51:3003/health

# Database (on group16)
docker exec -it postgres psql -U groupdb -d groupdb

# Prometheus UI (also what the course's Grafana queries for CPU/memory)
curl http://52.209.7.51:3008/-/ready
```

## Tear down

```sh
kubectl delete -k .
```

This doesn't touch Postgres: it's the docker container on group16.

## Notes

- Grafana is pinned with the `node-role.kubernetes.io/control-plane` label and uses a host port, so its public address is the control-plane node's IP, `52.209.7.51`.
- Grafana uses host port 3001 because 3000 is taken by the docker Grafana on group16.
