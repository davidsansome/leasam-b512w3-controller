# Leasam B512W3 wall controller – reverse engineering notes

Board: **Leasam Controls B512W3 23221-3** (back silkscreen). Label: B51281 W/C, S/N 03K4820, 512C8V63.
Terminals: **SENS 1, SENS 2, AUX, COMMON, POWER**.
Installed wiring uses **POWER, COMMON and AUX** only.

Photos: `pcb-images/`. Scope captures: `scope-screenshots/`.

---

## 1. Verified by tracing (continuity checks)

### Microcontroller
- U6 is an **ST72C334J4** (TQFP-44). Pin functions below are from the ST72334J datasheet pin table.

### AUX input
- AUX connects to **R66** and **R55** (top pads). **Nothing else** is connected to AUX.
- **R66** other end → **U8 pin 8**. U8 pin 1 is top-left; pin 8 has continuity with **C3** (logic supply rail).
- **R55** other end → **bottom of C8** → **U6 pin 11 (PD4/AIN4)**.
- **Top of C8** → ground.
- R55 and R66 are both marked `1002` (10 kΩ).

```
logic supply (C3 / U8 pin 8)
        │
       R66 10k
        │
AUX ────┴──── R55 10k ──┬── U6 pin 11 (PD4/AIN4)
                        C8
                        │
                       GND
```

- From the datasheet: **PD4/AIN4 is plain I/O or ADC input only**. It has no external interrupt, no timer input capture and no SCI (UART) function. The UART pins (PE1/RDI pin 1, PE0/TDO pin 44) are **not** connected to AUX.

### Sensor inputs (same structure as AUX)
- **SENS 1** → top of **R62** and **R63**, and one pin of the on-board thermistor TH1. Other side → **U6 pin 8 (PD1/AIN1)**.
- **SENS 2** → top of **R64** and **R65**. Other side → **U6 pin 9 (PD2/AIN2)**.

### POWER line and Q2
- **D1 and ZD1** left pins → **POWER**.
- **R4** (marked `3R30`, 3.3 Ω) left pin → the same D1/ZD1/POWER node.
- **R4** right pin → **Q2 middle pin**.
- **Q2 top pin** → **COMMON**.
- **Q2 bottom pin** → **R6** (approx. 3.2–3.3 kΩ) → **U6 pin 12 (PD5/AIN5)**.
- From the datasheet: **PD5/AIN5 is also plain I/O or ADC only** (no timer, interrupt or UART function).

```
POWER ──┬── D1 ──► pad supply
        │
       R4 3.3Ω
        │
     Q2 middle
     Q2 bottom ◄── R6 ~3.3k ◄── U6 pin 12 (PD5)
     Q2 top
        │
COMMON ─┘
```

---

## 2. Measured with the scope (probe on POWER, ground on COMMON)

Values are read from the screenshots, so they are approximate.

### Waveform
| Property | Value |
|---|---|
| Idle level | ~19 V (off), ~16 V (cooling) |
| Pulse depth | down to ~1.3 V |
| Pulse width | ~25–30 µs total (`off-20us.png`) |
| Pulse shape | ~9 µs fall, ~6 µs at bottom, rises in two steps with a brief shelf at ~10 V |
| Pulses per frame | 9 (every capture) |
| Gaps between pulses | two lengths only: **short ≈ 1.9 ms**, **long ≈ 5.8 ms** (3× short) |
| Frame repeat | ~every **450 ms**, also when off (`off-200ms.png`) |
| Other | Rounded sags ~10 ms apart in heat-mode captures (not part of the pulse train) |

### Decoded frames
Each frame has 8 gaps. Writing **long gap = 1, short gap = 0**, in transmission order:

| State | Bits (gap 1 → 8) |
|---|---|
| off | `00000000` |
| fan high | `10000000` |
| fan medium | `00000001` |
| fan low | `00000100` |
| cool high | `10011000` |
| cool medium | `00011001` |
| cool low | `00011100` |
| heat high | `10011010` |
| heat medium | `00011011` |
| heat low | `00011110` |

Patterns that hold across all captures:
- Fan high → bit 1; fan medium → bit 8; fan low → bit 6. Exactly one is set whenever the system is on.
- Bits 4 and 5 are set in both cool and heat, and in neither fan-only nor off.
- Bit 7 is set only in heat.
- Bits 2 and 3 were never seen set.
- An earlier heat-mode capture showed only a fan bit, apparently taken before the system started calling for heat (not kept).

The scope's `.bin` save files contain no waveform data. All were identical: 805 bytes, all zeros except `01 01` at the end.

---

## 3. Measured with a multimeter (POWER line)

### Supply capability (original pad disconnected)
| Load across POWER–COMMON | Voltage | Current (V/R) |
|---|---|---|
| none (open circuit) | 19.0 V | 0 |
| 1 kΩ | 18.50 V | 18.5 mA |
| 470 Ω | 18.11 V | 38.5 mA |
| 100 Ω | 15.80 V | 158 mA |

