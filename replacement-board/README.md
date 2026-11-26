# ESP32 aircon controller + tablet charger

> **⚠️ Use at your own risk.** These boards replace part of the control electronics for a mains-powered ducted air conditioner / heat pump, and connect directly to the relay module's wiring. Getting it wrong can damage the unit, create a shock or fire hazard, or void your warranty and insurance. See the [top-level README](../README.md#safety) for the full disclaimer.

Two boards replace the Leasam B512W3 wall pad:

1. **ESP32 board**, mounted near the relay module. It connects to the module's **POWER, COMMON, AUX** wires, sends the relay frames the original pad sent, and takes commands over Wi-Fi. It also feeds a switched, current-limited ~16 V output up the old in-wall cable.
2. **Tablet board**, behind the tablet at the other end of that cable. It steps the voltage down to 5 V on a current-limited USB-A socket for slow-charging an Android tablet.

| Files | |
|---|---|
| `firmware/aircon_controller/aircon_controller.ino` | ESP32 firmware, for both this board and the breadboard prototype |
| `firmware/aircon_controller/thermostat.h`, `firmware/thermostat_test/` | the thermostat's decisions, and host-side checks for them |
| `home-assistant/blueprints/aircon_rooms.yaml` | HA blueprint that gives the thermostat its rooms and their temperatures |
| `kicad/esp32-board/` | KiCad project, designator map and netlist for board 1 |
| `kicad/tablet-board/` | KiCad project, designator map and netlist for board 2 |
| `../leasam-b512w3-notes.md` | reverse-engineering notes |
| `breadboard-prototype.md`, `kicad/breadboard-stage1/`, `kicad/breadboard-stage2/` | breadboard test circuit, built from common parts, for checking the module accepts our frames before fabricating |

See [`CHANGES.md`](CHANGES.md) for what changed after the 2026-11-04 design review, and why.

---

## 0. Power budget (measured)

| Measurement | Value |
|---|---|
| POWER open-circuit voltage | 19.0 V |
| Source resistance (load test: 1 kΩ / 470 Ω / 100 Ω → 18.50 / 18.11 / 15.80 V) | ≈ 20 Ω (≈ 25 Ω at light load) |
| Original pad's draw | 72 mA off, 90–100 mA running |

The design stays inside what the original pad drew:

| | Limit | Typical |
|---|---|---|
| Total draw from POWER (input limiter) | ≈ 105 mA | ~85–90 mA |
| ESP32 itself | — | ~10 mA average at ~16 V (Wi-Fi bursts come from the reservoir) |
| Tablet cable output (limiter 2) | ≈ 87 mA (80–93 mA) | ≤ ~64 mA worst case |
| Tablet USB output | ≈ 123 mA typ. at 5.07 V (R_ILIM 220k) | **≈ 0.5–0.75 W into the tablet** |

That's enough to charge a mostly-sleeping tablet slowly, and to support roughly **2 hours of screen-on time per day** on average.

The TPS2553 on the tablet board is the limit that matters in normal running. Limiter 2 sits well above the tablet board's worst-case draw, so it only acts on a short circuit. This matters because the tablet board's buck is a constant-power load: if limiter 2 ever clamped it, the input voltage would fall, the current would rise, and the board would cycle through UVLO.

---

## 1. ESP32 board

### 1.1 Block diagram

```
POWER ──┬─ R_TX 3.3Ω ── Q_TX ── GND          (TX: ~15 µs pulses)
        ├─ D_TVS SMBJ30A ── GND
        └─ D_IN ─► limiter 1 (≈105 mA) ─► VRES ─┬─► buck 3.3 V ─► ESP32-C3-WROOM-02
                                                ├─ EN divider (buck on ≈12.1 V / off ≈9.9 V)
                                                ├─ VSENSE ÷16 ─► GPIO3
                                                ├─► LP2950-5.0 ─► 5V_AUX
                                                └─► limiter 2 (≈87 mA, switched by GPIO5) ─► J_TAB ─► cable
                                                                         TAB_OUT ÷16 ─► GPIO1
AUX ──┬── R_AUX 10k ── 5V_AUX                    COMMON = GND
      └── 330k ──┬── GPIO0 (AUX_SENSE)
                100k ‖ 10 nF
                 GND
```

