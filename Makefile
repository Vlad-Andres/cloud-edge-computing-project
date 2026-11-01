# Start or stop the whole setup with one command. Run it on group16 from the
# repo root: Postgres runs in Docker there, and .env/auth/ are needed by
# kubectl apply -k. Stopping frees the CPU so the VMs earn back CPU credits.
#
#   make start                  Postgres -> autoscalers -> Kafka groups to the newest message -> app
#   make stop                   app -> autoscalers -> Postgres
#   make status                 nodes, pods, autoscalers
#   make autoscalers-install    metrics-server + KEDA (safe to run repeatedly)
#   make autoscalers-uninstall  remove them again; a running app keeps its pod counts
#
# Stress tests (after make start; run make watch in a second terminal):
#   make stress-rest            HTTP load on the REST API  [DURATION=60s CONCURRENCY=50 EXPERIMENT=<id>]
#   make stress-kafka           Kafka load for store-consumer  [PROFILE=parallel STRESS_LAG=100]
#   make watch                  live view of autoscalers, pods and node load
#
# Benchmark:
#   make bench-rest             response time of both REST endpoints, fails above 5 ms  [EXPERIMENT=<id>]

SHELL := /bin/bash

METRICS_SERVER := https://github.com/kubernetes-sigs/metrics-server/releases/download/v0.9.0/components.yaml
KEDA           := https://github.com/kedacore/keda/releases/download/v2.21.0/keda-2.21.0.yaml
POSTGRES       := postgres
# Moves consumer groups to the newest message (scripts/reset-kafka-offsets.py);
# arguments: <topic> <group>...
RESET_OFFSETS  := docker run --rm -v "$(CURDIR)/auth":/auth:ro \
                    -v "$(CURDIR)/scripts/reset-kafka-offsets.py":/reset.py:ro python:3.12-slim \
                    sh -c 'pip install -q --disable-pip-version-check --root-user-action=ignore confluent-kafka && python /reset.py "$$@"' reset

REST_URL    ?= http://52.209.7.51:3003
DURATION    ?= 60s
CONCURRENCY ?= 50
EXPERIMENT  ?=
# A profile from useful-services/experiment-producer (parallel spreads over the partitions)
PROFILE     ?= parallel
# Lag per store-consumer pod during stress-kafka (3000 normally), so scaling shows
STRESS_LAG  ?= 100

.DEFAULT_GOAL := status
.PHONY: start stop status postgres-up postgres-down autoscalers-install autoscalers-uninstall check-docker-consumers \
        kafka-reset watch stress-rest stress-kafka bench-rest

start: check-docker-consumers postgres-up autoscalers-install kafka-reset
	kubectl apply -k .
	for d in rest-api store-consumer notify-consumer; do kubectl -n cec rollout status deploy/$$d --timeout=180s; done
	@$(MAKE) --no-print-directory status

# A new run starts at the current events instead of replaying everything since
# the last run. Kafka only accepts this while no consumer of the group runs.
kafka-reset:
	@if kubectl get namespace cec >/dev/null 2>&1; then \
	  echo "App is running: Kafka positions left as they are"; \
	else \
	  $(RESET_OFFSETS) experiment group16-k8s-store group16-k8s-notify; \
	fi

# Deleting the namespace removes the whole app, even when KEDA is already gone
# or .env is missing (kubectl delete -k would fail on those). Pods on a node
# that is down never confirm their deletion and would block it, so they are
# force-removed after 60 s.
stop:
	kubectl delete namespace cec --ignore-not-found --wait=false
	@for i in $$(seq 1 60); do kubectl get namespace cec >/dev/null 2>&1 || break; sleep 1; done; \
	if kubectl get namespace cec >/dev/null 2>&1; then \
	  echo "Force-removing pods stuck on unreachable nodes"; \
	  kubectl -n cec delete pods --all --force --grace-period=0 >/dev/null 2>&1 || true; \
	  kubectl wait --for=delete namespace/cec --timeout=120s; \
	fi
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
	-docker exec $(POSTGRES) sh -c 'psql -U "$$POSTGRES_USER" -d "$$POSTGRES_DB" -c "TRUNCATE experiments CASCADE"'
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

