# How to join the cluster

Adds a VM (Ubuntu, like the AWS VMs) to the group's Kubernetes cluster as a worker. Steps 1–7 run on your VM; step 8 runs on the control plane (group16).

The cluster is kubeadm v1.34 over Tailscale with Flannel as CNI; the control plane is `ip-10-0-1-151` (group16, Tailscale IP `100.122.68.0`). Nothing has to change in the manifests: networking, node-exporter and new pods pick up the node by themselves.

## 1. Tailscale

Join the group's tailnet (ask for an invite or auth key). The printed IP is the node's address in the cluster:

```sh
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up
tailscale ip -4
```

## 2. Kernel modules and sysctls

What Kubernetes/Flannel need, persistent across reboots:

```sh
printf 'overlay\nbr_netfilter\n' | sudo tee /etc/modules-load.d/k8s.conf
printf 'net.bridge.bridge-nf-call-iptables = 1\nnet.bridge.bridge-nf-call-ip6tables = 1\nnet.ipv4.ip_forward = 1\n' | sudo tee /etc/sysctl.d/k8s.conf
sudo modprobe overlay
sudo modprobe br_netfilter
sudo sysctl --system
```

## 3. Swap off

kubelet refuses to start with swap on, so this must print nothing (otherwise `sudo swapoff -a` and remove the swap line from `/etc/fstab`):

```sh
swapon --show
```

## 4. containerd

CRI plugin enabled and systemd cgroups, matching the cluster's `cgroupDriver: systemd`. Skip the install line if Docker is on the VM (it brings containerd); Docker's default containerd config disables the CRI plugin, and the default config below enables it again:

```sh
sudo apt-get install -y containerd
sudo mkdir -p /etc/containerd
containerd config default | sudo tee /etc/containerd/config.toml >/dev/null
sudo sed -i 's/SystemdCgroup = false/SystemdCgroup = true/' /etc/containerd/config.toml
sudo systemctl restart containerd
grep SystemdCgroup /etc/containerd/config.toml   # must say true
```

## 5. kubeadm, kubelet and kubectl

Same minor version as the cluster (1.34):

```sh
sudo apt-get update && sudo apt-get install -y apt-transport-https ca-certificates curl gpg
sudo mkdir -p -m 755 /etc/apt/keyrings
curl -fsSL https://pkgs.k8s.io/core:/stable:/v1.34/deb/Release.key | sudo gpg --dearmor -o /etc/apt/keyrings/kubernetes-apt-keyring.gpg
echo 'deb [signed-by=/etc/apt/keyrings/kubernetes-apt-keyring.gpg] https://pkgs.k8s.io/core:/stable:/v1.34/deb/ /' | sudo tee /etc/apt/sources.list.d/kubernetes.list
sudo apt-get update && sudo apt-get install -y kubelet kubeadm kubectl
sudo apt-mark hold kubelet kubeadm kubectl
```

## 6. Make kubelet advertise the Tailscale IP

```sh
echo "KUBELET_EXTRA_ARGS=--node-ip=$(tailscale ip -4)" | sudo tee /etc/default/kubelet
```

## 7. Join

Only if the VM was ever part of another cluster, reset it first. `kubeadm reset` doesn't clean CNI config, network interfaces or kubeconfig, hence the rest. Don't flush iptables: that would also wipe Docker's and Tailscale's rules.

```sh
sudo kubeadm reset -f --cri-socket unix:///run/containerd/containerd.sock
sudo rm -f /etc/cni/net.d/10-flannel.conflist /run/flannel/subnet.env
sudo ip link delete cni0
sudo ip link delete flannel.1
rm -rf $HOME/.kube
```

The join command contains a secret token that expires after 24h, so it isn't in this repo. Ask whoever has access to group16 to run this there and send you the line it prints (one line works for several VMs while it's valid):

```sh
sudo kubeadm token create --print-join-command
```

It prints a complete command starting with `kubeadm join 100.122.68.0:6443`. Run that whole line on your VM with `sudo` in front. If it fails because the token expired, ask for a new one.

## 8. Verify (on the control plane)

```sh
kubectl get nodes -o wide
```

The new node is `Ready` with its Tailscale IP as `INTERNAL-IP`.

Running pods don't move to a new node by themselves. To spread them out (notify-consumer loses the state of running experiments, so not during a demo or test run):

```sh
kubectl -n cec rollout restart deployment store-consumer rest-api notify-consumer
kubectl -n cec get pods -o wide
```

## Troubleshooting

- `kubeadm join` failing with `container runtime is not running`: containerd's CRI plugin is disabled (Docker's default config). Redo step 4.
- A `kube-flannel-ds` pod failing with `Failed to check br_netfilter`: step 2 was skipped on that node.
- Pods stuck with `"cni0" already has an IP address different from ...`: the node kept a stale subnet from an old cluster. On that node `sudo ip link delete cni0`, then delete the stuck pods.
