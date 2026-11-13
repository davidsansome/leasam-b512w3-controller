# Breadboard prototype: ESP32 → relay module

This is a minimum test to check that an ESP32 can drive the relay module the way the original pad did. It uses only parts that are easy to get: an ESP32 dev board, resistors, capacitors, small-signal BJTs, diodes, a fuse and an adjustable buck module (MP1584 or LM2596).

It's built in two stages:

1. **Stage 1: transmit only.** The ESP32 is powered from USB and only sends frames. This answers the main question: does the module act on our frames?
2. **Stage 2: self-powered.** The ESP32 runs from the module's POWER line, like the real board will.

Sketch: [`firmware/aircon_controller/aircon_controller.ino`](firmware/aircon_controller/aircon_controller.ino), the same firmware as the PCB. It builds for the breadboard when compiled for a classic ESP32 (`esp32:esp32:esp32`), and for the PCB when compiled for an ESP32-C3. It is controlled from Home Assistant over MQTT: copy `secrets.example.h` to `secrets.h` (which git ignores) and fill in the Wi-Fi and MQTT broker settings, and it appears as an **Aircon** device with Power, Mode (Cool / Heat / Fan only) and Fan speed controls, a Compressor running sensor, and diagnostics for AUX voltage (10 s mean, minimum and maximum), Wi-Fi signal and compressor lockout. It needs the PubSubClient library, and Home Assistant needs the MQTT integration (e.g. the Mosquitto broker add-on). Later firmware updates can go over Wi-Fi (ArduinoOTA, host `aircon-test`). Pins below are for a classic ESP32 DevKit (ESP32-WROOM-32). An ESP32-C3 dev board won't work as-is: a C3 build uses the PCB's pins and expects its tablet output circuit.

Stage 1 schematic: [`kicad/breadboard-stage1/`](kicad/breadboard-stage1/) (KiCad 10, plus a PDF). In the schematic, R_TX is **R4**, R_AUX is **R5**, and the AUX read divider is **R6 / R7 / C2**. Every other designator matches this page.

Stage 2 schematic: [`kicad/breadboard-stage2/`](kicad/breadboard-stage2/). It is the whole stage 1 circuit plus the power section in §3. There, R_IN is **R8**, the buck module is **U2**, the VRES divider is **R9 / R10**, and the 5 V bulk capacitor is **C4**.

> **Never connect the original pad and the prototype at the same time.** Turn the aircon off at the isolator before changing any wiring.

---

## 1. Transmitter (both stages)

The original pad pulls POWER to COMMON through 3.3 Ω for ~15 µs (Q2 + R4). Here, two NPNs in a Darlington do the same job. One transistor can't do it alone: the pulse current is ~0.8 A, and at that current even an S8050 has an hFE of only ~40 (datasheet minimum). That would need ~20 mA of base current, more than a GPIO should supply.

```
 POWER ────────────────┐
                       R_TX ≈ 3.3 Ω   (2× 2 Ω in series = 4 Ω)
                       │
                       ├────────────────┐
                       │ C              │ C
 GPIO25 ──C1──┬──R1──B─┤ Q1             │
         220nF│  470Ω  │ E ─────────┬─B─┤ Q2
              │                     │   │ E
          D1 ─┴─ (cathode up)      R3   │
          P6KE15A     R2 10k     470 Ω  │
              │        │            │   │
 COMMON ──────┴────────┴────────────┴───┴──── ESP32 GND
```

| Part | Value | Why |
|---|---|---|
| Q2 | **S8050** (or BC337) | carries the 0.8 A pulses. S8050: 1.5 A max, hFE ≥ 40 at 800 mA. BC337: 800 mA continuous, 1 A peak. The pulses are 15 µs at 0.03% duty, so there's no heating. |
| Q1 | **2N3904** (or BC547, S9013, 2N2222) | only carries Q2's base current, ~20 mA. GPIO base current × hFE² ≫ 0.8 A. |
| R_TX | 3–4 Ω: 2× 2 Ω in series, or 7× 22 Ω in parallel (any wattage; average power is under 1 mW) | sets the pulse current, as R4 does in the pad. It isn't optional: the module's source may be stiffer than 20 Ω for short pulses (the notes saw 1.3 V where 2.7 V was expected), and R_TX caps the current if so. A single 22 Ω is too much: POWER would only fall to ~10 V. |
| C1 | 220 nF (anything 100 nF–1 µF, non-polarised) | **AC coupling:** a crashed or booting ESP32 that holds the pin high can only turn the transistors on for ~0.5 ms, not short the module's supply |
| R1 | 470 Ω | base current ≈ 4 mA |
| D1 | P6KE15A (any P6KE from 10A up, or a 1N4148). Band (cathode) to the C1/R1 junction, other end to COMMON. | Resets C1 on the falling edge so every pulse is the same. It only ever conducts forwards, so a unidirectional TVS acts as an ordinary diode here. It sees at most ~2 V reverse. Avoid the P6KE6.8A, which has more leakage and capacitance. |
| R2 | 10 kΩ, C1/R1 node to COMMON | keeps the input defined when idle |
| R3 | 470 Ω, Q2 base to COMMON | turns Q2 off quickly at the end of each pulse |

