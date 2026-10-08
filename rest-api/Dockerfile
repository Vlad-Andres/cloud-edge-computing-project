FROM python:3.12-slim

WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
COPY app.py .

EXPOSE 3003
# gunicorn = production server with 4 worker processes (Flask's dev server is single-threaded)
CMD ["gunicorn", "-w", "4", "-b", "0.0.0.0:3003", "app:app"]

# -w 4: 4 worker processes; this is a starting default value, can be tuned