- **Transmitting** copies the original pad's Q2/R4: Q_TX (AO3422, R_DS(on) rated at V_GS = 2.5 V) pulls POWER to GND through 3.3 Ω. The gate is AC-coupled (C_G, R_GD), so a crashed ESP32 can't hold the line low.
- **D_IN** isolates everything behind it from the line during pulses. The module only ever sees a steady load.
- **Limiter 1** caps the total draw at about what the original pad drew.
- **Limiter 2** feeds the tablet cable. It caps the tablet's share so the ESP32 always has headroom, and protects the module if the in-wall cable is ever shorted.

### 1.2 Limiters

Both use the same two-transistor circuit. The limit is ≈ 0.65 V / R_S.

```
IN ──●── R_S ──●── node B                     Q_PASS  = BCP53-16 (PNP, SOT-223)
     │         │                              Q_SENSE = MMBT3906 (PNP, SOT-23)
  Q_SENSE E  Q_PASS E
  Q_SENSE B ───┘
  Q_SENSE C ──●── Q_PASS B ── base drive to GND
Q_PASS C ──► OUT
```

| | Limiter 1 (input) | Limiter 2 (tablet output) |
|---|---|---|
| IN → OUT | D_IN cathode → VRES | VRES → TAB_OUT |
| R_S | 6.2 Ω → ≈ 105 mA | 7.5 Ω → ≈ 87 mA |
| Base drive | R_B 10k to GND (always on) | R_B2 10k → Q_EN (MMBT3904) → GND. R_BE2 47k holds it off. |
| Switching | — | GPIO5 → R_ENB 10k → Q_EN base (R_ENBD 100k pull-down) |
| Worst-case heat | ≈ 2 W for ~0.3 s while the reservoir charges at power-up | ≈ 1.4 W into a shorted cable, until the firmware switches off (~2 s) |

Give both Q_PASS tabs a copper pour of at least 1 cm². The tab is the collector (VRES for limiter 1, TAB_OUT for limiter 2).

On the tablet output there's also D_TVS2 (SMBJ24A), C_TAB 100 nF, and a ÷16 sense divider (150k/10k, 100 nF) to GPIO1.

### 1.3 Reservoir, 3.3 V buck, ESP32

- **Reservoir:** 2× 1000 µF 63 V plus 2.2 µF 50 V X7R.
- **Buck enable divider:** 1M / 113k (on ≈ 12.1 V, off ≈ 9.9 V).
- **VSENSE:** 150k / 10k ÷16 to GPIO3.
- **Buck:** LMR51606YDBVR (1.1 MHz PFM) with 10 µH, 69.8k / 22.1k feedback (3.33 V), and 2× 22 µF + 100 µF out.

ESP32-C3-WROOM-02 pins:

| Pin | Signal | Connection |
|---|---|---|
| 1 | 3V3 | 3V3, 10 µF + 100 nF |
| 2 | EN | 10k to 3V3, 1 µF to GND, J_PROG |
| 3 | IO4 | TX gate drive (R_G 47 Ω → C_G 100 nF → Q_TX gate) |
| 4 | IO5 | tablet output enable |
| 7 / 16 | IO8 / IO2 | 10k pull-ups (strapping) |
| 8 | IO9 | BOOT → J_PROG |
| 10 | IO10 | LED via 1k |
| 11 / 12 | RXD0 / TXD0 | J_PROG |
| 5 / 6 / 13 / 14 | IO6 / IO7 / IO18 / IO19 | not connected |
| 15 | IO3 | VSENSE |
| 18 | IO0 | AUX_SENSE (AUX ÷4.3: 330k / 100k, 10 nF) |
| 17 | IO1 | tablet output sense |
| 9, 19 | GND | GND (19 is the centre pad) |

### 1.4 Connectors

