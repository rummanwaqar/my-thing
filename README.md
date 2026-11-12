# my-thing

Slint app for the Spotify Car Thing.

## Pomodoro

Presets: 25/5, 50/10, 15/3 (focus/break minutes).

| State | Knob press | Knob turn | Back (Esc) | Timer reaches 0 |
|---|---|---|---|---|
| **Ready** | start Focus at the preset's focus length | cycle presets (wraps) | nothing | n/a |
| **Focus** | pause / resume | ±1 min per detent, clamped to 1–99 min (also while paused) | → Ready | → FocusDone |
| **FocusDone** (alarm) | start Break at the preset's break length | Focus for 5 more minutes | → Ready | n/a |
| **Break** | skip → Ready | ±1 min, same clamp | → Ready | → BreakDone |
| **BreakDone** (alarm) | start Focus | Break for 5 more minutes | → Ready | n/a |

On the desktop: ←/→ = knob turn, Enter = knob press, Esc = back. To reach an alarm quickly, turn down to 1:00 in Focus or Break.

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

Build, copy to the device, and run as a fullscreen client of the image's Weston (stops Chromium first; Ctrl-C to quit):

```sh
cargo device-run
```

- `cargo device-build` only builds: `target/aarch64-unknown-linux-gnu/release/my-thing`
- `DEVICE=10.42.1.74 cargo device-run` if mDNS doesn't resolve
- `SLINT_DEBUG_PERFORMANCE=refresh_lazy,overlay cargo device-run` shows FPS
- Weston rotates the portrait panel, so Slint renders unrotated on the GPU (femtovg) with crisp text. Rendering straight to the display with `SLINT_KMS_ROTATION` made femtovg draw text as outlines (bold), and the software renderer was too slow (~12 fps on some screens).
- Restore the kiosk: `ssh root@superbird-kiosk.local reboot`
