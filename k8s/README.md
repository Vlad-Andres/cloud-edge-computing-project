# Kubernetes

Runs the whole project on the cluster. Everything lives in namespace `cec` and is deployed with Kustomize (built into `kubectl`) from the repo root:

```sh
kubectl apply -k .
```

`kustomization.yaml` in the repo root lists the manifests in this folder and builds ConfigMaps/Secrets from files that already exist: `monitoring/grafana/...`, `.env` and `auth/`. 

The consumers use their own consumer groups (`group16-k8s-store`, `group16-k8s-notify`), so they run next to the docker setup on group16 without taking its partitions.

The REST API Service listens on port 3003 of both nodes' private IPs (`externalIPs`: `10.0.1.151` behind `52.209.7.51`, `10.0.1.230` behind `18.203.181.249`), and kube-proxy spreads the connections over all rest-api pods, whichever address they arrive at. Nothing else may use port 3003 on those nodes (e.g. the docker REST API on group16).

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

KEDA scales store-consumer on its Kafka backlog. Install it before deploying: `kubectl apply -k .` fails on the ScaledObject while KEDA's CRDs are missing.

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

After pushing new images (tag `latest` is always pulled):

```sh
kubectl -n cec rollout restart deployment store-consumer notify-consumer rest-api
```

## Autoscaling

Each component runs the smallest count that covers the course's default load (~200 events/s, 100-200 req/s). Pods are added only when that measured limit is reached.

| Component | Pods | Scaled by | One pod handles |
|---|---|---|---|
| store-consumer | 1-3 | KEDA, on the Kafka backlog the pods report to Prometheus: pods = ceil(lag / 3000) (`k8s/store-consumer-scaledobject.yaml`) | ~580 events/s (1.7 ms per event, almost all of it the Postgres write) |
| rest-api | 2-4 | HPA, on CPU above 500m per pod, i.e. above ~540 req/s (`k8s/rest-api-hpa.yaml`) | 200 req/s at ~370m CPU |
| notify-consumer | fixed 2 | not scaled | |

- store-consumer can run with 1 pod: while it restarts, events wait in Kafka. rest-api keeps 2, so a restarting pod never means failed requests.
- store-consumer stops at 3 because all pods write into the one Postgres on group16. rest-api stops at 4 (2 per node), because more pods would only share the same CPUs.
- notify-consumer isn't scaled. It keeps each experiment's state in memory, and every rebalance moves partitions to a pod that never saw those experiments' configuration, so their notifications would be lost.
- `replicas` is deliberately missing from the store-consumer and rest-api Deployments; with it, every `kubectl apply -k .` would reset what the autoscaler chose.

Watch it:

```sh
kubectl -n cec get hpa,scaledobject -w
kubectl -n cec top pods
```

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
- During the demo the Docker consumers on group16 must be stopped; otherwise every event is written twice and every notification sent twice.
