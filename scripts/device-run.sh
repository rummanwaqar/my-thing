#!/bin/sh
# Cargo runner for aarch64: copy the binary to the Car Thing and run it there,
# as a fullscreen client of the image's Weston (which rotates the panel).
# Usage (via cargo): cargo device-run [-- app args]
# Env: DEVICE (default superbird-kiosk.local), SLINT_DEBUG_PERFORMANCE (forwarded if set).
set -eu

bin="$1"; shift
dev="root@${DEVICE:-superbird-kiosk.local}"
dir=/var/lib/my-thing

ssh "$dev" "mkdir -p $dir && date -s @$(date +%s) >/dev/null"
scp -q "$bin" "$dev:$dir/my-thing"

perf=""
[ -n "${SLINT_DEBUG_PERFORMANCE-}" ] && perf="SLINT_DEBUG_PERFORMANCE=$SLINT_DEBUG_PERFORMANCE"

# Starting Weston also starts Chromium (it's WantedBy Weston), so stop Chromium after.
exec ssh -t "$dev" "systemctl start superbird-weston; \
systemctl stop chromium-kiosk superbird-kiosk-daemon; \
for i in \$(seq 1 100); do [ -S /run/weston/wayland-1 ] && break; sleep 0.05; done; \
cd $dir && XDG_RUNTIME_DIR=/run/weston WAYLAND_DISPLAY=wayland-1 SLINT_FULLSCREEN=1 $perf ./my-thing $*"
