#!/bin/sh
# Share this computer's internet with the Car Thing over USB (run on the host, once).
#
# The device's USB network (CDC-NCM) is a /29 like 10.42.1.72/29: the device is
# at .74 and runs a DHCP server for the host, but nothing routes it further. On
# boot the device (my-thing-uplink, in the Yocto image) points its default route
# at the first address of that /29 (.73 here). This script makes the host be
# that address and NAT the device's traffic out to the internet:
#
#   1. NetworkManager profile "my-thing-usb": the device's USB link, matched by
#      its MAC (stable: derived from the device serial, so any USB port works),
#      static .73/29, never used as this computer's default route.
#   2. systemd service "my-thing-share-internet": at boot, turns on IPv4
#      forwarding and adds iptables rules (scripts/host-nat.sh) that let the
#      device's traffic through and masquerade it. Stopping it removes them.
#
# Also removes the older ufw-based setup (before.rules block, route rule) if an
# earlier version of this script added it. Safe to run again. Needs sudo.
#
# Usage: scripts/host-share-internet.sh          set up (device plugged in)
#        scripts/host-share-internet.sh --undo   remove it again (device not needed)
# Env:   MY_THING_MAC=02:11:44:..  MY_THING_NET=10.42.1.72/29
#        Use these when the device isn't plugged in; otherwise they're detected.
set -eu

PROFILE=my-thing-usb
SERVICE=my-thing-share-internet
UNIT=/etc/systemd/system/$SERVICE.service
HELPER=/usr/local/lib/my-thing/host-nat.sh
ENVFILE=/etc/default/$SERVICE
here=$(cd "$(dirname "$0")" && pwd)

# Left by earlier versions of this script, which used ufw.
UFW_RULES=/etc/ufw/before.rules
UFW_BEGIN="# BEGIN my-thing: NAT for the Car Thing (scripts/host-share-internet.sh)"
UFW_END="# END my-thing"

