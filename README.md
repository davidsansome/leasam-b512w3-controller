# Leasam B512W3 aircon wall controller — reverse engineering and replacement

This repo documents how a **Leasam Controls B512W3** wall pad talks to its outdoor relay module, and replaces it with an ESP32 board (controlled from Home Assistant) plus a repurposed Kindle as a touchscreen wall controller.

## Safety

> **⚠️ Use at your own risk.**
>
> - This project reverse-engineers and replaces part of the control electronics for a **mains-powered air conditioner / heat pump**. Getting it wrong can damage the unit, create a shock or fire hazard, or trip breakers in ways that aren't obvious from outside.
> - None of this is reviewed, endorsed, or supported by Leasam, your HVAC manufacturer, or your installer. It is **not a certified replacement part**.
> - It may **void your warranty**, your insurance, or breach local electrical regulations in your area — check before you start.
> - If you are not confident working inside a powered appliance, get a licensed electrician or HVAC technician to help, or to review your wiring before you connect anything.
> - Every measurement, assumption and bit of "guesswork" in these notes was true for **one specific unit**. Other Leasam models, firmware revisions, or installations may differ in ways that aren't safe to assume away.
> - **Never connect a prototype or replacement board and the original pad to the module at the same time.** Turn the aircon off at the isolator before changing any wiring.

## What's here

| | |
|---|---|
| [`leasam-b512w3-notes.md`](leasam-b512w3-notes.md) | Reverse-engineering notes on the original wall pad: how it talks to the outdoor relay module (protocol, timing, voltages), what's verified vs. guessed, and open questions. Start here. |
| [`pcb-images/`](pcb-images/) | Photos of the original pad's board, used for tracing the circuit. |
| [`scope-screenshots/`](scope-screenshots/) | Oscilloscope captures of the pad's signal, for each mode and fan speed. |
| [`replacement-board/`](replacement-board/) | The replacement: an ESP32 board that drives the relay module the way the original pad did (plus a thermostat), a second board that trickle-charges a tablet over the old in-wall cable, their firmware, and the KiCad designs. |
| [`kindle-controller/`](kindle-controller/) | A jailbroken Kindle Paperwhite, running a Rust e-ink UI, as the wall-mounted touchscreen — talking to Home Assistant, not to the relay module directly. Includes a 3D-printable wall case. |

## Suggested reading order

1. [`leasam-b512w3-notes.md`](leasam-b512w3-notes.md) — what the original pad does and how it was figured out.
2. [`replacement-board/README.md`](replacement-board/README.md) — the replacement electronics and firmware that take over that job.
3. [`replacement-board/breadboard-prototype.md`](replacement-board/breadboard-prototype.md) — a bring-up/test build using only easy-to-source parts, before committing to the PCB.
4. [`kindle-controller/README.md`](kindle-controller/README.md) — the wall-mounted touch UI.

## License

See [LICENSE](LICENSE).