| Ref | Pins |
|---|---|
| J_MOD (3-way 5.08 mm screw terminal) | 1 POWER, 2 COMMON, 3 AUX: short wires to the relay module |
| J_TAB (2-way 5.08 mm screw terminal) | 1 OUT+, 2 GND: to the in-wall cable |
| J_PROG (1×6 header) | 1 3V3, 2 GND, 3 EN, 4 IO9, 5 RXD0, 6 TXD0. The only wired way to flash the board (see below). |
| AUX | Always loaded with R_AUX 10k to 5V_AUX, the same as the original pad (R66 10k to its 5 V rail). Read on GPIO0. |

**Flashing.** After the first flash, update over Wi-Fi (OTA). For the first flash, or to recover from an OTA update that broke Wi-Fi, use a 3.3 V USB-UART adapter on J_PROG:

- Connect adapter GND → pin 2, TX → pin 5 (RXD0), RX → pin 6 (TXD0).
- Power the board from J_MOD or a bench supply, not from the adapter. Leave pin 1 (3V3) unconnected: most adapters can't supply the ESP32's Wi-Fi bursts, and it would back-feed the buck's output.
- To enter the bootloader: hold pin 4 (IO9) to GND, briefly short pin 3 (EN) to GND, then release IO9. The board has no auto-reset circuit, so an adapter's DTR/RTS lines won't do this.

---

## 2. Tablet board

```
J_IN+ ──┬─ D_TVS SMBJ24A ── GND
        └─ D_REV SS16 ─► VIN ─┬─ C_BULK 100µF 35V, C_IN 2.2µF 50V
                              ├─ UVLO divider 1M / 150k → EN (on ≈ 9.4 V, off ≈ 7.6 V)
                              └─ LMR51606 buck ─► 5 V (160k / 30k → 5.07 V, 10 µH, 2× 10 µF 50 V)
                                                   └─► TPS2553 current-limit switch ─► VBUS ─► USB-A
                                                       R_ILIM 220k → ≈ 123 mA typ.
```

- **D_TVS before D_REV:** if the cable is wired backwards, the TVS conducts and the ESP32 board's limiter 2 holds the current at ~87 mA until the firmware's fault detection switches it off. Nothing gets damaged.
- **TPS2553 (not the -1 version):** in overload it holds a constant current rather than latching off. A tablet asking for 1–2 A just sees VBUS sag slightly, and its charger backs off. Without this, the tablet would pull the input down, the buck would cut out and restart, and the tablet would see the charger repeatedly connecting and disconnecting.
- **The output limit is set well below what the cable can supply.** At worst case (≈ 0.74 W out, ≈ 85% buck efficiency, ~13.5 V at the board) the board draws ≈ 64 mA, against limiter 2's minimum of ≈ 79 mA after base drive. That margin is what stops the input collapsing.
- **Data lines:** left open by default, which tablets treat as a basic USB port. JP_DCP shorts D+ and D− (a "dedicated charger") if your tablet charges better that way. The limit is the same either way.
- **USB connection:** use a normal USB-A to USB-C cable to the tablet. The cable handles the USB-C side.

---

## 3. Firmware

One sketch, [`firmware/aircon_controller/`](firmware/aircon_controller/), serves both this board and the breadboard prototype. The board is picked from the chip it's compiled for: ESP32-C3 (`esp32:esp32:esp32c3`) builds for this board, and a classic ESP32 (`esp32:esp32:esp32`) builds for the breadboard. It needs the PubSubClient library. Copy `secrets.example.h` to `secrets.h` (git ignores it) and fill in the Wi-Fi, MQTT broker and OTA password.

It's controlled from Home Assistant over MQTT discovery, as an **Aircon** device: Power, Mode (Cool / Heat / Fan only / Auto), Fan speed, a Compressor running sensor, the [thermostat](#thermostat), and diagnostics. It also handles RMT frame timing, the compressor lockout, OTA (host `aircon`), Wi-Fi recovery and a loop watchdog. See the comment at the top of the sketch for the details.

Before starting Wi-Fi, it waits up to 20 s for VRES to reach 13 V, so the reservoir is charged before Wi-Fi's current bursts. "VRES at boot" and "Boot Wi-Fi wait" report how that went.

The **tablet output manager** is configurable at the top of the sketch.