Wire D1 and R2 from the C1/R1 junction to COMMON. Wire R3 from the Q1 emitter/Q2 base junction to COMMON.

- **Don't use for Q2:** small 100–200 mA parts (2N3904, BC547, S9013/9014, C945, C1815) or the high-voltage 2N5551. They're fine as Q1.
- **PNP parts won't work in this circuit:** 2SA1015, BC327, BC557, 2N2907, 2N3906, 2N5401, S8550, S9012, S9015.
- **Pinouts differ** (TO-92, flat face towards you, legs down): S8050, 2N3904, 2N2222 and S9013 are **E B C**. BC337 and BC547 are **C B E**. C1815 is **E C B**. Check the datasheet for your exact part's markings.
- **Wiring:** keep the POWER → R_TX → Q2 → COMMON loop short. Use solid wire, or wire it on a screw terminal rather than through long breadboard rows, since it carries ~0.8 A pulses.
- **No fuse in this branch.** A fuse adds 1–2 Ω in series, which makes the pulses shallower. The multimeter shunt did the same thing when the "E-C" error appeared. C1 is the protection here. The module's ~20 Ω source resistance also limits a dead short to < 1 A.

**What to expect on the scope** (POWER vs COMMON): an idle level of ~19 V, then 9 pulses down to ~3–4 V. The Darlington's ~1 V drop makes them a bit shallower than the original ~1.3 V. Gaps are 1.92 ms or 5.76 ms, and frames repeat every 450 ms. If the module ignores the frames or shows "E-C", reduce R_TX to ~2 Ω (a single 2 Ω, or 10× 22 Ω in parallel) to make the pulses deeper.

## 2. AUX (both stages)

The original pad holds AUX up with 10 kΩ to its 5 V rail (R66). The module may expect this, so copy it. The divider lets the sketch log AUX as well.

```
 ESP32 5V pin ── R_AUX 10k ──┬── AUX
                             └── 100k ──┬── GPIO34
                                       33k  (‖ C2, optional)
                                        │
                                      COMMON
```

**C2 is optional. Leave it out at first.** The PCB uses 10 nF here. The smallest common part, 0.22 µF, would make a ~5 ms filter with the divider, and that could smooth away fast AUX signalling. The ESP32's ADC reads the divider's ~25 kΩ source well enough without it. Fit 0.22 µF only if the readings are noisy.

In stage 1, the dev board's 5V pin is USB VBUS. In stage 2, it's the buck output.

## 3. Power from the POWER line (stage 2 only)

```
 POWER ── F1 ──►|── R_IN ──┬───────────── VRES ── buck IN+    buck OUT+ (5.0 V) ── ESP32 5V/VIN
       RXEF010  D2   47 Ω  │                                                      
                           C3 470–1000 µF, 35 V            VRES ── 100k ──┬── GPIO35 (optional)
                           │                                             10k
 COMMON ───────────────────┴──────────────── buck IN−     buck OUT− ─────┴── ESP32 GND
```

