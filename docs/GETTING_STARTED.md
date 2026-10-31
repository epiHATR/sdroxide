# Getting started

This page is where to begin if you have not run SDRoxide yet: what you need,
which build to download, and how to install it on Linux, Windows, and macOS.

For day-to-day operation after install, use the
[User Manual](https://github.com/epiHATR/sdroxide/wiki).
For “which asset on the Releases page?”, see
[`release/DOWNLOAD.md`](../release/DOWNLOAD.md).

---

## 1. Where to start

1. Pick your OS section below and install the **Standard** build first.
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

Details: [`release/DOWNLOAD.md`](../release/DOWNLOAD.md).

### Optional extras

| Feature | Extra |
| --- | --- |
| USB SDR on Linux | udev rules (`.deb` installs them; AppImage/tarball need a one-time copy) |
| Meshtastic LongFast | `lorarx` on `PATH` (not bundled) |
| HD Radio | `libnrsc5` (not bundled) |
| Spoken announcements | Voice files ship with the package |

---

## 3. Installation — Linux

### Quick try: AppImage

1. Download `sdroxide-<version>-linux-x86_64-compat.AppImage` (or `aarch64` on
   ARM / Raspberry Pi) from
   [Releases](https://github.com/epiHATR/sdroxide/releases).
2. Make it executable and run:

```sh
chmod +x sdroxide-*-linux-*-compat.AppImage
./sdroxide-*-linux-*-compat.AppImage
```

Needs FUSE (`fuse3`). Without it:

```sh
APPIMAGE_EXTRACT_AND_RUN=1 ./sdroxide-*-linux-*-compat.AppImage
```

### Recommended: `.deb`

1. Download the matching `.deb` (`-compat` for Ubuntu 22.04 / Debian 12 /
   Raspberry Pi OS bookworm and similar; plain for newer glibc).
2. Install:

```sh
sudo apt install ./sdroxide-*.deb
# or
sudo dpkg -i sdroxide-*.deb && sudo apt -f install
```

The package installs the binary, desktop entry, and USB **udev** rules.

### Portable tarball

Unpack and run `./sdroxide`. Install udev rules once:

```sh
sudo cp 60-sdroxide-*.rules /usr/lib/udev/rules.d/
sudo udevadm control --reload
```

Then unplug/replug the dongle.

### SoapySDR builds on Linux

Install Soapy and modules (`libsoapysdr0.8`, `soapysdr-module-all`, ALSA). The
`-soapysdr.deb` declares dependencies. Prefer Standard unless you need a
Soapy-only device.

### Runtime libraries

- **Standard:** ALSA (`libasound2` / `libasound2t64`).
- **SoapySDR builds:** SoapySDR + driver modules as above.
- Prefer **`-compat`** artifacts if you see `GLIBC_2.39' not found`.

---

## 4. Installation — Windows

1. Download the **Standard** Windows installer
   (`sdroxide-<version>-windows-x86_64.msi`) from
   [Releases](https://github.com/epiHATR/sdroxide/releases), or the portable
   zip if you prefer not to install.
2. Run the MSI (or unpack the zip and run `sdroxide.exe`).
3. For USB SDRs (RTL-SDR, RX-888, …), bind the device to **WinUSB** with
   [Zadig](https://zadig.akeo.ie/).  
   RX-888 appears as two USB ids over its life cycle — bind **both**:
   `04b4:00f3` (FX3 bootloader) and `04b4:00f1` (after firmware upload).
4. SoapySDR builds: install
   [PothosSDR](https://downloads.myriadrf.org/builds/PothosSDR/) and enable
   “Add PothosSDR to the system PATH”.

Windows Defender / SmartScreen may warn on first run for an unsigned download —
choose **More info → Run anyway** if you trust the GitHub release asset.

---

## 5. Installation — macOS

1. Download the **Standard** Apple Silicon or Intel `.dmg`
   (`sdroxide-<version>-macos-aarch64.dmg` or `macos-x86_64`) from
   [Releases](https://github.com/epiHATR/sdroxide/releases).
2. Open the DMG and drag **sdroxide.app** to **Applications** (or
   `~/Applications`).
3. First launch: the release is **ad-hoc signed / not notarized**. Prefer:

```sh
xattr -dr com.apple.quarantine /Applications/sdroxide.app
open /Applications/sdroxide.app
```

If Gatekeeper still blocks (process stuck, “damaged”), install under a **new
name** or from a local build instead of re-dragging the Firefox/Chrome download
into `/Applications` — quarantine + provenance on that path can hang dyld. A
known-good approach:

```sh
# after clearing quarantine and ad-hoc signing locally
open ~/Applications/sdroxide.app
```

Or right-click → **Open** when macOS offers the unidentified-developer dialog.

4. SoapySDR builds: `brew install soapysdr` plus a driver (e.g. `soapyrtlsdr`).
5. USB receivers need no extra driver package on macOS.

---

## 6. First launch

```sh
sdroxide                 # desktop, restores last session
sdroxide --siggen        # no radio
sdroxide --server        # browser UI at http://127.0.0.1:4950
sdroxide --probe         # list devices
```

Embedded web client: every current release binary can serve the UI with
`--server` (no separate web zip required).

Config lives under the platform config directory (see
[[13. Configuration files]] / User Manual §13), e.g. on macOS
`~/Library/Application Support/org.sdroxide.sdroxide/`.

---

## 7. Next steps

- [[2. Basic operation]] — dial, filters, PTT, display strip  
- [[3. Digital modes]] — FT8 and the rest  
- [[15. Radio-specific notes]] — per-device quirks  
- [[14. Troubleshooting]] — when something will not open or decode  

Building from source: [CONTRIBUTING.md](https://github.com/epiHATR/sdroxide/blob/main/CONTRIBUTING.md).