| Rule | Default |
|---|---|
| Stays off for the first 30 s after boot | `TAB_BOOT_DELAY_MS` |
| Switches on when VRES ≥ 14 V for 10 s | `TAB_ON_MIN_V`, `TAB_ON_SETTLE_MS` |
| **Load shedding:** off if VRES < 13 V for 0.5 s | `TAB_SHED_V` |
| **Fault:** off if output < 10 V for 2 s while on (shorted cable, reversed wiring) | `TAB_FAULT_V` |
| Retry after a shed or fault | 60 s |

The Home Assistant entities for the tablet output:
- **Tablet output:** a switch that enables or disables the output. Turning it on also clears a pending retry wait. It isn't saved, so it is on again after every reboot.
- **Tablet output status:** text, such as "On", "Waiting: VRES settling", "Shed: VRES low" or "Fault: output low".
- **Tablet output voltage:** the 10 s mean.
- **Tablet output sheds** and **Tablet output faults:** counts since boot.

AUX, VRES and the tablet output are sampled at ~1 kHz. Every 10 s the firmware publishes the mean, plus the lowest and highest single sample (AUX minimum/maximum, VRES minimum), so short dips and spikes still show up. Home Assistant's history serves as the AUX log, for spotting rare events such as a defrost or a fault. The AUX ratio on this board hasn't been calibrated against a meter yet.

A rising shed count means the tablet's draw is pushing the budget. Lower R_ILIM's current (raise R_ILIM) or check VRES. A fault count means something is wrong on the cable side.

The firmware compiles for both chips (Arduino-ESP32 3.3.12). Apart from the 1 kHz sampling and the thermostat, the breadboard build is the sketch that was tested on the breadboard. The PCB-only parts (tablet output, VRES wait, pins and dividers) haven't run on a board yet.

### Thermostat

With **Thermostat** on and the mode Cool or Heat, the controller switches the whole unit off ("idle") when the room temperature reaches the **Set point**, and back on once it's **Thermostat hysteresis** beyond it (above the set point when cooling, below it when heating). Power, Mode and Fan speed keep their values throughout. **Operation** shows what the unit is really doing: `Off`, `Heating`, `Cooling` or `Fan` (fan only, or the fan running while the compressor lockout holds the compressor off). With Thermostat off (continuous) the unit runs whatever the temperature, and in Fan only mode the thermostat has no effect.

