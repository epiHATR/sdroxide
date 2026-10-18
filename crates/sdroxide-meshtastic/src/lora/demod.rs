//! Soft LoRa LongFast demodulator (SF11 / 250 kHz / CR 4/5 / sync 0x2B).
//!
//! Written from Semtech SX1276 datasheet §4.1 and AN1200.22 (chirp modulation,
//! preamble / sync / SFD, explicit header, diagonal interleaving). No GPL LoRa.

use std::f32::consts::PI;
use std::sync::Arc;
use std::time::{Duration, Instant};

use num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use sdroxide_dsp::ComplexResampler;
use tracing::info;

use super::fec::{
    crc16_ccitt, decode_explicit_header, diag_deinterleave, hamming_decode, header_symbol_from_bin,
    nibbles_to_bytes, payload_symbol_from_bin, payload_symbols_after_header, whiten,
};

/// Meshtastic LongFast modem constants.
pub const SF: u8 = 11;
pub const BW_HZ: f64 = 250_000.0;
/// Internal demod rate: 2× BW so each chirp is exactly `2·2^SF` samples.
pub const DEM_RATE_HZ: f64 = 500_000.0;
pub const CR: u8 = 1; // 4/5
pub const SYNC_WORD: u8 = 0x2B;
pub const PREAMBLE_SYMS: usize = 16;

const N_CHIP: usize = 1 << SF; // 2048
/// Oversample factor at [`DEM_RATE_HZ`].
const OSF: usize = 2;
const SPS: usize = N_CHIP * OSF; // 4096
/// SFD is 2.25 downchirps.
const SFD_SAMPLES: usize = SPS * 2 + SPS / 4;

/// One decoded PHY payload (Meshtastic header + ciphertext) plus SNR estimate.
#[derive(Debug, Clone)]
pub struct PhyPacket {
    pub bytes: Vec<u8>,
    pub snr_db: f32,
    pub rssi_dbm: Option<f32>,
    /// Semtech payload CRC when the PHY checked it (`None` if unknown).
    pub crc_ok: Option<bool>,
}

/// Streaming LongFast demod: feed window-rate I/Q, drain packets.
pub struct LongFastDemod {
    in_rate_hz: f64,
    rs: Option<ComplexResampler>,
    rs_buf: Vec<Complex32>,
    fft: Arc<dyn Fft<f32>>,
    upchirp: Vec<Complex32>,
    buf: Vec<Complex32>,
    work: Vec<Complex32>,
    state: State,
    noise_pwr: f32,
    /// Rate-limit noisy hunt logs.
    last_hunt_log: Instant,
    /// Peak preamble run seen since last log.
    hunt_best_run: usize,
    hunt_best_snr: f32,
}

enum State {
    Hunt {
        run: usize,
        last_bin: Option<u32>,
    },
    /// Saw downchirps after preamble — finish SFD then header.
    Sfd {
        cfo_bin: u32,
        /// Samples of SFD already consumed toward [`SFD_SAMPLES`].
        got: usize,
        snr: f32,
    },
    Header {
        start: usize,
        cfo_bin: u32,
    },
    Payload {
        start: usize,
        cfo_bin: u32,
        payload_len: usize,
        has_crc: bool,
        cr: u8,
        symbols_needed: usize,
        symbols: Vec<u32>,
        /// Nibbles that arrived inside the 8-symbol explicit-header block.
        header_nibbles: Vec<u8>,
    },
}