| Part | Value | Why |
|---|---|---|
| F1 | RXEF010 resettable PPTC fuse (0.1 A hold, ~0.2 A trip, 72 V) | protects the module and R_IN from a short in the supply branch. R_IN and the module's ~20 Ω limit a short after R_IN to ~0.28 A, so F1 must trip below that: an RXEF050 (1 A trip) or a 250 mA glass fuse would never trip. It's not in the TX path, so its ~1–2 Ω resistance doesn't matter here. |
| D2 | P6KE30A (or 36A / 51A) used forwards, band towards R_IN (or a 1N4007 / 1N5819) | isolates the reservoir from the line while it's pulsed low, like D1 in the pad and D_IN on the PCB |
| R_IN | 47 Ω, ½ W (or 2× 100 Ω ¼ W in parallel) | limits the inrush into C3 to ≈ 0.28 A for a few tens of ms. Steady-state drop is ~2 V. |
| C3 | 470–1000 µF, 35 V (25 V minimum) | covers Wi-Fi current bursts |
| buck (U2) | MP1584 module (4.5–28 V in), set to **5.0 V before connecting the ESP32** | the dev board's own regulator makes 3.3 V |
| C4 | 470–1000 µF electrolytic, 10 V or more, + to 5V, right at the ESP32's 5V and GND pins | supplies the ESP32's Wi-Fi start-up bursts locally. Without it the 5 V rail dips each time Wi-Fi starts, the board resets (red LED flickers) and never joins the network, even though a meter shows a steady 5 V. |
| R9 / R10 | 100k / 10k, VRES to GPIO35 | lets the ESP32 read VRES, to check the supply holds up when Wi-Fi transmits. The sketch publishes the 10 s average as "VRES" and the lowest sample as "VRES minimum". Without R9/R10 fitted, GPIO35 floats and both readings are meaningless. |

**Why 47 Ω and not more:** the buck is a constant-power load (~0.6 W with Wi-Fi). With 19 V behind ~20 Ω (module) + R_IN, the most power the line can deliver is V²/4R. At R_IN = 100 Ω that's only ≈ 0.7 W, so the supply would collapse. At 47 Ω it's ≈ 1.2 W, and VRES settles around 14–15 V.

**Setting the MP1584 to 5.0 V:** build F1, D2, R_IN and C3, and connect the MP1584's input only. Power the aircon and put a multimeter on OUT+/OUT−. Turn the brass multi-turn trimmer until it reads 5.0 V. It can take several turns before anything changes, and the direction varies between makers. Then turn the aircon off and connect OUT+ to the ESP32's 5V pin. It can't be set from USB 5V, because the input must be higher than the output.

**USB and stage 2:** don't plug USB in while the board is powered from the buck, unless you know your board has a diode between VBUS and 5V. USB is only needed for the first flash; after that, updates can go over Wi-Fi and control is through Home Assistant.

---

## 4. Test procedure

### 4.1 Bench (no aircon)

A 19 V laptop power brick makes a good imitation of the module. Feed it to POWER/COMMON through a **22 Ω** resistor to match the module's source resistance.

1. Fill in `secrets.h` (copied from `secrets.example.h`) and flash the sketch over USB. The **Aircon** device should appear in Home Assistant (Settings → Devices & services → MQTT) with Power off.
2. Stage 1: with the scope on POWER, check the pulses as described in §1. Turn Power on with Fan speed High and Mode Fan only, then switch Mode to Cool and Fan speed to Medium. The gaps should change to match the table in the notes. The compressor bits come on after a 3-minute lockout; the Compressor lockout diagnostic shows the countdown.
3. Stage 2: set the buck to 5.0 V with the ESP32 disconnected (see §3). Then connect the ESP32, disconnect USB, and power from the brick or the aircon. VRES (a meter on C3) should sit at ~14–17 V. Check it doesn't dip below ~9 V when Wi-Fi connects.

### 4.2 On the aircon

1. Turn the aircon off at the isolator. Disconnect the pad's POWER, COMMON and AUX wires. Connect them to the prototype. Do stage 1 first.
2. Turn the aircon on. With Power off, the sketch sends the "off" frame. Check the display shows no error (wait ~1 minute).
3. Set Mode to Fan only, turn Power on, then step Fan speed through Low, Medium and High. The indoor fan should follow within a frame or two.
4. Set Mode to Cool and Fan speed to Low. After the 3-minute lockout, the compressor and outdoor fan should start.
5. Turn Power off. Try Mode Heat only after the 3-minute lockout has passed again.
6. Record the AUX voltage diagnostic in each state (it's averaged over 10 s). They are the first data on what the module sends back.

If the module doesn't respond, compare the scope trace with `scope-screenshots/` (pulse depth first, then the gaps). Try a lower R_TX before anything else.

## 5. Differences from the PCB design

| | Breadboard | PCB |
|---|---|---|
| TX switch | small-signal BJT Darlington (2N3904 + S8050), Vce ≈ 1 V | AO3422 MOSFET (deeper pulses) |
| Input limiting | 47 Ω resistor | ≈ 105 mA two-transistor current limiter |
| 3.3 V | MP1584 → 5 V → dev board LDO | LMR51606 buck straight to 3.3 V |
| AUX read divider | 100k / 33k (÷4.0) | 330k / 100k (÷4.3) |
| Tablet output | none | limiter 2 + J_TAB |
