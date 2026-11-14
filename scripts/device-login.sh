#!/bin/sh
# Copy the desktop's Google login onto the Car Thing, so the app there is logged in.
#
# The device has no browser, so log in on the desktop first (cargo run, open the
# link, "Logged in as ..."). Then this copies the OAuth client config and the
# saved login to the device's persistent data partition (/var), where the
# my-thing service looks for them ($HOME=/var/lib/my-thing), and restarts it.
# They survive reboots and scripts/device-install.sh; a factory reset wipes them.
#
# Usage: scripts/device-login.sh
# Env:   DEVICE (default my-thing.local)
set -eu

dev="root@${DEVICE:-my-thing.local}"
config_home=${XDG_CONFIG_HOME:-$HOME/.config}
state_home=${XDG_STATE_HOME:-$HOME/.local/state}
client="$config_home/my-thing/google-client.json"
login="$state_home/my-thing/google-login.json"

remote_home=/var/lib/my-thing
remote_client_dir=$remote_home/.config/my-thing
remote_login_dir=$remote_home/.local/state/my-thing

[ -f "$client" ] || { echo "no $client: set up the Google OAuth client first (README, Google login)" >&2; exit 1; }
[ -f "$login" ] || { echo "no $login: run 'cargo run' on this computer and log in first" >&2; exit 1; }

echo "copying the Google login to $dev"
ssh "$dev" "set -e; umask 077; mkdir -p $remote_client_dir $remote_login_dir"
# Copy under temporary names, then rename: the app never sees a half-written file.
scp -q "$client" "$dev:$remote_client_dir/google-client.json.incoming"
scp -q "$login" "$dev:$remote_login_dir/google-login.json.incoming"
# sync before restarting: the Car Thing loses power when unplugged, and Linux keeps
# new file data in memory for up to 30 s (unplugging right after left empty files).
ssh "$dev" "set -e; \
chmod 600 $remote_client_dir/google-client.json.incoming $remote_login_dir/google-login.json.incoming; \
mv $remote_client_dir/google-client.json.incoming $remote_client_dir/google-client.json; \
mv $remote_login_dir/google-login.json.incoming $remote_login_dir/google-login.json; \
sync; \
systemctl restart my-thing"

echo "restarted my-thing; waiting for it to log in..."
ssh "$dev" "for i in \$(seq 1 20); do journalctl -u my-thing -b --no-pager -n 50 | grep -q 'google: Logged in as' && break; sleep 1; done; \
journalctl -u my-thing -b --no-pager -n 50 | grep -E 'google:|calendar:' | tail -n 8"
