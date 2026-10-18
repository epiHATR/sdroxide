//! Auto TUNE: step the dial across the 400–406 MHz radiosonde band until a
//! decode locks, then park.
//!
//! The decode lane is only ~48 kHz wide, so covering six megahertz means many
//! short hops. Spectrum peaks (from [`sdroxide_rs41`]'s channel bank) decide
//! whether to dwell for a frame or skip; a successful decode parks the VFO.

use std::time::{Duration, Instant};

use sdroxide_types::{RS41_BAND_HI_HZ, RS41_BAND_LO_HZ, Rs41Sonde};

use crate::scanner::slice_centers;

/// Settle after each hardware / VFO move before reading peaks.
const SETTLE: Duration = Duration::from_millis(220);
/// No carrier hint: leave quickly.
const SKIP: Duration = Duration::from_millis(450);
/// Carrier hint: wait for a JSON decode.
const DWELL: Duration = Duration::from_millis(2_800);
/// After a lock goes quiet, resume hunting.
const QUIET: Duration = Duration::from_secs(45);

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Idle,
    Hunting { step: usize, since: Instant },
    Dwelling { step: usize, until: Instant },
    Locked,
}

/// Band hunter owned by the engine.
#[derive(Debug)]
pub struct SondeBandHunt {
    phase: Phase,
    steps: Vec<f64>,
    /// Operator / chip wants the hunt running.
    armed: bool,
    quiet_since: Option<Instant>,
}

impl Default for SondeBandHunt {
    fn default() -> Self {
        SondeBandHunt {
            phase: Phase::Idle,
            steps: Vec::new(),
            armed: false,
            quiet_since: None,
        }
    }
}

impl SondeBandHunt {
    /// Rebuild the step list for the current sonde lane width.
    pub fn set_lane_rate(&mut self, lane_hz: f64) {
        self.steps = band_steps(lane_hz);
    }

    pub fn scanning(&self) -> bool {
        matches!(self.phase, Phase::Hunting { .. } | Phase::Dwelling { .. })
    }

    pub fn scan_hz(&self) -> Option<f64> {
        match self.phase {
            Phase::Hunting { step, .. } | Phase::Dwelling { step, .. } => {
                self.steps.get(step).copied()
            }
            _ => None,
        }
    }

    pub fn armed(&self) -> bool {
        self.armed
    }

    /// Start (or restart) a hunt from the low end of the band.
    pub fn start(&mut self, lane_hz: f64) {
        self.set_lane_rate(lane_hz);
        self.armed = true;
        self.quiet_since = None;
        if self.steps.is_empty() {
            self.phase = Phase::Idle;
            return;
        }
        self.phase = Phase::Hunting { step: 0, since: Instant::now() };
    }

    /// Stop hunting; leave the dial where it is.
    pub fn stop(&mut self) {
        self.armed = false;
        self.phase = Phase::Idle;
        self.quiet_since = None;
    }

    /// Park after a decode: stay armed so a quiet timeout can re-hunt.
    pub fn mark_locked(&mut self) {
        self.armed = true;
        self.phase = Phase::Locked;
        self.quiet_since = Some(Instant::now());
    }

    /// Operator tuned by hand — cancel any in-progress hunt.
    pub fn stop_for_operator(&mut self) {
        if self.scanning() || matches!(self.phase, Phase::Locked) {
            self.stop();
        }
    }

