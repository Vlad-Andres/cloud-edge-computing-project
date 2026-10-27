# Start or stop the whole setup with one command. Run it on group16 from the
# repo root: Postgres runs in Docker there, and .env/auth/ are needed by
# kubectl apply -k. Stopping frees the CPU so the VMs earn back CPU credits.
#
#   make start                  Postgres -> autoscalers -> app
#   make stop                   app -> autoscalers -> Postgres
#   make status                 nodes, pods, autoscalers
#   make autoscalers-install    metrics-server + KEDA (safe to run repeatedly)
#   make autoscalers-uninstall  remove them again; a running app keeps its pod counts

SHELL := /bin/bash

METRICS_SERVER := https://github.com/kubernetes-sigs/metrics-server/releases/download/v0.9.0/components.yaml
KEDA           := https://github.com/kedacore/keda/releases/download/v2.21.0/keda-2.21.0.yaml
POSTGRES       := postgres

.DEFAULT_GOAL := status
.PHONY: start stop status postgres-up postgres-down autoscalers-install autoscalers-uninstall check-docker-consumers

start: check-docker-consumers postgres-up autoscalers-install
	kubectl apply -k .
	for d in rest-api store-consumer notify-consumer; do kubectl -n cec rollout status deploy/$$d --timeout=180s; done
	@$(MAKE) --no-print-directory status

# Deleting the namespace removes the whole app, even when KEDA is already gone
# or .env is missing (kubectl delete -k would fail on those)
stop:
	kubectl delete namespace cec --ignore-not-found --wait=true --timeout=180s
	@$(MAKE) --no-print-directory autoscalers-uninstall
	@$(MAKE) --no-print-directory postgres-down

status:
	kubectl get nodes
	kubectl -n cec get pods,hpa -o wide 2>/dev/null || true
	kubectl -n cec get scaledobject 2>/dev/null || true
	kubectl top nodes 2>/dev/null || echo "(kubectl top needs metrics-server: make autoscalers-install)"

# The Docker consumers on group16 would read the same events: every event
# would be stored twice and every notification sent twice
check-docker-consumers:
	@if docker ps --format '{{.Names}}' | grep -qE '^(store|notify)-consumer$$'; then \
	  echo "Docker consumers are running; stop them first: docker stop store-consumer notify-consumer"; exit 1; fi

# Starts the existing container, or recreates it like database/README.md if it
# was started with --rm and is gone. The data lives in the pgdata volume.
postgres-up:
	@if docker container inspect $(POSTGRES) >/dev/null 2>&1; then \
	  docker start $(POSTGRES); \
	else \
	  docker network create cec >/dev/null 2>&1 || true; \
	  docker run -d --name $(POSTGRES) --restart unless-stopped --network cec \
	    --env-file .env \
	    -v "$(CURDIR)/database/init.sql":/docker-entrypoint-initdb.d/init.sql \
	    -v pgdata:/var/lib/postgresql/data \
	    -p 5432:5432 postgres:16-alpine; \
	fi
	until docker exec $(POSTGRES) pg_isready -q; do sleep 1; done

postgres-down:
	docker stop $(POSTGRES) 2>/dev/null || true

# --kubelet-insecure-tls: kubeadm's kubelets use self-signed certificates.
# The grep keeps the flag from being added twice on a reinstall.
# KEDA must be installed before the app: the app contains a ScaledObject.
autoscalers-install:
	kubectl apply -f $(METRICS_SERVER)
	kubectl -n kube-system get deploy metrics-server -o jsonpath='{.spec.template.spec.containers[0].args}' | grep -q kubelet-insecure-tls \
	  || kubectl -n kube-system patch deploy metrics-server --type=json \
	       -p '[{"op":"add","path":"/spec/template/spec/containers/0/args/-","value":"--kubelet-insecure-tls"}]'
	kubectl apply --server-side -f $(KEDA)
	kubectl wait --for=condition=established crd/scaledobjects.keda.sh --timeout=60s
	kubectl -n keda wait --for=condition=Available deploy --all --timeout=180s
	kubectl -n kube-system rollout status deploy/metrics-server --timeout=180s
	@# App already running: recreate its ScaledObject, removed with KEDA's CRDs
	@if kubectl get namespace cec >/dev/null 2>&1; then kubectl apply -k .; fi

autoscalers-uninstall:
	kubectl delete -f $(KEDA) --ignore-not-found
	kubectl delete -f $(METRICS_SERVER) --ignore-not-found
