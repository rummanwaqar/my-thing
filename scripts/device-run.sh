#!/bin/sh
# Cargo runner for aarch64: copy the binary to the Car Thing and run it there in the
# foreground, in place of the installed my-thing service. The service comes back on exit.
# Usage (via cargo): cargo device-run [-- app args]
# Env: DEVICE (default my-thing.local), SLINT_DEBUG_PERFORMANCE (forwarded if set).
set -eu

bin="$1"; shift
dev="root@${DEVICE:-my-thing.local}"
# Not /var/lib/my-thing: that's the service's state dir.
dir=/var/lib/my-thing-dev

ssh "$dev" "mkdir -p $dir && date -s @$(date +%s) >/dev/null"
scp -q "$bin" "$dev:$dir/my-thing"

perf=""
[ -n "${SLINT_DEBUG_PERFORMANCE-}" ] && perf="SLINT_DEBUG_PERFORMANCE=$SLINT_DEBUG_PERFORMANCE"

# Weston already runs on the my-thing image; only the installed app has to make room.
exec ssh -t "$dev" "trap 'systemctl start my-thing' EXIT INT TERM HUP; \
systemctl stop my-thing; \
for i in \$(seq 1 100); do [ -S /run/weston/wayland-1 ] && break; sleep 0.05; done; \
cd $dir && XDG_RUNTIME_DIR=/run/weston WAYLAND_DISPLAY=wayland-1 SLINT_FULLSCREEN=1 $perf ./my-thing $*"
