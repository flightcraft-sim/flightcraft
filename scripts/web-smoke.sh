#!/usr/bin/env bash
# Load the web build in headless Chrome twice, once on WebGPU and once forced onto WebGL 2, and fail
# on any GPU validation error, shader error, panic, or exception (see scripts/web-smoke.mjs).
#
#   scripts/web-smoke.sh <dist-dir> <screenshot-dir>
#
# CHROME is the browser command (default: google-chrome). Headless WebGPU uses Chrome's software
# adapter, which sometimes fails to start; the WebGPU pass retries until it gets that adapter.
set -euo pipefail
dist=$1
shots=$2
chrome=${CHROME:-google-chrome}
http_port=8737
devtools_port=9337
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$shots"

python3 -m http.server "$http_port" --bind 127.0.0.1 --directory "$dist" >/dev/null 2>&1 &
server=$!
trap 'kill "$server" 2>/dev/null || true' EXIT
sleep 1

run() { # <renderer> <gpu query value> <attempt> <extra chrome flags...>
  local renderer=$1 gpu=$2 attempt=$3
  shift 3
  local profile
  profile=$(mktemp -d)
  # shellcheck disable=SC2086
  $chrome --headless=new --no-first-run --no-default-browser-check --user-data-dir="$profile" \
    --use-angle=swiftshader --enable-unsafe-swiftshader --remote-debugging-port="$devtools_port" \
    "$@" about:blank >/dev/null 2>&1 &
  local browser=$!
  local status=0
  node "$here/web-smoke.mjs" "$devtools_port" "http://127.0.0.1:$http_port/?script=takeoff.jsonl&gpu=$gpu" 20000 \
    "$shots/${renderer// /}-$attempt.png" "$renderer" || status=$?
  kill "$browser" 2>/dev/null || true
  wait "$browser" 2>/dev/null || true
  rm -rf "$profile"
  return "$status"
}

webgpu_ok=0
for attempt in 1 2 3 4 5; do
  if run WebGPU webgpu "$attempt" --enable-unsafe-webgpu --enable-features=Vulkan,WebGPU --use-webgpu-adapter=swiftshader; then
    webgpu_ok=1
    break
  fi
  echo "web-smoke: WebGPU attempt $attempt did not pass"
done
run "WebGL 2" webgl2 1
if [ "$webgpu_ok" != 1 ]; then
  echo "web-smoke: no WebGPU pass in 5 attempts"
  exit 1
fi
echo "web-smoke: WebGPU and WebGL 2 both clean"
