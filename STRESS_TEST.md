# Stress test commands

```bash
# From the repository root
docker network create cec 2>/dev/null || true

# IF not existent
DOCKER_BUILDKIT=1 docker build --build-arg BIN=store-consumer \
  -t store-consumer-img consumer
DOCKER_BUILDKIT=1 docker build --build-arg BIN=notify-consumer \
  -t notify-consumer-img consumer

# Start the database and Kafka first, then start both consumers
docker run --rm -d --init --network cec \
  --name store-consumer --env-file .env \
  -v "$(pwd)/auth":/usr/src/app/auth:ro \
  -p 9091:9091 store-consumer-img group16 group16-stress-store

# docker run --rm -d --init --network cec \
#   --name notify-consumer --env-file .env \
#   -v "$(pwd)/auth":/usr/src/app/auth:ro \
#   -p 9092:9091 notify-consumer-img group16 group16-stress-notify

# Expected peaks: baseline 80/s, ramp 80/s, burst 120/s, over-100 200/s.
# Details: useful-services/experiment-producer/loads/README.md
# Run one profile: baseline, ramp, burst, or over-100
cd useful-services/experiment-producer
TOPIC=group16 \
BROKERS=kafka.cec.dlandau.nl:19092,kafka.cec.dlandau.nl:29092,kafka.cec.dlandau.nl:39092 \
  docker compose --profile burst up

# Replace ramp above with baseline, burst, or over-100 for the other tests

# Watch consumers, in separate terminals
docker logs -f store-consumer
# docker logs -f notify-consumer

# Cleanup
docker compose down
cd ../..
docker rm -f store-consumer 2>/dev/null || true
```