impl LongFastDemod {
    pub fn new(in_rate_hz: f64) -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(SPS);
        let upchirp = make_upchirp(SPS);
        let rs = ComplexResampler::new(in_rate_hz, DEM_RATE_HZ);
        info!(
            in_rate_hz,
            dem_rate = DEM_RATE_HZ,
            sps = SPS,
            resample = rs.is_some(),
            "LongFast demod ready"
        );
        Self {
            in_rate_hz,
            rs,
            rs_buf: Vec::new(),
            fft,
            upchirp,
            buf: Vec::with_capacity(SPS * 64),
            work: vec![Complex32::new(0.0, 0.0); SPS],
            state: State::Hunt {
                run: 0,
                last_bin: None,
            },
            noise_pwr: 1e-6,
            last_hunt_log: Instant::now() - Duration::from_secs(10),
            hunt_best_run: 0,
            hunt_best_snr: 0.0,
        }
    }

    pub fn set_rate(&mut self, in_rate_hz: f64) {
        if (in_rate_hz - self.in_rate_hz).abs() < 1.0 {
            return;
        }
        *self = Self::new(in_rate_hz);
    }

    pub fn push(&mut self, iq: &[Complex32]) -> Vec<PhyPacket> {
        if iq.is_empty() {
            return Vec::new();
        }
        // Land everything on the clean 500 kS/s grid before dechirp.
        if let Some(rs) = self.rs.as_mut() {
            self.rs_buf.clear();
            rs.push(iq, &mut self.rs_buf);
            self.buf.extend_from_slice(&self.rs_buf);
        } else {
            self.buf.extend_from_slice(iq);
        }

        let cap = SPS * 256;
        if self.buf.len() > cap {
            let drop = self.buf.len() - cap;
            self.buf.drain(..drop);
            self.rewind_state(drop);
        }

        let mut out = Vec::new();
        loop {
            match self.step() {
                Step::NeedMore => break,
                Step::Packet(p) => out.push(p),
                Step::Continue => {}
            }
        }
        self.flush_hunt_log();
        out
    }

    fn flush_hunt_log(&mut self) {
        if self.last_hunt_log.elapsed() < Duration::from_millis(1500) {
            return;
        }
        if self.hunt_best_run == 0 && self.hunt_best_snr < 4.0 {
            return;
        }
        info!(
            best_preamble_run = self.hunt_best_run,
            best_snr = format!("{:.1}", self.hunt_best_snr),
            noise = format!("{:.2e}", self.noise_pwr),
            buf = self.buf.len(),
            "LongFast hunt"
        );
        self.hunt_best_run = 0;
        self.hunt_best_snr = 0.0;
        self.last_hunt_log = Instant::now();
    }

    fn rewind_state(&mut self, drop: usize) {
        match &mut self.state {
            State::Hunt { .. } | State::Sfd { .. } => {}
            State::Header { start, .. } | State::Payload { start, .. } => {
                if *start >= drop {
                    *start -= drop;
                } else {
                    self.state = State::Hunt {
                        run: 0,
                        last_bin: None,
                    };
                }
            }
        }
    }

    fn step(&mut self) -> Step {
        match self.state {
            State::Hunt { run, last_bin } => self.step_hunt(run, last_bin),
            State::Sfd { cfo_bin, got, snr } => self.step_sfd(cfo_bin, got, snr),
            State::Header { start, cfo_bin } => self.step_header(start, cfo_bin),
            State::Payload {
                start,
                cfo_bin,
                payload_len,
                has_crc,
                cr,
                symbols_needed,
                ref symbols,
                ref header_nibbles,
            } => {
                let symbols = symbols.clone();
                let header_nibbles = header_nibbles.clone();
                self.step_payload(
                    start,
                    cfo_bin,
                    payload_len,
                    has_crc,
                    cr,
                    symbols_needed,
                    symbols,
                    header_nibbles,
                )
            }
        }
    }

    fn step_hunt(&mut self, mut run: usize, mut last_bin: Option<u32>) -> Step {
        if self.buf.len() < SPS {
            self.state = State::Hunt { run, last_bin };
            return Step::NeedMore;
        }
        let (bin, peak, mean) = self.correlate_up(0);
        self.noise_pwr = 0.95 * self.noise_pwr + 0.05 * mean.max(1e-12);
        let snr = peak / self.noise_pwr.max(1e-12);
        if snr > self.hunt_best_snr {
            self.hunt_best_snr = snr;
        }

        let stable = last_bin
            .map(|lb| bin.abs_diff(lb) <= 4 || bin.abs_diff(lb) >= (N_CHIP as u32) - 4)
            .unwrap_or(true);
        let is_preamble = snr > 5.0 && stable;

        // Always peek for a downchirp once we have a real preamble run — SFD
        // can start while the up-correlate is still marginally above noise.
        let (_, dpeak0, _) = if run >= 8 && self.buf.len() >= SPS {
            self.correlate_down(0)
        } else {
            (0, 0.0, 0.0)
        };
        let dsnr0 = dpeak0 / self.noise_pwr.max(1e-12);
        let sfd_here = run >= 8 && dsnr0 > snr && dsnr0 > 4.0;

        if is_preamble && !sfd_here {
            run += 1;
            last_bin = Some(bin);
            if run > self.hunt_best_run {
                self.hunt_best_run = run;
            }
            self.buf.drain(..SPS);
            self.state = State::Hunt { run, last_bin };
            Step::Continue
        } else if run >= 8 {
            // Preamble (and sync upchirps) just ended — hunt the 2.25-symbol SFD.
            let cfo = last_bin.unwrap_or(bin);
            let mut best_dsnr = dsnr0;
            let mut best_off = 0usize;
            let try_offs = [0usize, SPS / 16, SPS / 8, SPS / 4, (3 * SPS) / 8, SPS / 2];
            for &off in &try_offs {
                if self.buf.len() < off + SPS {
                    break;
                }
                let (_, dpeak, _) = self.correlate_down(off);
                let dsnr = dpeak / self.noise_pwr.max(1e-12);
                if dsnr > best_dsnr {
                    best_dsnr = dsnr;
                    best_off = off;
                }
            }
            let commit = best_dsnr > 3.5;
            if commit {
                if best_off > 0 {
                    self.buf.drain(..best_off.min(self.buf.len()));
                }
                info!(
                    preamble_run = run,
                    cfo_bin = cfo,
                    up_snr = format!("{snr:.1}"),
                    down_snr = format!("{best_dsnr:.1}"),
                    sfd_off = best_off,
                    "LongFast SFD detected"
                );
                self.state = State::Sfd {
                    cfo_bin: cfo,
                    got: 0,
                    snr: best_dsnr.max(snr),
                };
                Step::Continue
            } else if is_preamble {
                // Gap in down detect but up still good — keep accumulating.
                run += 1;
                last_bin = Some(bin);
                self.buf.drain(..SPS);
                self.state = State::Hunt { run, last_bin };
                Step::Continue
            } else {
                info!(
                    preamble_run = run,
                    up_snr = format!("{snr:.1}"),
                    down_snr = format!("{best_dsnr:.1}"),
                    "LongFast preamble ended without SFD"
                );
                let slide = (SPS / 8).max(1);
                self.buf.drain(..slide.min(self.buf.len()));
                self.state = State::Hunt {
                    run: 0,
                    last_bin: None,
                };
                Step::Continue
            }
        } else {
            // Fine slide while searching. Tolerate one weak chirp mid-run.
            if run >= 3 && snr > 3.0 {
                self.buf.drain(..SPS.min(self.buf.len()));
                self.state = State::Hunt { run, last_bin };
                Step::Continue
            } else {
                let slide = (SPS / 8).max(1);
                self.buf.drain(..slide.min(self.buf.len()));
                self.state = State::Hunt {
                    run: 0,
                    last_bin: None,
                };
                Step::Continue
            }
        }
    }

    fn step_sfd(&mut self, cfo_bin: u32, got: usize, snr: f32) -> Step {
        // Need room to search header start across the whole SFD (2.25 sym)
        // plus a little slop — do not assume a fixed SFD length.
        let need = SFD_SAMPLES + SPS * 10;
        if self.buf.len() < need {
            self.state = State::Sfd { cfo_bin, got, snr };
            return Step::NeedMore;
        }
        let _ = got;
        let _ = SYNC_WORD;
        info!(
            cfo_bin,
            snr = format!("{snr:.1}"),
            "LongFast entering header"
        );
        self.state = State::Header {
            start: 0,
            cfo_bin,
        };
        Step::Continue
    }

    fn step_header(&mut self, start: usize, cfo_bin: u32) -> Step {
        let n_hdr = 8;
        // SFD is 2.25 downchirps; header begins somewhere from ~1.5×SPS to
        // ~2.75×SPS after SFD detect, depending on which downchirp we locked.
        let search_lo = SPS + SPS / 2;
        let search_hi = 3 * SPS;
        if self.buf.len() < start + search_hi + SPS * n_hdr {
            self.state = State::Header { start, cfo_bin };
            return Step::NeedMore;
        }

        let step = (SPS / 64).max(1);
        let mut found = None;
        let mut best_off = 0usize;
        let cfo_tries = [
            cfo_bin,
            cfo_bin.wrapping_add(1) & (N_CHIP as u32 - 1),
            cfo_bin.wrapping_sub(1) & (N_CHIP as u32 - 1),
            cfo_bin.wrapping_add(2) & (N_CHIP as u32 - 1),
            cfo_bin.wrapping_sub(2) & (N_CHIP as u32 - 1),
        ];
        let mut miss_syms = Vec::new();
        let mut miss_off = search_lo;
        'search: for &cfo in &cfo_tries {
            for off in (search_lo..=search_hi).step_by(step) {
                let mut syms = Vec::with_capacity(n_hdr);
                let mut min_peak = f32::MAX;
                for i in 0..n_hdr {
                    let (bin, peak, _) = self.correlate_up(start + off + i * SPS);
                    min_peak = min_peak.min(peak);
                    let raw = bin.wrapping_sub(cfo) & (N_CHIP as u32 - 1);
                    syms.push(header_symbol_from_bin(raw));
                }
                // Ignore windows where header chirps are weak (still in SFD).
                if min_peak < self.noise_pwr * 4.0 {
                    continue;
                }
                if miss_syms.is_empty() {
                    miss_syms = syms.clone();
                    miss_off = off;
                }
                if let Some(decoded) = decode_explicit_header(&syms, SF) {
                    found = Some((decoded, cfo));
                    best_off = off;
                    break 'search;
                }
            }
        }

        let Some(((hdr, header_nibbles), cfo_bin)) = found else {
            info!(
                syms = ?miss_syms,
                miss_off,
                cfo_bin,
                "LongFast header checksum miss — resync"
            );
            self.buf.drain(..(start + search_lo).min(self.buf.len()));
            self.state = State::Hunt {
                run: 0,
                last_bin: None,
            };
            return Step::Continue;
        };

        // LongFast @ 250 kHz / SF11: chirp < 16 ms → LDRO off for payload.
        let ldr = false;
        let symbols_needed =
            payload_symbols_after_header(hdr.payload_len, hdr.has_crc, SF, hdr.cr, ldr);
        info!(
            payload_len = hdr.payload_len,
            cr = hdr.cr,
            has_crc = hdr.has_crc,
            symbols_needed,
            header_nibbles = header_nibbles.len(),
            timing_off = best_off,
            "LongFast header OK"
        );
        self.buf.drain(..start + best_off + SPS * n_hdr);
        self.state = State::Payload {
            start: 0,
            cfo_bin,
            payload_len: hdr.payload_len,
            has_crc: hdr.has_crc,
            cr: hdr.cr,
            symbols_needed,
            symbols: Vec::with_capacity(symbols_needed),
            header_nibbles,
        };
        Step::Continue
    }

    fn step_payload(
        &mut self,
        start: usize,
        cfo_bin: u32,
        payload_len: usize,
        has_crc: bool,
        cr: u8,
        symbols_needed: usize,
        mut symbols: Vec<u32>,
        header_nibbles: Vec<u8>,
    ) -> Step {
        while symbols.len() < symbols_needed {
            let idx = start + symbols.len() * SPS;
            if self.buf.len() < idx + SPS {
                self.state = State::Payload {
                    start,
                    cfo_bin,
                    payload_len,
                    has_crc,
                    cr,
                    symbols_needed,
                    symbols,
                    header_nibbles,
                };
                return Step::NeedMore;
            }
            let (bin, _, _) = self.correlate_up(idx);
            let raw = bin.wrapping_sub(cfo_bin) & (N_CHIP as u32 - 1);
            symbols.push(payload_symbol_from_bin(raw, SF));
        }

        // Decode in blocks of (4+CR) symbols → nibbles → bytes → dewhiten.
        // Header block already contributed the first payload nibbles.
        let block = (cr as usize) + 4;
        let mut nibbles = header_nibbles;
        let mut i = 0;
        while i + block <= symbols.len() {
            let codewords = diag_deinterleave(&symbols[i..i + block], SF as usize);
            nibbles.extend(hamming_decode(&codewords, cr + 4));
            i += block;
        }
        let mut bytes = nibbles_to_bytes(&nibbles);
        let want = payload_len + if has_crc { 2 } else { 0 };
        if bytes.len() > want {
            bytes.truncate(want);
        }
        // TX: payload → CRC → whiten(payload+CRC). RX: dewhiten all, then CRC.
        if bytes.len() >= want.min(bytes.len()) && !bytes.is_empty() {
            let n = bytes.len().min(want);
            whiten(&mut bytes[..n]);
        }

        let mut crc_ok = true;
        if has_crc && bytes.len() >= payload_len + 2 {
            let got_le = u16::from_le_bytes([bytes[payload_len], bytes[payload_len + 1]]);
            let got_be = u16::from_be_bytes([bytes[payload_len], bytes[payload_len + 1]]);
            let expect = crc16_ccitt(&bytes[..payload_len]);
            crc_ok = got_le == expect || got_be == expect;
            bytes.truncate(payload_len);
        } else if bytes.len() > payload_len {
            bytes.truncate(payload_len);
        }

        let snr_db = 10.0 * (snr_lin(self.noise_pwr)).log10();
        let consume = start + symbols_needed * SPS;
        if self.buf.len() >= consume {
            self.buf.drain(..consume);
        } else {
            self.buf.clear();
        }
        self.state = State::Hunt {
            run: 0,
            last_bin: None,
        };

        // Mesh header is 16 bytes; accept CRC-miss during bring-up so AES text
        // can still surface if the poly/endian is slightly off.
        if bytes.len() >= 16 {
            if crc_ok {
                info!(
                    bytes = bytes.len(),
                    snr_db = format!("{snr_db:.1}"),
                    "LongFast payload OK"
                );
            } else {
                info!(
                    bytes = bytes.len(),
                    snr_db = format!("{snr_db:.1}"),
                    "LongFast payload CRC miss — still decoding"
                );
            }
            Step::Packet(PhyPacket {
                bytes,
                snr_db: snr_db.clamp(-20.0, 30.0),
                rssi_dbm: None,
                crc_ok: Some(crc_ok),
            })
        } else {
            info!(
                got = bytes.len(),
                payload_len,
                crc_ok,
                has_crc,
                "LongFast payload rejected"
            );
            Step::Continue
        }
    }

    fn correlate_up(&mut self, offset: usize) -> (u32, f32, f32) {
        self.correlate(offset, true)
    }

    fn correlate_down(&mut self, offset: usize) -> (u32, f32, f32) {
        self.correlate(offset, false)
    }

    fn correlate(&mut self, offset: usize, up: bool) -> (u32, f32, f32) {
        // Up: × conj(upchirp). Down: × upchirp (= conj(downchirp)).
        for i in 0..SPS {
            let r = self.upchirp[i];
            self.work[i] = if up {
                self.buf[offset + i] * r.conj()
            } else {
                self.buf[offset + i] * r
            };
        }
        self.fft.process(&mut self.work);
        let mut best_i = 0usize;
        let mut best = 0.0f32;
        let mut sum = 0.0f32;
        for (i, c) in self.work.iter().enumerate() {
            let p = c.norm_sqr();
            sum += p;
            if p > best {
                best = p;
                best_i = i;
            }
        }
        let mean = sum / SPS as f32;
        // OSF=2: fold FFT bin onto 0..2^SF-1.
        let bin = (best_i / OSF) as u32 % N_CHIP as u32;
        (bin, best, mean)
    }
}

