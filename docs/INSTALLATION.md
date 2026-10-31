# Installation

How to install SDRoxide on Linux, Windows, and macOS from a
[GitHub Release](https://github.com/epiHATR/sdroxide/releases).

Start with [Getting Started](GETTING_STARTED.md) for prerequisites and which
build to pick (**Standard** vs **SoapySDR**). Asset naming details:
[`release/DOWNLOAD.md`](../release/DOWNLOAD.md).

---

## Linux

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

1. Download the **`-compat`** `.deb` unless you know you need the plain one.
   Use `-compat` on Ubuntu 22.04, Debian 12/13, Mint 21, Raspberry Pi OS
   bookworm, and anything else that reports `Depends: libc6 (>= 2.43)` (or a
   similar glibc error) against the non-compat package.
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

## Windows

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

## macOS

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
name** or from a local build instead of re-dragging the browser download into
`/Applications` — quarantine + provenance on that path can hang dyld. A
known-good approach:

```sh
# after clearing quarantine and ad-hoc signing locally
open ~/Applications/sdroxide.app
```

Or right-click → **Open** when macOS offers the unidentified-developer dialog.

4. SoapySDR builds: `brew install soapysdr` plus a driver (e.g. `soapyrtlsdr`).
5. USB receivers need no extra driver package on macOS.

---

## After install

See [Getting Started](GETTING_STARTED.md) for first-launch commands and next
reading in the [User Manual](https://github.com/epiHATR/sdroxide/wiki).
