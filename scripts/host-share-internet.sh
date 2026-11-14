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
#   2. ufw (the active firewall): IPv4 forwarding on, masquerade for the /29,
#      and a route rule allowing the device's traffic through.
#
# Safe to run again. Needs sudo for the ufw part.
#
# Usage: scripts/host-share-internet.sh          set up (device plugged in)
#        scripts/host-share-internet.sh --undo   remove it again
# Env:   MY_THING_MAC=02:11:44:..  MY_THING_NET=10.42.1.72/29
#        Use these when the device isn't plugged in; otherwise they're detected.
set -eu

PROFILE=my-thing-usb
RULES=/etc/ufw/before.rules
SYSCTL=/etc/ufw/sysctl.conf
BEGIN="# BEGIN my-thing: NAT for the Car Thing (scripts/host-share-internet.sh)"
END="# END my-thing"

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

command -v nmcli >/dev/null || die "nmcli not found (this script expects NetworkManager)"
command -v ufw >/dev/null || die "ufw not found (this script expects ufw as the firewall)"

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

if [ "${1-}" = "--undo" ]; then
    say "Removing NetworkManager profile $PROFILE"
    nmcli connection delete "$PROFILE" 2>/dev/null || echo "(not there)"

    say "Removing the NAT block from $RULES"
    if sudo grep -qF "$BEGIN" "$RULES"; then
        sudo cp "$RULES" "$RULES.my-thing.bak"
        sudo sed -i "\|^$BEGIN\$|,\|^$END\$|d" "$RULES"
    else
        echo "(not there)"
    fi

    say "Removing the ufw route rule"
    sudo ufw route delete allow from "$net" || true
    sudo ufw reload

    echo
    echo "Done. IPv4 forwarding in $SYSCTL is left on (other tools may rely on it);"
    echo "comment out net/ipv4/ip_forward=1 there and run 'sudo ufw reload' to turn it off."
    exit 0
fi

say "1/4 NetworkManager profile $PROFILE: $host_ip/29 on MAC $mac"
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

say "2/4 IPv4 forwarding in $SYSCTL"
if sudo grep -q '^net/ipv4/ip_forward=1' "$SYSCTL"; then
    echo "(already on)"
elif sudo grep -q '^#[[:space:]]*net/ipv4/ip_forward=1' "$SYSCTL"; then
    sudo sed -i 's|^#[[:space:]]*net/ipv4/ip_forward=1|net/ipv4/ip_forward=1|' "$SYSCTL"
    echo "uncommented"
else
    echo 'net/ipv4/ip_forward=1' | sudo tee -a "$SYSCTL" >/dev/null
    echo "added"
fi

say "3/4 Masquerade for $net in $RULES"
sudo grep -q '^\*filter' "$RULES" || die "no *filter section in $RULES; is this a ufw before.rules?"
sudo cp "$RULES" "$RULES.my-thing.bak"
# One pass: drop our block from an earlier run (the network may have changed),
# and put the new one before ufw's *filter table (*nat has to come first).
tmp=$(mktemp)
sudo awk -v begin="$BEGIN" -v end="$END" -v net="$net" '
    $0 == begin { skip = 1 }
    skip { if ($0 == end) skip = 0; next }
    !done && /^\*filter/ {
        print begin
        print "*nat"
        print ":POSTROUTING ACCEPT [0:0]"
        print "-A POSTROUTING -s " net " ! -d " net " -j MASQUERADE"
        print "COMMIT"
        print end
        done = 1
    }
    { print }
' "$RULES" > "$tmp"
sudo install -m 0640 -o root -g root "$tmp" "$RULES"
rm -f "$tmp"
echo "written (backup: $RULES.my-thing.bak)"

say "4/4 ufw: allow the device's traffic through, then reload"
sudo ufw route allow from "$net" comment 'my-thing Car Thing'
sudo ufw reload

say "Check"
if ping -c 1 -W 2 "$device_ip" >/dev/null 2>&1; then
    echo "device $device_ip answers"
else
    echo "device $device_ip doesn't answer yet (unplugged or still booting?)"
fi
cat <<EOF

Done. Check from the device (the image's my-thing-uplink sets its route at boot):
  ssh root@my-thing.local 'ip route; resolvectl query oauth2.googleapis.com; ping -c 1 1.1.1.1'
Undo: $0 --undo
EOF
