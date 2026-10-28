#!/bin/sh
# Reserves CPU and memory on this node for everything that is not a pod: the
# OS, kubelet, containerd, SSH and, on group16, Postgres in Docker. The
# scheduler then only places pods into the rest, and the kubelet evicts pods
# before the node itself runs out of memory (the VMs have no swap).
#
# Run once on every node:  sudo scripts/reserve-node.sh [cpu] [memory]
# Defaults: 500m CPU; 1Gi memory on nodes with more than 3 GiB RAM (group16:
# Postgres runs outside Kubernetes there), 512Mi on the 2 GB nodes.
set -eu

CONFIG=/var/lib/kubelet/config.yaml
MEM_KIB=$(awk '/MemTotal/ {print $2}' /proc/meminfo)
CPU=${1:-500m}
if [ -n "${2:-}" ]; then MEM=$2; elif [ "$MEM_KIB" -gt 3145728 ]; then MEM=1Gi; else MEM=512Mi; fi

python3 - "$CONFIG" "$CPU" "$MEM" <<'EOF'
import sys, yaml
path, cpu, mem = sys.argv[1:]
config = yaml.safe_load(open(path))
config["systemReserved"] = {"cpu": cpu, "memory": mem}
yaml.safe_dump(config, open(path, "w"), default_flow_style=False)
EOF
systemctl restart kubelet
echo "Reserved $CPU CPU and $MEM memory on $(hostname); kubelet restarted. Running pods keep running."