→ The module's POWER output behaves like **≈ 19 V behind ≈ 20 Ω** (≈ 23–27 Ω at light loads). The 100 Ω test was only brief, so long-term capability above ~100 mA is unknown.

### Original pad's current (meter in series with POWER)
| State | Current |
|---|---|
| Off | 72 mA |
| Running | 90–100 mA (unreliable, see below) |

### Observations
- **With the meter in series, the running test failed:** the display showed **"E-C"** and the compressor switched off. The meter's shunt resistance in series with POWER is the likely cause (see guesswork). The meaning of "E-C" is unknown; no Leasam error-code list was found.
- **Earlier, probing the line with a multimeter** sometimes made the aircon turn on or the controller show an error. This was recorded before we knew which wire was which.

---

## 4. Guesswork – NOT verified

These are interpretations. None have been confirmed by tracing or a targeted test.

### Signalling
- **The pad transmits to the relay module by pulling POWER low through Q2/R4.** The topology fits, but no capture yet shows Q2's drive (R6 / U6 pin 12) alongside the POWER dips.
- Q2 pin roles are assumed from the standard SOT-23 BJT layout: middle = collector, top = emitter, bottom = base.
- The source resistance itself is measured (§3). What isn't explained: with ≈ 20 Ω behind 19 V, the pad's 3.3 Ω pulse should only pull the line to ≈ 2.7 V, but the scope showed ≈ 1.3 V. Possible explanations: the source is softer for short pulses (e.g. inductance or a series resistor after a capacitor), or the screenshot ground reference is slightly off.
- **"E-C" is probably a communication error.** A meter on a mA range adds several ohms in series, so the pad's pulses would reach the module much shallower (e.g. ≈ 7–8 V instead of ≈ 2–3 V with a 10 Ω shunt), causing missed or corrupt frames. The meaning of the code is not confirmed.
- The ~10 mA rise in the pad's draw when running is probably its extra display LEDs. It's unconfirmed because of the meter problem above.
- **AUX is probably the return channel (module → pad)**, read by the ADC on PD4. Its meaning is unknown: serial data, a multi-level status, a remote on/off or fault input, or a sensor. It has the same circuit shape as the SENS inputs. The pad itself cannot drive AUX hard. At most it could pull to ~2.5 V via PD4 and R55.
- Loading AUX (e.g. with a multimeter) probably shifts the voltage the ADC sees. That would explain the controller erroring or the aircon turning on when probing.
- Both protocol pins are plain GPIO/ADC, so the timing is probably generated in software (consistent with the slow ms-scale gaps).

### Meaning of the bits
- The frame probably carries **one bit per relay** in the relay module.
- Bits 4 and 5 are probably compressor and outdoor fan.
- Bit 7 is probably the heat-pump reversing valve (the DIP switch is set to HEATPUMP).
- Bits 2 and 3 might be second-stage or supplementary heat (the display has a STAGE legend) or an auxiliary output.
- Bit polarity (long = 1) is an assumption. It's chosen because "off" then reads as all zeros.

### Waveform details
- The ~10 V shelf on each pulse's rising edge is probably D1 conducting again while the pad's capacitors recharge.
- The ~10 ms sags in heat mode are probably 100 Hz ripple from a rectified 24 V AC supply in the relay module, under heavier load.

### Other components
- **X2** marked "8,00 C": probably an 8.00 MHz ceramic resonator. That gives a 4 MHz CPU clock. C14/C15 being unpopulated fits a resonator with built-in capacitors.
- **U2** marked "8L05A": probably an L78L05-style 5 V regulator (logic supply).
- **U7** (LM393) and **U8** (marked "912", probably TS912 op-amp): roles not traced. Not in the AUX path apart from U8 pin 8 being used as a supply tie point for R66.
- **U5** M41T56 RTC with X1 32.768 kHz crystal and B1 CR2032 backup: clock / timer functions.
- **P2** (unpopulated DIP-style footprint near U6, beside R24–R31): possibly the ST7 in-circuit programming header. If readout protection isn't set, the firmware might be readable.
- The 2×10 pad array and two 7-pad rows are the through-hole leads of the 7-segment display and the DIP switch SW1 (from comparing front/back photos, not continuity).

---

## 5. Open questions / next steps
- Measure the pad's **running current** without disturbing the signalling: use a 1 Ω resistor in series and read the voltage across it in mV or on the scope.
- Check whether the module tolerates **> 100 mA for long periods** (e.g. 150 Ω for 10–15 min, feeling for warm parts).
- Capture **R6 / U6 pin 12** on CH2 alongside POWER to confirm the dips are the pad's.
- Capture **AUX** (scope, not logic analyser) during state changes to find out what the module sends back.
- Find states that set **bits 2 and 3**.
- Measure the AUX voltage with only the relay module connected, to see whether the module also pulls it up.
- Optionally check P2 → U6 continuity against the ST7 ICC pins.
