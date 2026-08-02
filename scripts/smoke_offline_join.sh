#!/usr/bin/env bash
# Offline join smoke: build, start server, verify TCP + playability unit test.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "== build =="
cargo build -p rustbound-server --release

TMP="$(mktemp -d)"
cleanup() {
  if [[ -n "${SERVER_PID:-}" ]]; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT

PORT="${RUSTBOUND_SMOKE_PORT:-25566}"
cat > "$TMP/server.properties" <<PROP
server-ip=127.0.0.1
server-port=${PORT}
online-mode=false
max-players=8
motd=Rustbound smoke
gamemode=1
view-distance=8
level-name=${TMP}/world
network-compression-threshold=256
autosave-interval=60
PROP

echo "== start server on :${PORT} =="
./target/release/rustbound-server --config "$TMP/server.properties" >"$TMP/server.log" 2>&1 &
SERVER_PID=$!
sleep 1

if ! kill -0 "$SERVER_PID" 2>/dev/null; then
  echo "FAIL: server exited early"
  cat "$TMP/server.log"
  exit 1
fi

echo "== TCP check =="
python3 - "$PORT" <<'PY'
import socket, sys
port = int(sys.argv[1])
s = socket.create_connection(("127.0.0.1", port), timeout=3)
s.close()
print(f"OK: accepted on {port}")
PY

echo "== playability unit smoke =="
cargo test -p rustbound-server --lib server_offline_playability_smoke -- --nocapture

echo "OK: offline smoke passed"
echo "Manual next step: Minecraft Java 1.20.1 offline client -> 127.0.0.1:${PORT}"
tail -n 20 "$TMP/server.log" || true
