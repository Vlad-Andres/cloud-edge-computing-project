FROM python:3.12-slim

WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
COPY app.py .

# The workers' metric files (see app.py); empty in every new container
ENV PROMETHEUS_MULTIPROC_DIR=/tmp/prometheus
RUN mkdir -p /tmp/prometheus

EXPOSE 3003 9091
# granian = production server with a Rust HTTP layer that keeps client connections open.
# RSGI: granian's own async interface; GRANIAN_WORKERS processes, each an event loop
# (uvloop) with up to 4 database connections (max_size in app.py). --metrics: granian's
# own metrics on port 9091
ENV GRANIAN_WORKERS=2
CMD ["granian", "--interface", "rsgi", "--host", "0.0.0.0", "--port", "3003", "--http", "1", "--respawn-failed-workers", "--metrics", "--metrics-address", "0.0.0.0", "--metrics-port", "9091", "--metrics-scrape-interval", "5", "app:app"]
