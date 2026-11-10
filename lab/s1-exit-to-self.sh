#!/usr/bin/env bash
# M0 spike S1: exit-to-self attach (docs/spikes/S1-exit-to-self.md).
# Runs inside the El Tor regtest lab guest (testnet/labctl, SovereignTechnology/eltord fe6b8d6c4251) after
# `labctl up`, as the lab user, with the lab's default path 006m,007m,004r (004r = the exit, our "seeder").
#   A  baseline: Chutney's permissive exit policy, labctl's own checksummed fetch through the paid-only client.
#   B  the exit's policy narrowed to the origin only ("exit-to-self"); Tor attaches streams itself.
#   C  same policy; __LeaveStreamsUnattached=1 and we ATTACHSTREAM the stream to the pinned paid circuit.
# Prints evidence and one RESULT line per run. Restores the client's stream setting at the end; the exit keeps the
# narrow policy (wipe or `labctl down && up` to reset).
set -uo pipefail
export ELTOR_LAB_STATE=${ELTOR_LAB_STATE:-/lab}
L=(python3 /src/eltord/testnet/labctl)
EXIT=004r CLIENT=005c BYTES=1048576
NODES=$ELTOR_LAB_STATE/net/nodes
FP=$(awk '{print toupper($2)}' "$NODES/$EXIT/fingerprint")
POLICY='accept 127.0.0.1:18880,reject *:*'
log() { echo "[$(date -u +%H:%M:%SZ)] $*"; }
ctl() { "${L[@]}" ctl "$@"; }
strip() { sed -E 's/^[a-z-]+-status=//'; }
fetch() { "${L[@]}" fetch "$BYTES" --timeout "$1"; }
paid_circuit() {  # BUILT circuit whose path is exactly 006m,007m,004r
  ctl "$CLIENT" "GETINFO circuit-status" | strip |
    awk '$2 == "BUILT" && $3 ~ /~006m,[^,]*~007m,[^,]*~004r$/ { print $1 }' | tail -1
}

log "S1 exit $EXIT fingerprint $FP"
log "A: baseline (permissive exit)"
fetch 120; echo "RESULT A rc=$?"

log "B: narrowing $EXIT to: $POLICY"
ctl "$EXIT" "SETCONF ExitPolicy=\"$POLICY\"" && ctl "$EXIT" "GETINFO exit-policy/full" | head -5
seen=0
for i in $(seq 120); do
  if ctl "$CLIENT" "GETINFO desc/id/$FP" 2>/dev/null | grep -q '^accept 127.0.0.1:18880'; then seen=$i; break; fi
  sleep 2
done
log "client's copy of the $EXIT descriptor carries the narrow policy: $([ "$seen" -gt 0 ] && echo "yes, after ~$((seen * 2)) s" || echo NO)"
ctl "$CLIENT" "GETINFO desc/id/$FP" | grep -E '^(accept|reject)' | head -4
log "paid circuit before B: $(paid_circuit)"
fetch 90; echo "RESULT B rc=$?"

log "C: we attach streams ourselves"
ctl "$CLIENT" "SETCONF __LeaveStreamsUnattached=1"
CID=$(paid_circuit)
log "paid circuit: ${CID:-none}"
ctl "$CLIENT" "GETINFO circuit-status" | strip | grep -E "^$CID " || true
"${L[@]}" fetch "$BYTES" --timeout 90 > "$ELTOR_LAB_STATE/tmp/s1c.json" 2>&1 &
F=$!
SID=
for i in $(seq 30); do
  SID=$(ctl "$CLIENT" "GETINFO stream-status" | strip | awk '$4 ~ /^127\.0\.0\.1:18880$/ { print $1 }' | head -1)
  [ -n "$SID" ] && break
  sleep 1
done
log "pending stream: ${SID:-none}"
ctl "$CLIENT" "GETINFO stream-status" | strip
if [ -n "$SID" ] && [ -n "$CID" ]; then
  ctl "$CLIENT" "ATTACHSTREAM $SID $CID"; echo "ATTACHSTREAM rc=$?"
fi
wait "$F"; rc=$?
cat "$ELTOR_LAB_STATE/tmp/s1c.json"; echo "RESULT C rc=$rc"
ctl "$CLIENT" "SETCONF __LeaveStreamsUnattached=0"

log "evidence: settlement and the client's log"
"${L[@]}" status 2>&1 | tail -15
grep -h -E 'might support|doomed exit|ATTACHSTREAM|attach|exit policy|RESOURCELIMIT' "$NODES/$CLIENT"/*.log 2>/dev/null | tail -25
