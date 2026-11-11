# my-thing

Slint app for the Spotify Car Thing.

## Desktop

```sh
cargo run
```

## Device

One-time setup: install the Yocto SDK (see `../plans/0-hello-world.md`, Phase A) and link it here:

```sh
rustup target add aarch64-unknown-linux-gnu
ln -s ~/.local/superbird-sdk sdk
```

Build, copy to the device, and run (stops the kiosk first; Ctrl-C to quit):

```sh
cargo device-run
```

- `cargo device-build` only builds: `target/aarch64-unknown-linux-gnu/release/my-thing`
- `DEVICE=10.42.1.74 cargo device-run` if mDNS doesn't resolve
- `SLINT_DEBUG_PERFORMANCE=refresh_lazy,overlay cargo device-run` shows FPS
- `SLINT_BACKEND=linuxkms-software cargo device-run` uses the software renderer
- Restore the kiosk: `ssh root@superbird-kiosk.local reboot`
