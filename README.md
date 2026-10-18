# SDR Oxide

[![ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/W7J0271P9Q)

A PowerSDR/Thetis-style SDR transceiver in Rust: panadapter and waterfall,
dual VFOs, many digi modes (FT8, CW skimmer, SSTV, …), logbook, Winlink, solar
3D / satellites, and pluggable backends (RTL-SDR, RX-888, HPSDR, TCI, CAT,
SoapySDR, and more). One binary runs as a native desktop app, a remote UI, or
a WebSocket server that serves the same UI to a browser.

<img width="2560" height="1600" alt="Panadapter and waterfall" src="https://github.com/user-attachments/assets/d4b29901-3e84-4c49-a987-ffec795017f2" />

<img width="1682" height="1212" alt="Operating panel" src="https://github.com/user-attachments/assets/aa08f5d3-ec62-4d91-9dd0-13bde1b0ae43" />

<img width="1496" height="933" alt="Main window" src="https://github.com/user-attachments/assets/902a73ff-c8bf-43cd-9fc3-884d40ce4b04" />

## Docs

- **[User Manual](https://github.com/epiHATR/sdroxide/wiki)** — how to operate it
  (source: [`docs/USER_MANUAL.md`](docs/USER_MANUAL.md))
- [QO-100 quick-start](docs/qo100-quickstart.en.md)
  ([Türkçe](docs/qo100-quickstart.tr.md))
- [Download / which build](release/DOWNLOAD.md) — Standard vs SoapySDR, AppImage,
  `.deb`, `.dmg`, `.msi`
- [Contributing](CONTRIBUTING.md)

## Run

| Mode | Command |
| --- | --- |
| Native desktop | `sdroxide` |
| Server (browser UI) | `sdroxide --server` → `http://host:4950` |
| Desktop → remote server | `sdroxide --connect host:4950` |

Pre-built installs are on the
[Releases](https://github.com/epiHATR/sdroxide/releases) page. Prefer the
**Standard** (non-SoapySDR) build unless you need a Soapy-only device; details
in [release/DOWNLOAD.md](release/DOWNLOAD.md).

Linux USB receivers need udev rules. Packages install them; from a checkout:

```sh
sudo cp packaging/linux/60-sdroxide-*.rules /usr/lib/udev/rules.d/
sudo udevadm control --reload
```

Then unplug/replug the dongle. Per-radio notes are in the
[wiki](https://github.com/epiHATR/sdroxide/wiki/15.-Radio-specific-notes).

## Building

Needs [rustup](https://rustup.rs/) (Rust 1.85+, edition 2024) and a C toolchain.
Clone with submodules:

```sh
git clone --recurse-submodules https://github.com/epiHATR/sdroxide
cd sdroxide
```

```sh
# Debian / Ubuntu
sudo apt install build-essential pkg-config cmake autoconf automake libtool \
                 libclang-dev libasound2-dev libopus-dev
# macOS
brew install pkg-config cmake autoconf automake libtool opus
```

```sh
cargo build --release -p sdroxide
./target/release/sdroxide
```

Browser client (for `--server` with a web UI):

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk
(cd crates/sdroxide-web && trunk build --release)
# either serve from disk:
./target/release/sdroxide --server --web-root crates/sdroxide-web/dist
# or bake it into the binary (build the web client first):
cargo build --release -p sdroxide --features embed-web
```

Hardware-free smoke tests: `--siggen`, or `--file <raw CF32 IQ>`.

More build detail (SoapySDR, permissions, packaging) lives in
[CONTRIBUTING.md](CONTRIBUTING.md) and [release/DOWNLOAD.md](release/DOWNLOAD.md).

## License

GPL-3.0-or-later. CW decoding links [DeepCW](https://github.com/e04/deepcw-engine)
(**AGPL-3.0-only**): if you run `sdroxide --server` for other people over a
network, you must offer them the Corresponding Source. See
[CONTRIBUTING.md](CONTRIBUTING.md).

## Credits

- Upstream project: [dividebysandwich/sdroxide](https://github.com/dividebysandwich/sdroxide)
- Grey-line shade, IBP beacon schedule, meteor-shower calendar, and Kp history
  ideas adapted from [OpenHamClock](https://github.com/accius/openhamclock) (MIT)
- CW neural decoder: [DeepCW](https://github.com/e04/deepcw-engine) (AGPL-3.0)
- UI built with [egui](https://github.com/emilk/egui)
