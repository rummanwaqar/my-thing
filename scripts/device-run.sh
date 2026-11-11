#!/bin/sh
# Cargo runner for aarch64: copy the binary to the Car Thing and run it there.
# Usage (via cargo): cargo device-run [-- app args]
# Env: DEVICE (default superbird-kiosk.local), SLINT_BACKEND (default
# linuxkms-software: crisper text than femtovg under rotation), SLINT_DEBUG_PERFORMANCE
# (forwarded if set).
set -eu

bin="$1"; shift
dev="root@${DEVICE:-superbird-kiosk.local}"
dir=/var/lib/my-thing

ssh "$dev" "mkdir -p $dir && date -s @$(date +%s) >/dev/null"
scp -q "$bin" "$dev:$dir/my-thing"

perf=""
[ -n "${SLINT_DEBUG_PERFORMANCE-}" ] && perf="SLINT_DEBUG_PERFORMANCE=$SLINT_DEBUG_PERFORMANCE"

exec ssh -t "$dev" "systemctl stop chromium-kiosk superbird-weston superbird-kiosk-daemon; \
cd $dir && SLINT_BACKEND=${SLINT_BACKEND:-linuxkms-software} SLINT_KMS_ROTATION=90 $perf ./my-thing $*"
