FROM python:3.12-slim

WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
COPY app.py .

EXPOSE 3003
# granian = production server with a Rust HTTP layer that keeps client connections open
# (Flask's dev server is single-threaded). 2 worker processes with 2 threads each, one
# database connection per thread (max_size in app.py)
CMD ["granian", "--interface", "wsgi", "--host", "0.0.0.0", "--port", "3003", "--http", "1", "--workers", "2", "--blocking-threads", "2", "--respawn-failed-workers", "app:app"]