enum Step {
    NeedMore,
    Continue,
    Packet(PhyPacket),
}

fn snr_lin(noise: f32) -> f32 {
    (1.0 / noise.max(1e-12)).max(1e-6)
}

fn make_upchirp(sps: usize) -> Vec<Complex32> {
    // Base upchirp over `sps` samples (OSF·2^SF): phase π·n·(n−N)/N.
    let n = sps as f32;
    let mut v = Vec::with_capacity(sps);
    for i in 0..sps {
        let ph = PI * (i as f32) * ((i as f32) - n) / n;
        v.push(Complex32::new(ph.cos(), ph.sin()));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sps_constants() {
        assert_eq!(SPS, 4096);
        assert_eq!(N_CHIP, 2048);
    }

    #[test]
    fn payload_symbol_count_grows_with_len() {
        let a = payload_symbols_after_header(16, true, 11, 1, false);
        let b = payload_symbols_after_header(48, true, 11, 1, false);
        assert!(b > a);
        assert!(a > 0);
    }

    #[test]
    fn demod_accepts_silence_without_panic() {
        let mut d = LongFastDemod::new(480_000.0);
        let iq = vec![Complex32::new(0.01, -0.01); 8192];
        let pkts = d.push(&iq);
        assert!(pkts.is_empty());
    }

    #[test]
    fn synthetic_upchirp_peaks_near_zero_bin() {
        let mut d = LongFastDemod::new(DEM_RATE_HZ);
        // One base upchirp at DEM_RATE — should correlate to bin ~0.
        let chirp = make_upchirp(SPS);
        d.buf = chirp;
        let (bin, peak, mean) = d.correlate_up(0);
        assert!(peak > mean * 10.0, "peak={peak} mean={mean}");
        assert!(bin <= 2 || bin >= (N_CHIP as u32) - 2, "bin={bin}");
    }
}
