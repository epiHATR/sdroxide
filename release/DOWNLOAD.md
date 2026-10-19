# Which build to download

Static help for GitHub release assets. Kept here (not in each release body) so
version notes stay limited to **What's new** and **Changes**.

Two builds are provided for every platform:

- **Standard** (no suffix) — built without SoapySDR, so it needs no
  libSoapySDR/PothosSDR at all. It still drives real SDR hardware:
  **RTL-SDR** dongles, **RX-888 / RX-888 Mk2** receivers and the
  **Airspy HF+** (Dual / Discovery / Ranger) are opened directly over
  USB by native pure-Rust drivers (the RX-888's FX3 firmware is
  bundled and uploaded automatically — no vendor driver package). Plus a CAT-controlled radio (Icom/Yaesu/Xiegu) over a
  serial port + USB sound card (**Settings → Audio/CAT**), and the
  **OpenHPSDR** and **TCI** network backends. This is the recommended
  download for most users, and the one packaged as an installer
  (Windows `.msi`, macOS `.dmg`).
- **SoapySDR** (`-soapysdr` suffix) — everything above *plus* a
  dynamic link against **SoapySDR**, for devices that have no native
  driver here (HackRF, Airspy R2/Mini, PlutoSDR, …). No Windows or
  macOS installer, but on Linux it comes as a `.deb` (package
  `sdroxide-soapysdr`) as well as a portable archive — take
  `sdroxide-<version>-linux-aarch64-compat-soapysdr.deb` for a
  Raspberry Pi, Orange Pi or other ARM board, and the `x86_64` one for
  a PC. Every `.deb` has the browser client built into the binary, so
  the server serves the web UI with nothing else installed.

