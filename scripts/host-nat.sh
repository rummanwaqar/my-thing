#!/bin/sh
# NAT for the Car Thing's USB network, with plain iptables. Run by the
# my-thing-share-internet systemd service (installed by scripts/host-share-internet.sh),
# not by hand.
#
# Rules live in their own chains (MY-THING-FWD, MY-THING-NAT) with one jump each
# from FORWARD and POSTROUTING, so start is safe to repeat and stop removes
# exactly what start added. They match the device's /29 by address, not by
# interface, so they work before the device is plugged in.
#
# Usage: host-nat.sh start|stop
# Env:   MY_THING_NET=10.42.1.72/29 (from /etc/default/my-thing-share-internet)
set -eu

FWD=MY-THING-FWD
NAT=MY-THING-NAT

stop() {
    while iptables -D FORWARD -j "$FWD" 2>/dev/null; do :; done
    while iptables -t nat -D POSTROUTING -j "$NAT" 2>/dev/null; do :; done
    iptables -F "$FWD" 2>/dev/null || true
    iptables -X "$FWD" 2>/dev/null || true
    iptables -t nat -F "$NAT" 2>/dev/null || true
    iptables -t nat -X "$NAT" 2>/dev/null || true
}

start() {
    net=${MY_THING_NET:?MY_THING_NET is not set}
    sysctl -q -w net.ipv4.ip_forward=1

    iptables -N "$FWD" 2>/dev/null || iptables -F "$FWD"
    iptables -A "$FWD" -s "$net" -j ACCEPT
    iptables -A "$FWD" -d "$net" -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
    # First in FORWARD, ahead of any firewall's (e.g. ufw's) default drop.
    iptables -C FORWARD -j "$FWD" 2>/dev/null || iptables -I FORWARD 1 -j "$FWD"

    iptables -t nat -N "$NAT" 2>/dev/null || iptables -t nat -F "$NAT"
    iptables -t nat -A "$NAT" -s "$net" ! -d "$net" -j MASQUERADE
    iptables -t nat -C POSTROUTING -j "$NAT" 2>/dev/null || iptables -t nat -A POSTROUTING -j "$NAT"
}

case "${1-}" in
    start) start ;;
    stop) stop ;;
    *) echo "usage: $0 start|stop" >&2; exit 2 ;;
esac
