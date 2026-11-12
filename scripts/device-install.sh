#!/bin/sh
# Permanently install the current code on the Car Thing, without a reflash.
# Builds for the device, strips the binary, copies it to /opt/my-thing/bin (the bandaid
# partition: survives reboots, but a full reflash resets it to the image's binary), keeps
# the old one as my-thing.previous, and restarts the service.
# Usage: scripts/device-install.sh             build + install
#        scripts/device-install.sh --rollback  swap .current and .previous back
# Env: DEVICE (default my-thing.local)
set -eu
cd "$(dirname "$0")/.."

dev="root@${DEVICE:-my-thing.local}"
opt=/opt/my-thing/bin

if [ "${1-}" = "--rollback" ]; then
    ssh "$dev" "set -e; cd $opt; \
[ -e my-thing.previous ] || { echo 'no my-thing.previous to roll back to' >&2; exit 1; }; \
mv my-thing.current my-thing.swap; mv my-thing.previous my-thing.current; mv my-thing.swap my-thing.previous; \
systemctl restart my-thing"
    echo "rolled back (run again to undo)"
    exit
fi

cargo device-build

bin=target/aarch64-unknown-linux-gnu/release/my-thing
sdk/sysroots/x86_64-pokysdk-linux/usr/bin/aarch64-poky-linux/aarch64-poky-linux-strip \
    -o "$bin.stripped" "$bin"

# Copy under a temporary name, then rename: a rename is atomic, so the service never
# sees a half-written binary.
scp -q "$bin.stripped" "$dev:$opt/my-thing.incoming"
ssh "$dev" "set -e; cd $opt; \
[ -e my-thing.current ] && mv my-thing.current my-thing.previous; \
mv my-thing.incoming my-thing.current; \
systemctl restart my-thing; \
sleep 2; systemctl is-active my-thing"
echo "installed $(ls -l "$bin.stripped" | awk '{print $5}') bytes; rollback: $0 --rollback"