watch:
	watch -n 2 'kubectl -n cec get hpa,scaledobject; echo; kubectl -n cec get pods -l "app in (rest-api,store-consumer)" -o wide; echo; kubectl top nodes'

# Queries the newest experiment (or EXPERIMENT=<id>) with hey; the rest-api HPA
# adds pods above 500m CPU per pod and removes them ~5 min after the load ends
stress-rest:
	@EXP="$(EXPERIMENT)"; \
	[ -n "$$EXP" ] || EXP=$$(docker exec $(POSTGRES) sh -c 'psql -U "$$POSTGRES_USER" -d "$$POSTGRES_DB" -tAc "SELECT experiment_id FROM experiments ORDER BY started_at DESC NULLS LAST LIMIT 1"'); \
	[ -n "$$EXP" ] || { echo "No experiment found; pass one: make stress-rest EXPERIMENT=<id>"; exit 1; }; \
	echo "Load: $(CONCURRENCY) parallel requests for $(DURATION) on experiment $$EXP"; \
	kubectl -n cec get hpa rest-api; \
	docker run --rm williamyeh/hey -z $(DURATION) -c $(CONCURRENCY) "$(REST_URL)/temperature/out-of-range?experiment-id=$$EXP"; \
	kubectl -n cec get hpa rest-api

# Uses the experiment with the most measurements (or EXPERIMENT=<id>): the largest response
bench-rest:
	@EXP="$(EXPERIMENT)"; \
	[ -n "$$EXP" ] || EXP=$$(docker exec $(POSTGRES) sh -c 'psql -U "$$POSTGRES_USER" -d "$$POSTGRES_DB" -tAc "SELECT experiment_id FROM measurement_averages GROUP BY experiment_id ORDER BY COUNT(*) DESC LIMIT 1"'); \
	[ -n "$$EXP" ] || { echo "No experiment found; pass one: make bench-rest EXPERIMENT=<id>"; exit 1; }; \
	scripts/bench-rest.sh "$(REST_URL)" "$$EXP"

# experiment is read-only for us, so store-consumer temporarily reads the
# group16 topic (in its own consumer group, moved to the newest message first so
# old test data isn't replayed) and scales at a lower lag. The trap restores the
# normal setup afterwards, also on errors and Ctrl+C. notify-consumer keeps
# reading experiment, so the test data never causes notifications.
stress-kafka:
	@set -e; \
	$(RESET_OFFSETS) group16 group16-k8s-stress-store; \
	trap 'echo "Restoring the normal setup"; kubectl apply -k "$(CURDIR)" >/dev/null; kubectl -n cec rollout status deploy/store-consumer --timeout=180s' EXIT; \
	kubectl -n cec patch deploy store-consumer --type=json \
	  -p '[{"op":"replace","path":"/spec/template/spec/containers/0/args","value":["group16","group16-k8s-stress-store"]}]'; \
	kubectl -n cec patch scaledobject store-consumer --type=json \
	  -p '[{"op":"replace","path":"/spec/triggers/0/metadata/threshold","value":"$(STRESS_LAG)"}]'; \
	kubectl -n cec rollout status deploy/store-consumer --timeout=180s; \
	cd "$(CURDIR)/useful-services/experiment-producer"; \
	TOPIC=group16 AUTH_DIR="$(CURDIR)/auth" docker compose --profile $(PROFILE) up; \
	TOPIC=group16 docker compose --profile $(PROFILE) down; \
	echo "Producer finished; waiting until the lag is worked off (Ctrl+C to skip)"; \
	for i in $$(seq 1 120); do \
	  lag=$$(kubectl -n cec get hpa keda-hpa-store-consumer -o jsonpath='{.status.currentMetrics[0].external.current.averageValue}'); \
	  pods=$$(kubectl -n cec get deploy store-consumer -o jsonpath='{.status.readyReplicas}'); \
	  echo "$$(date +%T)  lag per pod: $$lag  store-consumer pods: $$pods"; \
	  [ "$$lag" = "0" ] && break; sleep 5; \
	done
