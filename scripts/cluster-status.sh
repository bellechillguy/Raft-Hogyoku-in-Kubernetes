#!/bin/sh
set -eu

namespace=${NAMESPACE:-hogyoku}
base_port=${STATUS_PORT:-18081}
pods=$(kubectl -n "$namespace" get pods -l app.kubernetes.io/name=raft -o jsonpath='{range .items[*]}{.metadata.name}{"\n"}{end}')
forward_pid=""

cleanup() {
    if [ -n "$forward_pid" ]; then
        kill "$forward_pid" 2>/dev/null || true
        wait "$forward_pid" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

for pod in $pods; do
    ordinal=${pod##*-}
    port=$((base_port + ordinal))
    log_file="/tmp/hogyoku-port-forward-${pod}.log"
    kubectl -n "$namespace" port-forward "pod/${pod}" "${port}:8081" >"$log_file" 2>&1 &
    forward_pid=$!
    sleep 1
    printf '%s\t' "$pod"
    if ! curl -fsS "http://127.0.0.1:${port}/status"; then
        printf '{"error":"status unavailable"}'
    fi
    printf '\n'
    kill "$forward_pid" 2>/dev/null || true
    wait "$forward_pid" 2>/dev/null || true
    forward_pid=""
done