On Linux both builds additionally come as a **`-compat`** artifact
(e.g. `linux-x86_64-compat`), compiled against an older glibc (2.35).
Take the `-compat` file if the regular one refuses to start with
``version `GLIBC_2.39' not found`` — which is what happens on
stable/LTS distributions such as Ubuntu 22.04, Linux Mint 21,
Debian 12 and Raspberry Pi OS (bookworm). The regular Linux builds
need glibc 2.39 or newer (Ubuntu 24.04 / Mint 22 / Debian 13); the
`-compat` builds and `.deb`s work on both old and new distributions.

## Just want to try it? Take the AppImage

`sdroxide-<version>-linux-x86_64-compat.AppImage` (and the `aarch64`
one for a Raspberry Pi or other ARM board) is one file: download it,
`chmod +x` it, and run it. Nothing is installed, nothing is unpacked,
and there is no package manager involved.

It is the **Standard** build, so no SoapySDR is needed — and none of
the native drivers go through it anyway: RTL-SDR, RX-888, HackRF,
Airspy, Airspy HF+, SDRplay, LimeSDR, PlutoSDR, HPSDR, TCI, SpyServer
and a CAT rig on a sound card are all compiled in. It is built against
glibc 2.35, so it runs on old and new distributions alike.

Two things it cannot do for you, because no single file can:

- **USB receivers still need the udev rule.** The rule lives in
  `usr/share/doc/sdroxide` inside the AppImage; the tarball or the
  `.deb` beside it carries the same files if that is easier.
- **It needs FUSE to mount itself**, which every desktop distribution
  has (`fuse3`). Where it is missing — a minimal container, say — run
  it as `APPIMAGE_EXTRACT_AND_RUN=1 ./sdroxide-….AppImage` instead.

## Requirements (SoapySDR builds)

Install **SoapySDR** together with the driver module for your radio:

- **Windows**: install [PothosSDR](https://downloads.myriadrf.org/builds/PothosSDR/)
  and select "Add PothosSDR to the system PATH" during setup.
- **Linux**: the `-soapysdr.deb` declares its dependencies and
  recommends `soapysdr-module-all`. For the portable tarball install
  `libsoapysdr0.8`, `soapysdr-module-all`, and ALSA (`libasound2`).
- **macOS**: `brew install soapysdr` plus a driver such as
  `soapyrtlsdr` or `soapyhackrf`.

The **Standard** builds need only audio support (ALSA `libasound2` on
Linux; built in on Windows/macOS) — no SoapySDR. On macOS the app is
unsigned: right-click → Open on first launch, or clear quarantine with
`xattr -dr com.apple.quarantine sdroxide.app`.

## USB receivers (RTL-SDR, RX-888) — all builds

- **Linux**: the `.deb` installs the udev rules for you. Portable
  tarballs now ship them: `sudo cp 60-sdroxide-*.rules
  /usr/lib/udev/rules.d/ && sudo udevadm control --reload`, then
  replug. For the RX-888 **both** ids in the rule are required —
  `04b4:00f3` is the bare FX3 bootloader it enumerates as on every
  plug-in, `04b4:00f1` is the same device after sdroxide uploads
  firmware into it.
- **Windows**: bind the device to WinUSB with
  [Zadig](https://zadig.akeo.ie/) — for the RX-888, once for each of
  the two ids above.
- **macOS**: nothing to do.

The RX-888 FX3 firmware image is bundled in every binary and uploaded
to the receiver at open time. It is MIT licensed (© 2017-2020 Oscar
Steila IK1XPV and the rx888-firmware contributors) and its notice
ships as `LICENSE-rx888-firmware.txt` in every package.

## Meshtastic — all builds

Meshtastic LongFast decode needs **`lorarx`** from dxlAPRS on the
machine that runs the radio. It is **not** bundled (unlike the
radiosonde `*mod` helpers). Without it, **MESH** mode shows a
missing-helper warning and will not decode. Install OpenWebRX+'s
`dxlaprs-lora` package (or put a `lorarx` binary on `PATH` from
[oe5dxl.hamspirit.at](http://oe5dxl.hamspirit.at:8025/aprs/c/)), then
restart sdroxide. See `release/v1.6.12.md`.

## HD Radio — all builds

HD Radio (the digital sidecar of North American FM stations) is
decoded by **libnrsc5** from the [nrsc5](https://github.com/theori-io/nrsc5)
project, which is **not bundled**: sdroxide looks for it at startup
and uses it if it is there. Without it the **HD RADIO** mode is shown
greyed out, and hovering it says why. With a remote client, it is the
station's machine that needs it, not the client's.

- **Linux**: on Arch, the `nrsc5-git` AUR package. Elsewhere, build
  nrsc5 from source: `sudo make install` puts `libnrsc5.so` in
  `/usr/local/lib`, where sdroxide looks. A `libnrsc5.so` beside the
  executable works too.
- **Windows**: put `libnrsc5.dll` and the DLLs it depends on beside
  `sdroxide.exe`. nrsc5 publishes no Windows download; it builds with
  MSYS2.
- **macOS**: build and install nrsc5 from source, which puts
  `libnrsc5.dylib` in `/usr/local/lib`.

Restart sdroxide after installing it: the library is looked for once,
at startup.

## Spoken announcements

The radio can read itself out — frequency once the dial stops moving,
mode and band as one phrase, band-edge warnings, the SWR while you
tune up, and FT8/JS8 messages addressed to you — so it can be operated
without seeing it. Switch it on under **Settings → General → Voice
announcements**. The window is also exposed to NVDA, Orca and
VoiceOver.

The neural voice runs on your own machine: nothing is sent anywhere
and no speech service has to be installed. It ships in a `voices`
directory beside the program (inside `Contents/Resources` in the macOS
bundle, `/usr/share/sdroxide/voices` from the `.deb`), and is
**CC BY-NC-SA 4.0** rather than GPL — which is why it is a separate
data file that sdroxide reads at run time rather than part of the
binary. Its notice ships as `LICENSE-voice.txt`, and
`LICENSE-cmudict.txt` covers the pronunciation dictionary embedded in
the binary (BSD-2-Clause). Extra Piper voices can be dropped into
`speech_voices/` in the config directory and picked in Settings.

## CW decoding

CW is decoded by [DeepCW](https://github.com/e04/deepcw-engine), a
neural model embedded in every binary. It reads the spectrogram
directly instead of fitting a timing model to the envelope, which is
why it keeps copying well below the point where a classic decoder
gives up, and why it copies hand-sent CW whose timing no fit would
accept. The band skimmer uses the same model on every signal it finds.

The model and its reference implementation are **AGPL-3.0-only**, a
stronger copyleft than sdroxide's GPL, and it is linked into the
binary rather than read as a data file — so it covers the release as a
whole. In particular AGPL section 13 applies: **if you run
`sdroxide --server` and other people use that instance over a
network, you must offer them the Corresponding Source.** The notice
ships as `LICENSE-deepcw.txt` in every package.

Every binary embeds the web client: run `sdroxide --server` and open
`http://<host>:4950` in a browser. `sdroxide-web-*.zip` contains the
standalone web assets for use with `--web-root`.
