# Kubernetes

Runs the whole project on the cluster. Everything lives in namespace `cec` and is deployed with Kustomize (built into `kubectl`) from the repo root:

```sh
kubectl apply -k .
```

`kustomization.yaml` in the repo root lists the manifests in this folder and builds ConfigMaps/Secrets from files that already exist: `monitoring/grafana/...`, `.env` and `auth/`. Changing one of those files and re-applying restarts the pods that use it.

| Component | Kind | Where | Reachable at |
|---|---|---|---|
| Postgres | Service only, no pod | the docker container on the worker (group16) | `postgres:5432` (in cluster) |
| store-consumer | Deployment, 4 replicas | spread over the nodes | metrics `:9091` per pod |
| notify-consumer | Deployment, 2 replicas | spread over the nodes | metrics `:9091` per pod |
| REST API | Deployment, 2 replicas + Service | one per node | `http://52.209.7.51:3003` and `http://18.203.181.249:3003` |
| Prometheus | Deployment + Service | any node | `kubectl port-forward` (below) |
| Grafana | Deployment + Service | control-plane node | `http://18.203.181.249:3001` (`admin`/`admin`) |
| postgres-exporter | Deployment + Service | any node | `postgres-exporter:9187` (in cluster) |

The consumers use their own consumer groups (`group16-k8s-store`, `group16-k8s-notify`), so they run next to the docker setup on group16 without taking its partitions.

The experiment topic has 16 partitions: each store-consumer gets 4, each notify-consumer 8. Running two notify-consumers works because the producer keys every message by experiment ID, so all events of an experiment are in one partition and reach the same pod (notify-consumer keeps experiment state in memory).

Postgres is not in the cluster: `k8s/postgres.yaml` is a Service without pods whose endpoint is the worker's Tailscale IP, so `postgres:5432` reaches the docker Postgres the docker consumers also write to. Both setups storing the same events is harmless: the inserts use `ON CONFLICT DO NOTHING`.

The REST API Service listens on port 3003 of both nodes' private IPs (`externalIPs`: `10.0.1.151` behind `52.209.7.51`, `10.0.1.230` behind `18.203.181.249`), and kube-proxy spreads the connections over both replicas, whichever address they arrive at. Nothing else may use port 3003 on those nodes (e.g. the docker REST API on group16).

## Setting up

### 1. Cluster

kubeadm cluster (v1.34) over Tailscale, Flannel as CNI. All nodes advertise their Tailscale IP, so cluster traffic goes over the VPN.

| Node            | Role                                | Tailscale IP   | Public IP      |
|-----------------|-------------------------------------|----------------|----------------|
| `ip-10-0-1-230` | control plane (also schedules pods) | 100.125.52.112 | 18.203.181.249 |
| `ip-10-0-1-151` | worker                              | 100.122.68.0   |                |

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

#### Control plane (`ip-10-0-1-230`)

Config file. `taints: []` lets the control plane run pods too:

```sh
cat <<'EOF' > kubeadm-init.yaml
apiVersion: kubeadm.k8s.io/v1beta4
kind: InitConfiguration
localAPIEndpoint:
  advertiseAddress: 100.125.52.112
  bindPort: 6443
nodeRegistration:
  criSocket: unix:///run/containerd/containerd.sock
  kubeletExtraArgs:
    - name: node-ip
      value: 100.125.52.112
  taints: []
---
apiVersion: kubeadm.k8s.io/v1beta4
kind: ClusterConfiguration
kubernetesVersion: v1.34.12
controlPlaneEndpoint: 100.125.52.112:6443
apiServer:
  certSANs:
    - 100.125.52.112
    - 10.0.1.230
    - ip-10-0-1-230
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

#### Worker (`ip-10-0-1-151`)

On the worker, make kubelet advertise the Tailscale IP:

```sh
echo 'KUBELET_EXTRA_ARGS=--node-ip=100.122.68.0' | sudo tee /etc/default/kubelet
```

On the control plane, print a join command (the token is valid for 24h):

```sh
kubeadm token create --print-join-command
```

On the worker, run the printed command with `sudo`:

```sh
sudo kubeadm join 100.125.52.112:6443 --token <token> --discovery-token-ca-cert-hash sha256:<hash>
```

#### Verify

```sh
kubectl get nodes -o wide
kubectl get node ip-10-0-1-230 -o jsonpath='{.spec.taints}'
```

Both nodes `Ready` with their Tailscale IPs as `INTERNAL-IP`; the second command prints nothing (no taints).

- A `kube-flannel-ds` pod failing with `Failed to check br_netfilter`: the prerequisites were skipped on that node.
- Pods stuck with `"cni0" already has an IP address different from ...`: the node kept a stale subnet from an old cluster. On that node `sudo ip link delete cni0`, then delete the stuck pods.

### 2. Credentials

On the control-plane node, in the repo root (both are gitignored):

- `auth/` with the group's `ca.crt` and `kafka.keystore.pkcs12`
- `.env` with the credentials of the worker's docker Postgres. `NOTIFICATIONS_TOKEN` is the contents of the group's `auth/token`; `POSTGRES_HOST`/`POSTGRES_PORT` are ignored in the cluster (the pods always use `postgres:5432`).

```
NOTIFICATIONS_TOKEN=<contents of auth/token>

POSTGRES_HOST=postgres
POSTGRES_PORT=5432
POSTGRES_DB=<same as the worker's .env>
POSTGRES_USER=<same as the worker's .env>
POSTGRES_PASSWORD=<same as the worker's .env>
```

After changing `.env`, `kubectl apply -k .` restarts the pods that use it.

### 3. Build and push the images

The cluster pulls from Docker Hub (`vladandres/*`). The control-plane node has 2 GB of RAM, which isn't enough to compile the consumers, so add swap for the build and remove it afterwards (kubelet refuses to start with swap on):

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

### 4. Deploy

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

# Database (on the worker)
docker exec -it postgres psql -U groupdb -d groupdb

# Prometheus UI on http://localhost:9090 of the machine running this
kubectl -n cec port-forward svc/prometheus 9090:9090
```

## Tear down

```sh
kubectl delete -k .
```

This doesn't touch Postgres: it's the docker container on the worker.

## Notes

- Grafana is pinned with the `node-role.kubernetes.io/control-plane` label and uses a host port, so its public address is the control-plane node's IP, `18.203.181.249`.
- Grafana uses host port 3001 because 3000 is taken on the control-plane node.
