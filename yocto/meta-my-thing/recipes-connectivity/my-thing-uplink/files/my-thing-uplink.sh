#!/bin/sh
# my-thing-uplink: internet through the USB host the Car Thing is plugged into.
#
# The BSP's superbird-usb-gadget writes /run/systemd/network/11-usb-ncm.network
# with this device's address (10.42.1.<n+2>/29, derived from the serial number)
# and runs a DHCP server for the host, but gives the device no route or DNS.
# The host is expected at the first address of that /29 (the BSP's NCM_HOST_IP,
# one below ours), set up with NAT by scripts/host-share-internet.sh. This adds
# a networkd drop-in: default route via the host, public DNS.
#
# Without the host side, the route is just unreachable: no change from before.
set -eu

NET=/run/systemd/network/11-usb-ncm.network
DROPIN_DIR=/run/systemd/network/11-usb-ncm.network.d

addr=$(sed -n 's|^Address=\([0-9.]*\)/.*|\1|p' "$NET" | head -n 1)
if [ -z "$addr" ]; then
    echo "my-thing-uplink: no Address= in $NET" >&2
    exit 1
fi
host="${addr%.*}.$(( ${addr##*.} - 1 ))"

mkdir -p "$DROPIN_DIR"
cat > "$DROPIN_DIR/uplink.conf" <<EOF
# Written by my-thing-uplink at boot. The host routes us to the internet.
[Network]
Gateway=$host
DNS=1.1.1.1
DNS=9.9.9.9
# Send all DNS lookups over this link.
Domains=~.
EOF
echo "my-thing-uplink: device $addr, default route via host $host"

# Normally this runs before networkd starts; if it's already up, apply now.
if systemctl is-active --quiet systemd-networkd; then
    networkctl reload || true
fi