    /// What the engine should do this tick.
    pub fn tick(&mut self, carrier_hint: bool, sondes: &[Rs41Sonde]) -> HuntAction {
        if !self.armed {
            return HuntAction::None;
        }
        if self.steps.is_empty() {
            return HuntAction::None;
        }

        // Fresh decode while hunting → park.
        if self.scanning()
            && let Some(hz) = newest_sonde_hz(sondes)
        {
            return HuntAction::Lock(hz);
        }

        match self.phase {
            Phase::Idle => HuntAction::None,
            Phase::Locked => {
                if sondes.is_empty() {
                    let quiet = self.quiet_since.get_or_insert_with(Instant::now);
                    if quiet.elapsed() >= QUIET {
                        self.phase = Phase::Hunting { step: 0, since: Instant::now() };
                        return HuntAction::Tune(self.steps[0]);
                    }
                } else {
                    self.quiet_since = Some(Instant::now());
                }
                HuntAction::None
            }
            Phase::Hunting { step, since } => {
                if since.elapsed() < SETTLE {
                    return HuntAction::None;
                }
                if carrier_hint {
                    self.phase = Phase::Dwelling { step, until: Instant::now() + DWELL };
                    HuntAction::None
                } else if since.elapsed() >= SETTLE + SKIP {
                    self.advance(step)
                } else {
                    HuntAction::None
                }
            }
            Phase::Dwelling { step, until } => {
                if Instant::now() >= until {
                    self.advance(step)
                } else {
                    HuntAction::None
                }
            }
        }
    }

    fn advance(&mut self, step: usize) -> HuntAction {
        let next = (step + 1) % self.steps.len();
        self.phase = Phase::Hunting { step: next, since: Instant::now() };
        HuntAction::Tune(self.steps[next])
    }
}

/// Engine response to one hunt tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HuntAction {
    None,
    /// Move the dial to this absolute Hz (a band step).
    Tune(f64),
    /// Park on a decoded sonde and stop scanning.
    Lock(f64),
}

/// Lane-sized centres covering 400–406 MHz.
pub fn band_steps(lane_hz: f64) -> Vec<f64> {
    let span = lane_hz.max(20_000.0);
    slice_centers(RS41_BAND_LO_HZ, RS41_BAND_HI_HZ, span)
}

fn newest_sonde_hz(sondes: &[Rs41Sonde]) -> Option<f64> {
    sondes
        .iter()
        .max_by_key(|s| s.last_at)
        .map(|s| s.frequency_hz)
        .filter(|hz| hz.is_finite() && *hz > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_steps_cover_the_meteorological_band() {
        let steps = band_steps(48_000.0);
        assert!(steps.len() > 10, "expected many lane-sized hops, got {}", steps.len());
        assert!(steps[0] >= RS41_BAND_LO_HZ - 1.0);
        assert!(*steps.last().unwrap() <= RS41_BAND_HI_HZ + 1.0);
        // Overlapping tiles — consecutive steps closer than one full lane.
        for w in steps.windows(2) {
            assert!(w[1] - w[0] < 48_000.0, "gap too large: {} → {}", w[0], w[1]);
        }
    }

    #[test]
    fn hunt_advances_without_a_carrier_and_locks_on_decode() {
        let mut h = SondeBandHunt::default();
        h.start(48_000.0);
        assert!(h.scanning());
        let first = h.scan_hz().unwrap();
        // Force settle + skip window.
        if let Phase::Hunting { step, .. } = h.phase {
            h.phase = Phase::Hunting {
                step,
                since: Instant::now() - SETTLE - SKIP,
            };
        }
        match h.tick(false, &[]) {
            HuntAction::Tune(hz) => assert_ne!(hz, first),
            other => panic!("expected Tune, got {other:?}"),
        }
        let sonde = Rs41Sonde::new("TEST".into(), 402_500_000.0, Default::default(), 1);
        // Mid-dwell with a decode.
        h.phase = Phase::Dwelling { step: 0, until: Instant::now() + DWELL };
        assert_eq!(h.tick(true, &[sonde]), HuntAction::Lock(402_500_000.0));
        h.mark_locked();
        assert!(!h.scanning());
        assert_eq!(h.phase, Phase::Locked);
        assert!(h.armed());
    }

    #[test]
    fn operator_stop_clears_hunting() {
        let mut h = SondeBandHunt::default();
        h.start(48_000.0);
        h.stop_for_operator();
        assert!(!h.scanning());
        assert!(!h.armed());
    }
}
