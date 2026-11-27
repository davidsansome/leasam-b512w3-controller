# Design changes

## 2026-11-10: parts chosen for JLCPCB

Every part on both boards now has an LCSC/JLCPCB part number. Where the chosen part differed from the schematic value, the value was changed to match the part.

### ESP32 board

| Change | Part | Effect |
|---|---|---|
| C_RES: 1000 µF 50 V → 63 V | C2, C3 | More margin. The 16 × 25 mm can is 25 mm tall. |
| C_IN: 2.2 µF 100 V → 50 V X7R | C4 | Less margin than before, but VRES stays ≤ 19 V and D_TVS clamps at ≤ 48 V. |
| L1: 8.2 µH → 10 µH | L1 | The same SRN4018-100M as the tablet board. TI's table suggests 8.2 µH for 3.3 V at 1.1 MHz; 10 µH gives a little less ripple and a slightly slower load response. |
| C_OUT1/2: 22 µF 10 V → 25 V | C7, C8 | More margin. The chosen part (C45783) is X5R, not X7R, which is fine at 3.3 V. |
| C_OUT3: 100 µF 6.3 V tantalum → 100 µF 10 V KEMET T520 polymer tantalum (70 mΩ) | C9 | At 3.3 V it's well inside the derating, and polymer parts don't fail short the way standard tantalums can. Check the 3.3 V rail on a scope during Wi-Fi start-up: the extra low-ESR capacitance is well above TI's 10 µF example. |
| LED1: green → red | D3 | Matches the part (KT-0603R). About 1.3 mA through the 1k. |

### Tablet board

Four resistors moved to standard E24 values, the 5 V feedback divider was rescaled, and three capacitors changed:

| Change | Part | Effect |
|---|---|---|
| R_UV2: 158k → 150k | R2 | UVLO on ≈ 9.0 → ≈ 9.4 V (≤ 10.4 V worst case), off ≈ 7.3 → ≈ 7.6 V. Still well below the ~13.5 V worst case at the board. |
| R_FBT / R_FBB: 118k / 22.1k → 160k / 30k | R3, R4 | Same 5.07 V typ. With the LMR51606's 0.788–0.812 V reference and 1% resistors, the output stays within ≈ 4.91–5.23 V, inside USB 2.0's 5.25 V. (120k / 22k was tried first, but its worst case was ≈ 5.33 V.) |
| R_ILIM: 232k → 220k | R5 | USB limit ≈ 117 → ≈ 123 mA typ. Worst case is now ≈ 0.74 W out and ≈ 64 mA in (was ≈ 61 mA), still under limiter 2's ≈ 79 mA minimum. |
| C_BULK: radial THT → SMD electrolytic (6.3 × 7.7 mm) | C1 | Same 100 µF 35 V. |
| C_O1/2: 10 µF 25 V X7R → 50 V X5R | C4, C5 | More margin; fine at 5 V. |
| C_VBUS: 100 µF 6.3 V tantalum 3528 → 100 µF 16 V radial THT (6.3 mm, 2.5 mm pitch) | C7 | A 6.3 V tantalum at 5.07 V was over its derating. |

## 2026-11-08: removed the ESP32 board's USB-C port

Removed J4 (USB-C), U4 (USBLC6-2SC6) and R26/R27 (5.1k CC pull-downs), with nets USB_DP, USB_DM, USB_DP_C, USB_DM_C, USB_CC1, USB_CC2 and USB_VBUS. IO18/IO19 (module pins 13/14) are now unconnected. This reverses the USB-C row in the 2026-11-04 review changes below.

The firmware updates over Wi-Fi (OTA), so a wired connection is only needed for the first flash and for recovery. J_PROG already covers that with a USB-UART adapter (README section 1.4). Dropping the port saves two likely JLCPCB Extended parts, board area and the D+/D− routing. What's lost is flashing without pressing anything, and the plug-in serial console and JTAG.

## 2026-11-08: schematics redrawn with wires

Both board schematics were redrawn with wires, standard KiCad symbols and labelled functional blocks, replacing the one-box-per-part, label-on-every-pin drawing. Part UUIDs, references, values and footprints are unchanged, and the exported netlists matched the netlist files.

- Project-local symbols (transistors renumbered to the SOT-23/SOT-223 pin order, LMR51606, TPS2553, a function-grouped ESP32-C3-WROOM-02) are in each project's `.kicad_sym`, with a `sym-lib-table`.
- GND, 3V3, VRES and 5V now use power symbols, so they're global nets: Update PCB from Schematic renames `/GND` to `GND` and so on. The ESP32 board's Power netclass pattern is now `^/?(…)$` so those nets keep the Power class.

## 2026-11-04: ESP32 module unused pins

Added IO6 (pin 5) and IO7 (pin 6) to the ESP32 symbol with no-connect flags, and no-connect flags on J4's SBU pins. They're unused, but pcbnew warned that the footprint pads had no matching symbol pins.

## 2026-11-04: USB connector shield pins

The USB connector symbols numbered their shield pin "5" (tablet board J2) and "S1" (ESP32 board J4), but the KiCad footprints call those pads "SH". pcbnew couldn't match them, so the shields had no net. Both are now "SH" on GND. All other symbols' pin numbers match their footprints.

## 2026-11-04: removed the superseded single-board design

Deleted `kicad/aircon-replacement.*` (schematic, PCB, project) and `easyeda/`. They described an earlier single-board revision (18 Ω input limiter, no tablet output, no board outline) and were replaced by `kicad/esp32-board/` and `kicad/tablet-board/`. They're still in the git history if needed.

