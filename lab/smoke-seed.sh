#!/usr/bin/env bash
# Seeder and viewer smoke test, no El Tor: package a generated clip, sign a catalog, serve it on loopback, fetch
# the catalog, the hash list and every file over HTTP and check each against its sha256; then run `watch` (direct)
# and decode the whole video through its local HLS proxy with ffmpeg, as a player would.
# Needs ffmpeg, ffprobe, curl and a built `nutflix-eltor` (BIN, default target/debug/nutflix-eltor).
# Runs in a test VM; never on a laptop (see CONTRIBUTING).
set -euo pipefail

BIN=${BIN:-target/debug/nutflix-eltor}
W=$(mktemp -d)
trap 'kill "${SERVE_PID:-0}" "${WATCH_PID:-0}" 2>/dev/null || true; rm -rf "$W"' EXIT
FP=ABCDEF0123456789ABCDEF0123456789ABCDEF01
PORT=${PORT:-18443}

ffmpeg -loglevel error -f lavfi -i testsrc=size=640x360:rate=30 -f lavfi -i sine=frequency=440:sample_rate=48000 \
  -t 12 -c:v libx264 -pix_fmt yuv420p -c:a aac -shortest "$W/clip.mp4"

"$BIN" package "$W/clip.mp4" --store "$W/store" --video-id smoke-clip --title "Smoke clip" --entry "$W/entry.json"
"$BIN" key new --out "$W/catalog.key" > "$W/pubkey"
"$BIN" catalog sign --key "$W/catalog.key" --seeder "$FP" --port "$PORT" --valid-for 3600 --out "$W/catalog.json" \
  "$W/entry.json"
CONTACT=$("$BIN" marker --key "$W/catalog.key" --port "$PORT")
"$BIN" catalog verify "$W/catalog.json" --seeder "$FP" --contact "ops nobody $CONTACT"

"$BIN" serve --store "$W/store" --catalog "$W/catalog.json" --listen "127.0.0.1:$PORT" &
SERVE_PID=$!
for _ in $(seq 50); do curl -fsS "http://127.0.0.1:$PORT/catalog.json" -o /dev/null 2>/dev/null && break; sleep 0.1; done

get() {  # get <sha256> -> file in $W/got, checked
  curl -fsS "http://127.0.0.1:$PORT/$1" -o "$W/got/$1"
  [ "$(sha256sum < "$W/got/$1" | cut -d' ' -f1)" = "$1" ] || { echo "FAIL: $1 hash mismatch"; exit 1; }
}
mkdir -p "$W/got"
curl -fsS "http://127.0.0.1:$PORT/catalog.json" | cmp - "$W/catalog.json"
ROOT=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["videos"][0]["root"])' "$W/catalog.json")
get "$ROOT"
n=0
for f in $(python3 -I -c 'import json,sys; [print(f["sha256"]) for f in json.load(open(sys.argv[1]))["files"]]' "$W/got/$ROOT"); do
  get "$f"; n=$((n + 1))
done
code=$(curl -s --path-as-is -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/../$ROOT")
[ "$code" = 404 ] || { echo "FAIL: traversal answered $code"; exit 1; }
echo "origin: catalog bound to $FP, hash list $ROOT, $n files fetched and verified"

"$BIN" watch --origin "127.0.0.1:$PORT" --direct yes --seeder "$FP" --contact "$CONTACT" \
  --terms "$("$BIN" terms encode --rate-msat 1000 --bitrate 3000000)" > "$W/watch.url" 2> "$W/watch.log" &
WATCH_PID=$!
for _ in $(seq 100); do [ -s "$W/watch.url" ] && break; sleep 0.1; done
URL=$(cat "$W/watch.url")
case $URL in http://127.0.0.1:*/"$ROOT"/master.m3u8) ;; *) echo "FAIL: watch printed '$URL'"; cat "$W/watch.log"; exit 1 ;; esac
ffmpeg -loglevel error -i "$URL" -f null - 2> "$W/decode.log" || { echo "FAIL: decode"; cat "$W/decode.log"; exit 1; }
[ ! -s "$W/decode.log" ] || { echo "FAIL: decoder complained"; cat "$W/decode.log"; exit 1; }
dur=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$URL")
echo "watch: decoded the whole video through the local proxy (${dur}s)"
echo "PASS"
