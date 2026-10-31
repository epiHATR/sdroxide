# Getting started

This page is where to begin if you have not run SDRoxide yet: what you need,
and the path from install to first QSO.

**Install on your OS:** [Installation](INSTALLATION.md) (Linux, Windows, macOS).  
**Operate the radio:** [User Manual](https://github.com/epiHATR/sdroxide/wiki).  
**Which release asset?** [`release/DOWNLOAD.md`](../release/DOWNLOAD.md).

---

## 1. Where to start

1. Install the **Standard** build for your OS — see [Installation](INSTALLATION.md).
2. Plug in the radio (or start with no hardware — see “First launch”).
3. Open SDRoxide, set **My callsign** / **My grid** under Settings if you will
   transmit or upload spots.
4. Choose a backend under **Settings → Radio** (or let the last session restore).
5. For FT8 and friends, open the mode from the strip and work the operating
   panel under the waterfall.

| Goal | Path |
| --- | --- |
| Native desktop with a USB SDR | Install Standard → plug radio → run `sdroxide` |
| Browser UI on the same machine | Install → `sdroxide --server` → open `http://127.0.0.1:4950` |
| Headless station, UI elsewhere | Server on the radio PC → `sdroxide --connect host:4950` on the desktop |
| No hardware yet | `sdroxide --siggen` (built-in signal generator) |

Pre-built packages:
[Releases](https://github.com/epiHATR/sdroxide/releases).

---

## 2. Prerequisites

### All platforms

- A supported radio **or** willingness to use `--siggen` / a recorded IQ file.
- Speakers or headphones (demodulated audio).
- For digi TX: a station callsign and grid (Settings), and a legal setup for
  your licence.

### Choose Standard vs SoapySDR

| Build | Use when |
| --- | --- |
| **Standard** (recommended) | RTL-SDR, RX-888, Airspy HF+, OpenHPSDR, TCI, CAT + USB sound card, and other native backends |
| **SoapySDR** (`-soapysdr` suffix) | Devices that need Soapy modules (HackRF, Airspy R2/Mini, PlutoSDR, …) |

Details: [`release/DOWNLOAD.md`](../release/DOWNLOAD.md). Platform install steps:
[Installation](INSTALLATION.md).

### Optional extras

| Feature | Extra |
| --- | --- |
| USB SDR on Linux | udev rules (`.deb` installs them; AppImage/tarball need a one-time copy) |
| Meshtastic LongFast | `lorarx` on `PATH` (not bundled) |
| HD Radio | `libnrsc5` (not bundled) |
| Spoken announcements | Voice files ship with the package |

---

## 3. First launch

```sh
sdroxide                 # desktop, restores last session
sdroxide --siggen        # no radio
sdroxide --server        # browser UI at http://127.0.0.1:4950
sdroxide --probe         # list devices
```

Embedded web client: every current release binary can serve the UI with
`--server` (no separate web zip required).

Config lives under the platform config directory (User Manual
[§13 Configuration files](https://github.com/epiHATR/sdroxide/wiki/13.-Configuration-files)),
e.g. on macOS `~/Library/Application Support/org.sdroxide.sdroxide/`.

---

## 4. Next steps

- [Installation](INSTALLATION.md) — Linux / Windows / macOS packages  
- [Basic operation](https://github.com/epiHATR/sdroxide/wiki/2.-Basic-operation) — dial, filters, PTT, display strip  
- [Digital modes](https://github.com/epiHATR/sdroxide/wiki/3.-Digital-modes) — FT8 and the rest  
- [Radio-specific notes](https://github.com/epiHATR/sdroxide/wiki/15.-Radio-specific-notes) — per-device quirks  
- [Troubleshooting](https://github.com/epiHATR/sdroxide/wiki/14.-Troubleshooting) — when something will not open or decode  

Building from source: [CONTRIBUTING.md](https://github.com/epiHATR/sdroxide/blob/main/CONTRIBUTING.md).
