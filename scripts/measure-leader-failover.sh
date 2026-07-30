#!/bin/sh
set -eu

namespace=${NAMESPACE:-hogyoku}
base_port=${STATUS_PORT:-19081}
timeout_seconds=${TIMEOUT_SECONDS:-10}

pods=$(kubectl -n "$namespace" get pods \
    -l app.kubernetes.io/name=raft \
    -o jsonpath='{range .items[*]}{.metadata.name}{"\n"}{end}')

if [ -z "$pods" ]; then
    echo "No Raft pods found in namespace $namespace" >&2
    exit 1
fi

forward_pids=""

cleanup() {
    for pid in $forward_pids; do
        kill "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
    done
}
trap cleanup EXIT INT TERM

for pod in $pods; do
    ordinal=${pod##*-}
    port=$((base_port + ordinal))
    log_file="/tmp/hogyoku-failover-${pod}.log"
    kubectl -n "$namespace" port-forward "pod/${pod}" "${port}:8081" \
        >"$log_file" 2>&1 &
    forward_pids="$forward_pids $!"
done

get_leader() {
    for pod in $pods; do
        ordinal=${pod##*-}
        port=$((base_port + ordinal))
        status=$(curl -fsS --max-time 0.5 \
            "http://127.0.0.1:${port}/status" 2>/dev/null || true)
        case "$status" in
            *'"role":"leader"'*)
                printf '%s\n' "$pod"
                return 0
                ;;
        esac
    done
    return 1
}

now_ms() {
    python3 -c 'import time; print(int(time.time() * 1000))'
}

attempt=0
old_leader=""
while [ "$attempt" -lt 20 ] && [ -z "$old_leader" ]; do
    old_leader=$(get_leader || true)
    if [ -z "$old_leader" ]; then
        sleep 0.25
    fi
    attempt=$((attempt + 1))
done

if [ -z "$old_leader" ]; then
    echo "Could not find the current leader" >&2
    exit 1
fi

start_ms=$(now_ms)
deadline_ms=$((start_ms + timeout_seconds * 1000))
echo "Old leader : $old_leader"
echo "Deleting   : $old_leader"
kubectl -n "$namespace" delete pod "$old_leader" \
    --grace-period=0 \
    --force \
    --wait=false

new_leader=""
while [ "$(now_ms)" -lt "$deadline_ms" ]; do
    candidate=$(get_leader || true)
    if [ -n "$candidate" ] && [ "$candidate" != "$old_leader" ]; then
        new_leader=$candidate
        break
    fi
    sleep 0.1
done

end_ms=$(now_ms)

if [ -z "$new_leader" ]; then
    echo "No different leader appeared within ${timeout_seconds} seconds" >&2
    exit 1
fi

elapsed=$(python3 -c \
    'import sys; print(f"{(int(sys.argv[2]) - int(sys.argv[1])) / 1000:.3f}")' \
    "$start_ms" "$end_ms")

echo "New leader : $new_leader"
echo "Recovery   : ${elapsed} seconds"
