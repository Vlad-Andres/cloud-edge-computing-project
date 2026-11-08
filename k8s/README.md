# Kubernetes deployment

The project runs in the `cec` namespace. The cluster uses kubeadm with
Flannel for pod networking. The control-plane node is also available for
workloads, so the services can run on the small cluster used for the project.

## Components

- **REST API** receives experiment requests on port `3003` and reads the
  results from PostgreSQL.
- **Store consumer** reads experiment events from Kafka and stores them in the
  PostgreSQL database.
- **Notification consumer** reads notification events and sends the
  corresponding notifications.
- **PostgreSQL** is the shared database. In this deployment it runs outside
  Kubernetes on the group16 host and is exposed to the cluster through the
  `postgres` Service.
- **Prometheus and Grafana** collect and display application, node, and
  database metrics.

The REST API is exposed through the control-plane node. The consumers use
separate Kafka consumer groups from the Docker deployment, so both
deployments can be present without consuming each other's partitions.

## Scaling

The REST API runs with two replicas by default and is scaled to three replicas
by a CPU-based Horizontal Pod Autoscaler. Pods are spread across the available
nodes.

The store consumer is scaled by KEDA using the Kafka backlog reported to
Prometheus. It runs between one and three replicas. The notification consumer
has two fixed replicas because its in-memory experiment state should not move
between consumers during a rebalance.

The deployments for the REST API and store consumer do not set `replicas`;
their replica counts are managed by the autoscalers.

## Deploying

The root `kustomization.yaml` includes all resources in this directory.
Before deploying, the machine running `kubectl` must have:

- a `.env` file containing the application and PostgreSQL credentials;
- the Kafka certificates in `auth/`;
- access to the target Kubernetes cluster;
- metrics-server and KEDA installed for autoscaling.

Build and publish the images referenced by the root kustomization, then apply
the manifests from the repository root:

```sh
kubectl apply -k .
kubectl -n cec get pods
```

To deploy new image versions:

```sh
kubectl -n cec rollout restart deployment/store-consumer
kubectl -n cec rollout restart deployment/notify-consumer
kubectl -n cec rollout restart deployment/rest-api
```

## Monitoring

Prometheus scrapes the application and PostgreSQL exporters, as well as one
node exporter per cluster node. Grafana runs on the control-plane node and
uses Prometheus as its data source.

Useful checks:

```sh
kubectl -n cec get pods -o wide
kubectl -n cec get hpa,scaledobject
kubectl -n cec top pods
```

The REST API health endpoint is available on port `3003` of the exposed node.
Prometheus is available on port `3008`.

## Removing the deployment

```sh
kubectl delete -k .
```

This removes the Kubernetes resources only. PostgreSQL remains running on the
group16 host.
