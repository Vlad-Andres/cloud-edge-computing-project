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


## Data retention

Run every day at 3

```sh
(sudo crontab -l 2>/dev/null; \
  echo "0 3 * * * $(pwd)/scripts/cleanup-retention.sh >> /var/log/cleanup-retention.log 2>&1") \
  | sudo crontab -
```

 To check the existing cronjobs:

```sh
sudo crontab -l
```