**Auto** heats or cools as needed, around one set point. It works like Heat with a set point of *Set point − Auto deadband*, or like Cool with *Set point + Auto deadband* (deadband 0.5–5 °C, default 1). With a set point of 22 °C, a 1 °C deadband and 0.5 °C hysteresis, it cools from 23.5 °C down to 23 °C, and heats from 20.5 °C up to 21 °C; in between it idles. On entering Auto (or switching the power on) it starts in the direction of the side of the set point the temperature is on. It changes direction only once the temperature is past the other direction's resume point (20.5 °C or 23.5 °C here), so it can't flip between heating and cooling near the set point. A change of direction stops the compressor, and the 3-minute lockout applies before it starts the other way. **Auto direction** shows `Heat` or `Cool` (unknown outside Auto, or before there's a temperature). Auto needs the thermostat: with Thermostat off (continuous), the unit stays idle.

- **Which temperature:** **Thermostat room** is `Average` (every room with a fresh reading) or one room's name. **Thermostat temperature** shows the value in use.
- **Short cycles:** once the compressor starts, it runs for at least **Minimum run time** (default 3 min) before the thermostat may stop it. The 3-minute compressor lockout still applies when it restarts, so the fan comes on first.
- **No temperature:** with no usable reading, the unit stays idle. A reading is unusable if the sensor is unavailable, if it's over 5 minutes old (`ROOM_STALE_MS`), or while HA is offline (`homeassistant/status`). This also covers HA going down: you couldn't switch the unit off then, so it doesn't run unattended.
- **Thermostat status** says which of these applies: `Off`, `Continuous`, `Fan only`, `Running`, `Running: minimum run time`, `Idle: at set point`, `Idle: near set point` (Auto, between heating and cooling), `Idle: no temperature`, `Idle: no rooms configured` or `Idle: Auto needs the thermostat`.
- Set point (16–30 °C), hysteresis (0.1–3 °C, default 0.5), Auto deadband, minimum run time (0–30 min), the Thermostat switch, the room selection and the room list are kept over a reboot.

**Rooms** are set up in Home Assistant with the *Aircon rooms* blueprint, [`home-assistant/blueprints/aircon_rooms.yaml`](home-assistant/blueprints/aircon_rooms.yaml). Its form has a picker for the Aircon device and up to four rooms, each a temperature sensor (filtered to temperature sensors) and an optional name; the name defaults to the sensor's area. The automation publishes each room's name and sensor (retained) and its temperature (converted from °F if needed) to `<node>/room/<1-4>/…`, whenever a sensor changes and every minute. The controller lists the rooms in its **Rooms** entity's attributes, which is where the Kindle controller reads them from.

To add the blueprint (needs HA 2024.10 or later):

1. Copy `aircon_rooms.yaml` to `config/blueprints/automation/aircon/` in HA, then *Developer tools → YAML → Reload automations*. Once this repo is reachable online, *Settings → Automations & scenes → Blueprints → Import blueprint* with the file's raw URL does the same, and lets HA re-import newer versions.
2. On the blueprint, choose *Create automation*, pick the Aircon device and the rooms, and save. Change rooms there later.

Without it, Thermostat status reads `Idle: no rooms configured`, and Continuous mode still works.

`firmware/thermostat_test/thermostat_test.cpp` checks the thermostat's decisions on a PC: `g++ -std=c++17 -Wall -o thermostat_test thermostat_test.cpp && ./thermostat_test`.

---

## 4. Bring-up

1. **ESP32 board alone, on the bench.** Feed J_MOD through ~20 Ω from a 19 V supply to imitate the module. Check:
   - VRES charges and the buck starts.
   - Wi-Fi connects.
   - The pulses on the scope match the original captures.
2. **Tablet output.** Connect a 220 Ω 2 W resistor to J_TAB. After ~40 s, Home Assistant should show Tablet output status "On", with Tablet output voltage ≈ 15–16 V and roughly 70 mA flowing. Short J_TAB briefly: within ~2 s the status should change to a fault and the output switch off.
3. **Tablet board, on the bench.** Feed it from the ESP32 board's J_TAB, then:
   - with no load, check ≈ 5.07 V at USB
   - with a USB load tester, check the current limit settles at ~123 mA and the ESP32 board's VRES stays above 15 V
4. **Install.** Connect the ESP32 board at the module and the tablet board at the wall. Check that VRES minimum and Tablet output sheds stay healthy over a day.

---

## 5. Assumptions (not verified)

- **Running current is assumed to be ~100 mA.** The meter reading while running was disturbed by the meter itself. A measurement through a 1 Ω resistor would confirm it.
- **The module is assumed to tolerate ~90–105 mA continuously,** as it did with the original pad.
- **Frame meaning and timing:** see the notes file. Bits 2/3 are unknown, and timings are ±0.2 ms from screenshots.
- **Charging behaviour varies by tablet.** Most Android tablets reduce their charge current when VBUS sags, but check yours on the bench (step 3). The Kindle Paperwhite 10th gen doesn't:
  - With JP1 open it sees a computer port that never sets it up as a USB device, and doesn't charge. **Close JP1** so it sees a dedicated charger.
  - Even then, its charger's adaptive input limit trips on the TPS2553's limit and suspends. The Kindle controller detects this and sets a fixed 100 mA input limit (see `kindle-controller/src/charge_guard.rs`). Measured on the bench: about +50 mA into the battery, with VBUS at 5.0 V.
- **Part pinouts:** check Q_TX (1 G, 2 S, 3 D), BCP53 (1 B, 2 C, 3 E, tab C) and the LP2950CDT (1 IN, 2/tab GND, 3 OUT) against the actual parts you buy.
- **LP2950 output capacitor:** some LP2950 makes need a tantalum or a minimum ESR on the output. Check the datasheet of the brand you buy against C_5V (2.2 µF ceramic).
- **AUX:** the module side loads AUX with roughly 15k (it sits at ~3 V with the original pad). What it means is still unknown; the firmware reports it to Home Assistant.