say() { printf '\n==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

# The device's USB link on the host has a MAC starting 02:11:44 (the BSP's host_addr).
detect() {
    for dev in /sys/class/net/*; do
        mac=$(cat "$dev/address" 2>/dev/null || true)
        case "$mac" in
            02:11:44:*)
                iface=$(basename "$dev")
                cidr=$(ip -4 -o addr show dev "$iface" | awk '{print $4}' | head -n 1)
                echo "$mac $iface $cidr"
                return 0
                ;;
        esac
    done
    return 1
}

# 10.42.1.78/29 -> 10.42.1.72/29 (network of a /29).
net_of() {
    ip=${1%/*}
    last=${ip##*.}
    echo "${ip%.*}.$(( last / 8 * 8 ))/29"
}

# Older ufw-based setup: drop its NAT block and route rule (no-op if absent).
# The network comes from the block itself, so this works with the device unplugged.
remove_old_ufw() {
    if [ -f "$UFW_RULES" ] && sudo grep -qF "$UFW_BEGIN" "$UFW_RULES"; then
        old_net=$(sudo sed -n "\|^$UFW_BEGIN\$|,\|^$UFW_END\$|s|.*-s \([0-9./]*\) .*|\1|p" "$UFW_RULES" | head -n 1)
        echo "removing the old NAT block from $UFW_RULES (backup: $UFW_RULES.my-thing.bak)"
        sudo cp "$UFW_RULES" "$UFW_RULES.my-thing.bak"
        sudo sed -i "\|^$UFW_BEGIN\$|,\|^$UFW_END\$|d" "$UFW_RULES"
        if command -v ufw >/dev/null; then
            [ -z "$old_net" ] || sudo ufw route delete allow from "$old_net" >/dev/null 2>&1 || true
            sudo ufw reload >/dev/null
        fi
    else
        echo "(none)"
    fi
}

command -v nmcli >/dev/null || die "nmcli not found (this script expects NetworkManager)"

# Undo needs nothing from the device, so it works with it unplugged.
if [ "${1-}" = "--undo" ]; then
    say "Removing NetworkManager profile $PROFILE"
    nmcli connection delete "$PROFILE" 2>/dev/null || echo "(not there)"

    say "Removing service $SERVICE"
    if [ -f "$UNIT" ]; then
        sudo systemctl disable --now "$SERVICE" || true
        sudo rm -f "$UNIT" "$ENVFILE" "$HELPER"
        sudo rmdir "$(dirname "$HELPER")" 2>/dev/null || true
        sudo systemctl daemon-reload
    else
        echo "(not there)"
    fi

    say "Removing the old ufw setup"
    remove_old_ufw

    echo
    echo "Done. IPv4 forwarding stays on until the next reboot (other tools may rely on it)."
    exit 0
fi

command -v iptables >/dev/null || die "iptables not found"
command -v systemctl >/dev/null || die "systemctl not found (this script expects systemd)"

mac=${MY_THING_MAC-}
net=${MY_THING_NET-}
iface=""
if [ -z "$mac" ] || [ -z "$net" ]; then
    if found=$(detect); then
        d_mac=$(echo "$found" | cut -d' ' -f1)
        iface=$(echo "$found" | cut -d' ' -f2)
        d_cidr=$(echo "$found" | cut -d' ' -f3)
        mac=${mac:-$d_mac}
        if [ -z "$net" ]; then
            [ -n "$d_cidr" ] || die "the device link $iface has no IPv4 address yet; wait a few seconds, or set MY_THING_NET"
            net=$(net_of "$d_cidr")
        fi
    else
        die "no Car Thing USB link found (MAC 02:11:44:...). Plug it in, or set MY_THING_MAC and MY_THING_NET"
    fi
fi
base=${net%/*}
host_ip="${base%.*}.$(( ${base##*.} + 1 ))"
device_ip="${base%.*}.$(( ${base##*.} + 2 ))"
echo "Car Thing: MAC $mac${iface:+ on $iface}, network $net (host $host_ip, device $device_ip)"

say "1/3 NetworkManager profile $PROFILE: $host_ip/29 on MAC $mac"
nmcli -t -f NAME connection show | grep -qx "$PROFILE" ||
    nmcli connection add type ethernet con-name "$PROFILE" ifname "*" >/dev/null
nmcli connection modify "$PROFILE" \
    ethernet.mac-address "$mac" \
    ipv4.method manual ipv4.addresses "$host_ip/29" ipv4.never-default yes \
    ipv6.method link-local \
    connection.autoconnect yes connection.autoconnect-priority 100
echo "profile set"
if [ -n "$iface" ]; then
    # Switch the link from the DHCP profile NetworkManager made for it.
    nmcli connection up "$PROFILE" ifname "$iface" >/dev/null && echo "active on $iface"
fi

say "2/3 Service $SERVICE: iptables NAT for $net"
sudo install -D -m 0755 "$here/host-nat.sh" "$HELPER"
echo "MY_THING_NET=$net" | sudo tee "$ENVFILE" >/dev/null
sudo tee "$UNIT" >/dev/null <<UNIT
[Unit]
Description=Share internet with the Car Thing over USB (my-thing)
# After any firewall, so its rules load first and ours go in front of them.
After=network.target ufw.service iptables.service nftables.service firewalld.service

[Service]
Type=oneshot
RemainAfterExit=yes
EnvironmentFile=$ENVFILE
ExecStart=$HELPER start
ExecStop=$HELPER stop

[Install]
WantedBy=multi-user.target
UNIT
sudo systemctl daemon-reload
sudo systemctl enable "$SERVICE" >/dev/null
# Restart, not start: picks up a changed network or helper on a re-run.
sudo systemctl restart "$SERVICE"
echo "enabled and running ($UNIT)"

say "3/3 Removing the old ufw setup, if any"
remove_old_ufw

say "Check"
if ping -c 1 -W 2 "$device_ip" >/dev/null 2>&1; then
    echo "device $device_ip answers"
else
    echo "device $device_ip doesn't answer yet (unplugged or still booting?)"
fi
cat <<EOF

Done. Check from the device (the image's my-thing-uplink sets its route at boot):
  ssh root@my-thing.local 'ip route; resolvectl query oauth2.googleapis.com; ping -c 1 1.1.1.1'
Status: systemctl status $SERVICE
Undo:   $0 --undo
EOF
