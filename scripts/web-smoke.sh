#!/usr/bin/env bash
# Load the web build in Chrome four times and fail on any GPU validation error, shader error, panic,
# or exception (see scripts/web-smoke.mjs):
#
#   1. on WebGPU, building a new 3D view six seconds in (?late_view=6);
#   2. on WebGPU, destroying the GPU device at three seconds (?lose_device=3) and then building a new
#      3D view;
#   3. on WebGPU in headless Chrome, which loses the device by itself, building the same view;
#   4. forced onto WebGL 2, building the same late view.
#
#   scripts/web-smoke.sh <dist-dir> <screenshot-dir>
#
# CHROME is the browser command (default: google-chrome). Chrome's headless mode cannot present a
# WebGPU canvas (it has no shared-image backing for the swap chain, so the browser loses the device
# about a second after start), so the WebGPU passes run a headed Chrome on a virtual X display
# (XVFB, default "xvfb-run -a"; set it empty to run headless). Chrome's software WebGPU adapter
# sometimes fails to start; the WebGPU passes retry until they get it.
set -euo pipefail
dist=$1
shots=$2
chrome=${CHROME:-google-chrome}
xvfb=${XVFB-xvfb-run -a}
http_port=8737
devtools_port=9337
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$shots"

python3 -m http.server "$http_port" --bind 127.0.0.1 --directory "$dist" >/dev/null 2>&1 &
server=$!
trap 'kill "$server" 2>/dev/null || true' EXIT
sleep 1

run() { # <name> <renderer> <query> <attempt> <wrapper command or empty> <chrome flags...>
  local name=$1 renderer=$2 query=$3 attempt=$4 wrapper=$5
  shift 5
  local profile log="$shots/chrome-$name-$attempt.log"
  profile=$(mktemp -d)
  local headless=--headless=new
  [ -z "$wrapper" ] || headless=
  # shellcheck disable=SC2086
  $wrapper $chrome $headless --no-first-run --no-default-browser-check --user-data-dir="$profile" \
    --remote-debugging-port="$devtools_port" --enable-logging=stderr "$@" about:blank >"$log" 2>&1 &
  local browser=$!
  local status=0
  # A pass that never answers (a browser that failed to start, a page that never settles) fails
  # after three minutes instead of holding the job.
  timeout 180 node "$here/web-smoke.mjs" "$devtools_port" "http://127.0.0.1:$http_port/?script=takeoff.jsonl&$query" 25000 \
    "$shots/$name-$attempt.png" "$renderer" || status=$?
  kill "$browser" 2>/dev/null || true
  wait "$browser" 2>/dev/null || true
  # The wrapper can leave the browser itself and its virtual display behind.
  pkill -f "remote-debugging-port=$devtools_port" 2>/dev/null || true
  [ -z "$wrapper" ] || pkill Xvfb 2>/dev/null || true
  sleep 1
  # Chrome's helper processes can outlive the browser for a moment and keep writing the
  # profile; retry briefly, and never let temporary cleanup decide the result.
  for _ in 1 2 3 4 5; do
    rm -rf "$profile" 2>/dev/null && break
    sleep 1
  done
  return "$status"
}

webgpu_flags=(--enable-unsafe-webgpu --enable-features=Vulkan --use-vulkan=swiftshader --disable-vulkan-surface
  --ignore-gpu-blocklist --use-webgpu-adapter=swiftshader)

webgpu() { # <name> <query> <wrapper> <flags...>: retried, because the software adapter can fail to start
  local name=$1 query=$2 wrapper=$3
  shift 3
  for attempt in 1 2 3 4 5; do
    if run "$name" WebGPU "gpu=webgpu&$query" "$attempt" "$wrapper" "$@"; then
      return 0
    fi
    echo "web-smoke: WebGPU pass $name attempt $attempt did not pass"
  done
  echo "web-smoke: no WebGPU pass $name in 5 attempts"
  return 1
}

webgpu_status=0
webgpu webgpu late_view=6 "$xvfb" "${webgpu_flags[@]}" || webgpu_status=1
webgpu webgpu-device-destroyed "lose_device=3&late_view=6" "$xvfb" "${webgpu_flags[@]}" || webgpu_status=1
# Headless Chrome loses the device by itself about a second after start (see above). Whether it does
# is Chrome's business, so this pass accepts either; what it checks is that a new 3D view built
# after a real loss neither panics nor throws.
SMOKE_LOSS_OK=1 webgpu webgpu-headless late_view=6 "" --enable-unsafe-webgpu --enable-features=Vulkan,WebGPU \
  --use-angle=swiftshader --enable-unsafe-swiftshader --use-webgpu-adapter=swiftshader || webgpu_status=1
# Software WebGL 2 can also fail to start; retry it like the WebGPU passes.
webgl2_status=1
for attempt in 1 2 3; do
  if run webgl2 "WebGL 2" "gpu=webgl2&late_view=6" "$attempt" "" --use-angle=swiftshader --enable-unsafe-swiftshader; then
    webgl2_status=0
    break
  fi
  echo "web-smoke: WebGL 2 pass attempt $attempt did not pass"
done
if [ "$webgpu_status" != 0 ] || [ "$webgl2_status" != 0 ]; then
  exit 1
fi
echo "web-smoke: WebGPU (live, destroyed and headless-lost device) and WebGL 2 all clean"
