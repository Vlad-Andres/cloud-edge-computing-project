"""Moves consumer groups to the newest message of every partition of a topic.

make start runs this before deploying, so a new cluster run starts at the
current events instead of replaying everything since the last run. Kafka only
accepts it while no consumer of the group is running.

Usage (in a container with the group's credentials mounted at /auth):
    python reset-kafka-offsets.py <topic> <group> [<group> ...]
"""
import sys

from confluent_kafka import Consumer, TopicPartition

BROKERS = "kafka.cec.dlandau.nl:19092,kafka.cec.dlandau.nl:29092,kafka.cec.dlandau.nl:39092"

topic, groups = sys.argv[1], sys.argv[2:]
config = {
    "bootstrap.servers": BROKERS,
    "security.protocol": "ssl",
    "ssl.ca.location": "/auth/ca.crt",
    "ssl.keystore.location": "/auth/kafka.keystore.pkcs12",
    "ssl.keystore.password": "cc2023",
    "ssl.endpoint.identification.algorithm": "none",
    "enable.auto.commit": False,
}

for group in groups:
    consumer = Consumer({**config, "group.id": group})
    partitions = consumer.list_topics(topic, timeout=15).topics[topic].partitions
    newest = [
        TopicPartition(topic, p, consumer.get_watermark_offsets(TopicPartition(topic, p), timeout=15)[1])
        for p in partitions
    ]
    consumer.commit(offsets=newest, asynchronous=False)
    print(f"{group}: starts at the newest message on {len(newest)} partitions of {topic}")
    consumer.close()
