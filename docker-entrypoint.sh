#!/bin/sh
set -eu

: "${POD_NAME:=${HOSTNAME}}"
: "${POD_NAMESPACE:=hogyoku}"
: "${NODE_IDS:=1,2,3,4,5}"
: "${RAFT_PORT:=8000}"
: "${HEALTH_PORT:=8081}"
: "${RAFT_DATA_DIR:=/data}"
: "${INITIAL_CLUSTER_SIZE:=3}"
: "${CONTACT_NODE_ADDRESS:=raft-0.raft-svc:8000}"

ordinal=${POD_NAME##*-}
case "$ordinal" in
    ''|*[!0-9]*)
        echo "Cannot derive StatefulSet ordinal from POD_NAME=$POD_NAME" >&2
        exit 1
        ;;
esac

target=$((ordinal + 1))
position=0
node_id=""
old_ifs=$IFS
IFS=','
for candidate in $NODE_IDS; do
    position=$((position + 1))
    if [ "$position" -eq "$target" ]; then
        node_id=$candidate
        break
    fi
done
IFS=$old_ifs

if [ -z "$node_id" ]; then
    echo "NODE_IDS does not contain an ID for ordinal $ordinal" >&2
    exit 1
fi

if [ -z "${PEERS:-}" ]; then
    echo "PEERS must be supplied by the Raft ConfigMap" >&2
    exit 1
fi

advertise_address="${POD_NAME}.raft-svc.${POD_NAMESPACE}.svc.cluster.local:${RAFT_PORT}"

echo "Starting Raft node ${node_id} at ${advertise_address}"

if [ "$ordinal" -ge "$INITIAL_CLUSTER_SIZE" ]; then
    exec /usr/local/bin/server \
        --ip 0.0.0.0 \
        --port "$RAFT_PORT" \
        --health-port "$HEALTH_PORT" \
        --id "$node_id" \
        --peers "$PEERS" \
        --advertise-address "$advertise_address" \
        --data-dir "$RAFT_DATA_DIR" \
        --contact-node-address "$CONTACT_NODE_ADDRESS"
fi

exec /usr/local/bin/server \
    --ip 0.0.0.0 \
    --port "$RAFT_PORT" \
    --health-port "$HEALTH_PORT" \
    --id "$node_id" \
    --peers "$PEERS" \
    --advertise-address "$advertise_address" \
    --data-dir "$RAFT_DATA_DIR"
