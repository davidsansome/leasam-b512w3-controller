# Kindle wall controller

> **⚠️ Use at your own risk.** This is a touchscreen front-end for a system (see [`replacement-board/`](../replacement-board/)) that replaces part of the control electronics for a mains-powered air conditioner / heat pump. See the [top-level README](../README.md#safety) for the full disclaimer.

A full-screen touch UI for a jailbroken **Kindle Paperwhite 10th gen (FW 5.16.4)**. It talks to Home Assistant over the WebSocket API and shows up in HA as an **Aircon Wall Controller** device.

The screen follows mockup **B2** (two-column aircon panel) on the design canvas.

## What it does

- **Dashboard** (1448×1072 landscape):
  - **Weather** from a `weather.*` entity: current conditions, a rain summary and today's high/low.
  - **Next 24 hours:** temperature line and rain bars from `weather.get_forecasts`, refreshed every 30 min.
  - **Rooms:** up to four temperature chips, each with a sparkline of the last hour. The rooms come from the aircon controller's **Rooms** entity, which is set up in HA with the *Aircon rooms* blueprint (see [the firmware README](../replacement-board/README.md#thermostat)), not in `config.toml`. The history comes from HA's recorder when the room list arrives, then from live updates. With no rooms set up, the row says so. The last room list is kept while the aircon is unavailable.
    - In Thermostat mode, tapping a room makes the thermostat go by that room (the **Thermostat room** select); it gets a thick black border. Tapping it again goes back to the average of all rooms. Rooms can't be picked in Continuous mode.
  - **Aircon panel:**
    - On/Off button.
    - Compressor lockout chip (`MM:SS`, counted down locally and redrawn every 10 s; only the chip itself gets the occasional cleaning flash).
    - **Idle** chip in place of the lockout chip while the thermostat has switched the unit off: "Idle · at set point", "Idle · near set point" (Auto), "Idle · no temperature", "Idle · no rooms configured" or "Idle · Auto needs the thermostat", from **Thermostat status**.
    - Continuous/Thermostat toggle and a set point with −/+ (shown to one decimal; hidden in Continuous mode), with "Now 21.4° · average" (or the selected room) under it, from **Thermostat temperature**. Each control only appears if HA has its entity.
    - Heat/Cool/Fan/Auto tiles. A mode the select entity doesn't offer is greyed out. In Auto, the tile shows which way it's working ("Auto · Heat" or "Auto · Cool", from **Auto direction**). Auto needs the thermostat, so the Auto tile is greyed out in Continuous mode, and Continuous is greyed out in Auto.
    - Low/Medium/High fan speed.
  - **Aircon unavailable:** if the power entity is `unavailable`, the panel shows "Aircon unavailable" with when it was last seen, instead of the controls.
  - **Right-hand column:** menu button, local time (HH:MM) and the battery (filled to the charge level, with a bolt while charging).
- **Taps** show their result immediately, using a fast black/white (DU) refresh, and send the service call. The area is redrawn properly (GL16) about 1.5 s later, and the optimistic state is dropped once HA reports the real one, or after 8 s. A rejected call shows its error along the bottom. Ghosting is tracked in 32 px tiles: an area is flashed (GC16, just that area) once it has had 20 partial refreshes (`display.full_refresh_every`), and the whole screen flashes only when over half of it is due. Anything drawn against time (sparklines, the forecast) uses the minute on the clock, so redraws within a minute that change nothing on screen are skipped.
- **Idle power:** nothing polls.
  - The HA client thread sleeps in `poll()` until HA sends something, the UI makes a service call, or a ping (every 30 s) or diagnostics update is due.
  - The UI loop sleeps until the next minute (for the clock), and wakes more often only while something is counting down:
    - the compressor lockout, on 10 s boundaries;
    - the reconnect countdown, on 5 s boundaries;
    - a pending tap, notice or menu timeout.
  - The battery is read on the minute wake-up.
  - Signals wake the loop through a pipe, so stopping is immediate.
- **Disconnected:** shows a full-screen "Can't reach Home Assistant" screen with the reason and a retry countdown (in 5 s steps), and retries with backoff (1 s up to 60 s). HA is pinged every 30 s, so a dead Wi-Fi link is noticed within about 50 s.
- **Device in HA:** registers through the built-in `mobile_app` integration and appears under *Settings → Devices → Mobile App*. It has three diagnostic sensors, pushed every 5 minutes:
  - battery level (%)
  - uptime (s)
  - Wi-Fi signal (dBm)
- **Orientation:** landscape by default (`display.rotation = 90`; use 270 if the Kindle is mounted the other way up).
- **Menu:** the **⋮** button in the top-right corner, on every screen including the error screen, opens a dropdown with:
  - **Turn backlight on/off**, through powerd's `flIntensity`. It restores the previous level when turned back on.
  - **Sleep**, which shows a full-screen sleepy house (closed eyes, a smile and some Zs), closes the HA connection, and turns Wi-Fi (`com.lab126.cmd wirelessEnable`) and the touchscreen (`com.lab126.deviced enable_touch`) off, along with the backlight. Touches are ignored while asleep even if the touchscreen can't be turned off. Pressing the **power button** turns both back on and shows a **Waking up** screen (the house with its eyes open, stretching, with Wi-Fi arcs over the roof) until HA is connected, saying whether it's still waiting for Wi-Fi or connecting to HA. Wi-Fi takes about 11 s to come back. Meanwhile HA is retried every second (for up to a minute) rather than backing off, and as soon as the Kindle announces the network is up (`com.lab126.cmd connectionAvailable`, heard through `lipc-wait-event`), it reconnects immediately. If HA still isn't back after a minute, the usual "Can't reach Home Assistant" screen takes over. The same network event also cuts short the backoff after an ordinary Wi-Fi dropout. The backlight comes back on when waking if it was on before. The Kindle itself stays awake (light sleep): the stock firmware doesn't deep-sleep on USB power either.
  - **Exit controller**, which hands the screen back to the Kindle UI.
  - **Reboot**, which needs a second tap to confirm. The controller doesn't start automatically after a reboot yet.

  Tapping anywhere outside the menu closes it without triggering anything underneath.
- **Kindle UI:** stopped while the controller runs (`framework.mode = "stop"`, the default). This stops the `lab126_gui` service: reader app, book renderer, window manager and memory monitor. X, `blanket` and `kindle_browser` aren't part of that service, so they're paused with SIGSTOP instead.
  - **Idle CPU:** this roughly halves the Kindle's idle CPU, from about 1.0% to 0.4%, and frees the UI's memory.
  - **Exiting:** takes about a minute while the Kindle UI starts back up. The controller shows "Returning to the Kindle home screen" meanwhile.
  - **Coexist mode:** `"coexist"` leaves the UI running but paused, which exits faster but uses more CPU.
  - **Touch:** touches are grabbed so the Kindle UI never sees them.
  - **Never sleeps:** going to sleep would turn the touchscreen off, so the controller prevents it. powerd's `preventScreenSaver` stops the inactivity timeout, the power button is grabbed (`framework.block_power_button`; the controller still sees presses, to wake from **Sleep**), and if the Kindle is already asleep when the controller starts, it's woken up.
  - **Restoring:** everything is put back on exit. While the controller runs, KUAL is unavailable, so the menu (or SSH) is the way out.

## Layout

| Path | |
|---|---|
| `src/main.rs` | startup and event loop |
| `src/display/` | framebuffer (mmap), e-ink refresh via the `fbink` CLI, canvas with anti-aliased vector shapes (tiny-skia), fonts, icons |
| `src/input.rs` | touchscreen (evdev) thread |
| `src/framework.rs` | pausing/stopping the Kindle UI and restoring it |
| `src/sleep.rs` | Sleep: Wi-Fi and touch on/off, and listening for the network coming back |
| `src/ha/` | WebSocket client, entity state tracking, mobile_app registration |
| `src/diagnostics.rs` | battery / uptime / Wi-Fi readings |
| `src/ui/` | screens and touch handling (`mod.rs`), the dashboard (`dashboard.rs`), drawing helpers (`draw.rs`) |
| `src/clock.rs` | local time and ISO timestamps |
| `kual/aircon-controller/` | KUAL extension: launcher, supervisor and stop scripts, plus the Library launcher (`aircon-controller.sh`) and its cover (`icon.png`) |

### Configuring entities

Run this on the Kindle to list the entity ids worth putting in `config.toml`: weather, temperature sensors, and switches/selects/numbers.

```bash
/mnt/us/extensions/aircon-controller/bin/aircon-wall-controller --list-entities
```

The `[dashboard]` and `[aircon]` sections of `config.example.toml` show every option. The `[aircon]` defaults match the ids HA gives the controller's entities, and with `aircon.device` set they're found on the device even if HA renames them, so usually nothing there needs changing. Controls whose entities HA doesn't have are left out. If the mode select's option names differ from Heat / Cool / Fan only / Auto, set them in `[aircon.mode_options]`.

`[[dashboard.rooms]]` from older versions is ignored (with a warning in the log): rooms are now set up in HA.

### Previewing on a PC

`cargo test preview -- --ignored` renders sample screens to raw 8-bit files. Set `PREVIEW_DIR` to the output folder, and `PREVIEW_FONT` / `PREVIEW_BOLD` to a regular and a bold TTF. That's handy for checking layout changes without the Kindle.

`cargo test scriptlet_icon -- --ignored`, with the same variables, renders the Library cover to `icon.pgm` using the waking-house icon. Convert it with `magick icon.pgm icon.png` and copy it to `kual/aircon-controller/`. Use the Kindle's Amazon Ember fonts (`/usr/java/lib/fonts/Amazon-Ember-{Regular,Bold}.ttf`) to match its UI.

### Kindle background activity

Even with the controller idle, the Kindle's own cron jobs run every 15 minutes (`tinyrot` log rotation and `loginfo`), with more hourly. Each run is a burst of several seconds of CPU, mostly compressing logs. Leave them alone: the rotation stops `/var/log` (a RAM disk) from filling up.

## Building

Built on Windows with just `rustup`: no ARM toolchain, Docker or WSL. The target is static musl, linked with `rust-lld`, so it doesn't depend on the Kindle's libc.

```bash
rustup target add armv7-unknown-linux-musleabihf
```

```bash
cargo kindle
```

The binary lands in `target/armv7-unknown-linux-musleabihf/release/aircon-wall-controller`. Unit tests run on the host:

```bash
cargo test
```

## Installing

1. **FBInk.** Download a release for FW ≥ 5.16.3 (the `kindlehf` build) from NiLuJe's FBInk releases. Put its `fbink` binary in `kual/aircon-controller/bin/`, or set `display.fbink` in the config. The controller also looks in `/mnt/us/libkh/bin`, `/mnt/us/usbnet/bin` and `/mnt/us/koreader`.
2. **Files.** Copy `target/armv7-unknown-linux-musleabihf/release/aircon-wall-controller` into `kual/aircon-controller/bin/`.
3. **Config.** Copy `config.example.toml` to `kual/aircon-controller/config.toml`, then set:
   - the HA URL, using its **IP address** (the static binary can't resolve `.local` names);
   - a long-lived access token.
4. **Copy to the Kindle.** Copy the `kual/aircon-controller` folder to `/mnt/us/extensions/` on the Kindle, over USB or with `scp` via USBNetwork.
5. **Library button (optional).** Copy `kual/aircon-controller/aircon-controller.sh` to `/mnt/us/documents/`. On firmware with the scriptlet hotfix (5.16.x jailbreaks), it appears in the Library as *Aircon Wall Controller*, with `icon.png` as its cover. Tapping it starts the controller.
6. **Start it.** Tap *Aircon Wall Controller* in the Library, or in KUAL choose *Aircon Wall Controller → Start*.

Logs go to `/mnt/us/extensions/aircon-controller/controller.log`. Set `AIRCON_LOG=debug` in `supervise.sh` for touch coordinates, HA traffic and refresh calls. `state.json` holds the device id and webhook id. Delete it to register as a new device.

## Tested on the device

On the PW4 with FW 5.16.4 and HA 2026.10.4, these have been checked:
- the HA connection and the mobile_app device registration;
- touch mapping (no `touch.*` fixes needed);
- landscape rendering;
- the menu (open, close, backlight, arming reboot);
- restoring the Kindle UI on exit.

Exit and Reboot from the menu haven't been tried yet.

Settings worth knowing about:

- **Orientation.** `display.rotation = 270` flips landscape the other way up. Touch follows automatically.
- **Other KUAL builds / Kindles.** The `touch.*`, `display.invert`, waveform, `display.font` and `framework.pause_processes` settings exist for hardware that differs from the PW4.
- **KUAL launch.** If the menu entries do nothing, check that this KUAL version supports `"params"` in `menu.json`. KUAL isn't installed on the test Kindle yet; it's started from the Library scriptlet, or over SSH with `sh bin/start.sh`.

## Not done yet

- Starting automatically at boot.
- HTTPS / `wss://` (plain `http://` on the LAN only).
- Dimming the screen at night.
- Linking FBInk directly instead of spawning the CLI for each refresh.