## 2026-11-04: fixes from the design review

These changes come from a review of the notes, schematics and firmware. The PCB layout for the current schematics hadn't been done yet, so the review looked at the schematics only. The netlists and designator lists were regenerated from the edited schematics, and every pin connects as intended.

### ESP32 board

| Change | Parts | Why |
|---|---|---|
| Removed JP_AUX. R_AUX (10k) now permanently ties AUX to a new 5 V rail. | JP1 removed; R1 now AUX → 5V_AUX | The original pad loads AUX with 10k to its 5 V logic rail (R66). With the pad connected, AUX sits at ~3 V, so the module side has a real load of roughly 15k. Leaving AUX floating (the old default) or pulling it to 3V3 would present the module with something it never saw before. |
| Added a 5 V rail for the AUX pull-up | U3 LP2950CDT-5.0 (VRES → 5V_AUX), C15 2.2 µF | The board had no 5 V. The LP2950 takes up to 30 V in and draws ~75 µA. |
| AUX is now read on GPIO0 | R24 330k, R25 100k, C16 10 nF → IO0 (WROOM-02 pin 18) | We don't know what AUX means yet. "E-C" suggests the module may report something back, so the firmware should be able to log AUX over time (for example through a heat-pump defrost). The ÷4.3 divider reads up to ~10 V, and the 330k keeps the current into the ESP32 tiny if AUX ever rises to 24 V. The 10 nF filters most of the mains hum seen on AUX while the compressor runs. IO0 was the last free ADC1 pin. |
| Q_TX: DMN6140L → AO3422 | Q1 | The DMN6140L's V_GS(th) can be up to 3 V, and its R_DS(on) is only specified at 4.5 V and above. The ESP32 drives it at 3.3 V, so a high-threshold part could barely turn on at 0.8 A and give shallow pulses, the same failure that probably caused "E-C". The AO3422 is specified at V_GS = 2.5 V (≤ 200 mΩ), with V_GS(th) ≤ 2 V. Same SOT-23 pinout. |
| D_TVS: SMBJ33A → SMBJ30A | D1 | The AO3422 is rated 55 V. The SMBJ33A can clamp at up to ~53 V, which leaves too little margin. The SMBJ30A clamps at ≤ 48 V, and the line never goes above 19 V. |
| R_S2: 8.2 Ω → 7.5 Ω (limiter 2 ≈ 79 → ≈ 87 mA) | R17 | See "Tablet supply" below. |
| Added a USB-C port for flashing and debugging | J4 USB-C, U4 USBLC6-2SC6, R26/R27 5.1k on CC1/CC2; IO18/IO19 (pins 13/14) | Makes flashing and debugging possible over the C3's built-in USB-Serial-JTAG, without a USB-serial adapter. The 5.1k resistors let USB-C hosts enable the port. VBUS goes only to the ESD array and doesn't power the board. Check COMMON against earth before plugging in a mains-powered PC (see the review). |

### Tablet board

| Change | Parts | Why |
|---|---|---|
| R_ILIM: 200k → 232k (USB limit ≈ 135 → ≈ 117 mA typ.) | R5 | See "Tablet supply" below. 232k is the top of the TPS2553's recommended range. |

### Tablet supply (why R_S2 and R_ILIM changed together)

The tablet board's buck is a constant-power load. In the old design, its worst-case input (≈ 69–72 mA) was close to limiter 2's worst-case minimum (≈ 73 mA, after base drive and V_BE spread). If limiter 2 had clamped, the input voltage would have fallen, the current would have risen, and the board would have cycled through UVLO. The tablet would then see the charger connect and disconnect every minute.

Now the TPS2553 is the limit that matters in normal running. The board's worst case is ≈ 61 mA, and limiter 2 only clamps above ≈ 79 mA, so it acts only on a short circuit. A shorted cable plus the ESP32 still stays under limiter 1's 105 mA, so the total draw from the module doesn't change. The cost is about 0.1 W less into the tablet.

### Firmware

The firmware will be rewritten later. Interim changes:

- `TAB_ON_MIN_V`: 15 → 14 V. While cooling, the line sags to ~16 V, which leaves VRES ≈ 15.0 V. At a 15 V threshold, the tablet output might never come back on after a shed.
- AUX monitoring on GPIO0: `/state` reports the AUX mean, min, max and hum each second, and `/aux` lists the last 64 level changes.

### Not changed, still open

- ~~**Thermostat:** room temperature will come from a separate Wi-Fi sensor, outside this design. The firmware rewrite needs a setpoint, hysteresis and a fallback for when the sensor goes quiet.~~ Done since: the firmware has a thermostat (set point, hysteresis, minimum run time, idle with no fresh reading), fed with room temperatures from HA by the *Aircon rooms* blueprint. See [README §3, Thermostat](README.md#thermostat).
- ~~**What AUX means**~~ Found since: AUX reports the condenser-water flow interlock (VERIFY), about 1.3 V with flow and 0.6 V without (the original pad's "E-C" is the no-flow error), and the firmware now idles the whole unit with no flow. See [README §3, Flow interlock](README.md#flow-interlock). Whether AUX carries anything else is still unknown.
- What the module does when frames stop, and what bits 2/3 mean (probably zones, going by SW1's ZONES options). These need testing.
- The new parts were added on spare areas of the schematic sheet. Open it in KiCad to tidy the placement and run ERC.
