# cloud-edge-computing-project

## Monitoring service

```sh
docker run --rm -d --init \
  --network cec \
  --name store-consumer \
  -p 9091:9091 \
  --env-file .env \
  -v "$(pwd)/auth":/usr/src/app/auth \
  store-consumer-img "experiment" "group16-store"
```