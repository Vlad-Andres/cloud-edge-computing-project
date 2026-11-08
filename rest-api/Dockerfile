FROM python:3.12-slim

WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
COPY app.py .

ENV PROMETHEUS_MULTIPROC_DIR=/tmp/prometheus
RUN mkdir -p /tmp/prometheus

EXPOSE 3003 9091
ENV GRANIAN_WORKERS=2
CMD ["granian", "--interface", "rsgi", "--host", "0.0.0.0", "--port", "3003", "--http", "1", "--respawn-failed-workers", "--metrics", "--metrics-address", "0.0.0.0", "--metrics-port", "9091", "--metrics-scrape-interval", "5", "app:app"]
