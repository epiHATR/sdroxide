//! The SAT window: pick a satellite, lock on, and operate through it.
//!
//! The lock itself lives in the engine — this window only *asks* for one
//! ([`Command::SetSatLock`]) and displays the [`sdroxide_types::SatTrackStatus`]
//! stream that comes back: look angles, range, and the Doppler corrections as
//! actually applied. That split is why a lock keeps correcting with this
//! window closed, and why a remote client sees the same numbers.
//!
//! The picker is built from the same element sets the 3D view tracks: the
//! operator's pasted TLEs and the cached subscription listings (native; the
//! browser gets the pasted sets and the curated names, and the engine — which
//! has the caches — resolves the elements when it locks). Pass prediction for
//! the list is memoized one satellite per frame: a search over 48 hours is
//! cheap enough to do once, and far too dear to do ninety times per repaint.

use eframe::egui::{self, RichText};
use sdroxide_types::{Command, Mode, SatLockConfig, SatUplink};

use crate::app::SdroxideApp;
use crate::theme;
use crate::theme::ThemedScroll;

/// Everything the window remembers between frames.
/// Which page of the SATELLITE window is showing.
///
/// QO-100 lives here rather than in a window of its own because it *is* a
/// satellite — a geostationary one with a transponder, which is the only kind
/// this program can work through without Doppler. Its calibration simply
/// arrived first, before there was a satellite window to put it in.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::app) enum SatTab {
    #[default]
    Satellites,
    /// Satellites the operator marked with WATCH — same picker as Satellites,
    /// filtered to the persisted watch list.
    Watching,
    /// Every upcoming pass of every workable satellite, in time order (#623).
    Passes,
    Qo100,
}

pub(in crate::app) struct SatWinState {
    pub search: String,
    /// Selection on the SATELLITES tab — its own, so a pick there does not
    /// drive the Watching timeline and vice versa.
    pub selected: Option<u64>,
    /// Selection on the WATCHING tab.
    pub watch_selected: Option<u64>,
    /// Force-centre the SATELLITES list on the selected row next draw — set
    /// when a pick arrives from PASSES (or the 3D lock). The list also keeps
    /// the selection on-screen whenever it has drifted out of the viewport
    /// (e.g. another bird's lock pane ate the height).
    scroll_selected_into_view: bool,
    /// Explicit vertical offset for the Satellites / Watching list — applied
    /// once via [`ScrollArea::vertical_scroll_offset`]. Arrow keys use this
    /// because `scroll_to_me` does not reliably bring rows below the fold into
    /// view in this fixed-height list.
    list_scroll_y: Option<f32>,
    /// Same for the PASSES table.
    passes_scroll_y: Option<f32>,
    /// Which of the selected satellite's published links is chosen.
    pub link_idx: usize,
    /// The lock we last asked the engine for — what the DOPPLER/ROTATOR
    /// toggles re-send modified. `None` when the running lock came from
    /// another client, in which case the toggles are not shown.
    pub sent: Option<SatLockConfig>,
    list: Option<SatList>,
    /// The sampled arcs behind the two pass diagrams — the locked bird's and
    /// the selected one's. Two slots because both are on screen at once when
    /// the operator is eyeing up the next pass during the current one, and a
    /// single slot would rebuild itself twice a frame.
    lock_curve: Option<PassCurve>,
    pick_curve: Option<PassCurve>,
    /// Multi-pass elevation timeline for the Watching tab's selected satellite.
    watch_timeline: Option<WatchTimeline>,
    /// Horizon for that timeline: 24 or 48 hours.
    watch_span_h: u32,
    /// Peak the operator clicked on the timeline (`max_el_unix`), keeping its
    /// detail tooltip open until they click elsewhere or another peak.
    watch_peak_pin: Option<i64>,
    passes: PassTable,
    /// SATELLITES list column sort (click a heading).
    list_sort: ListSort,
    list_sort_asc: bool,
    /// Arrow keys step this list only after a click inside the SATELLITE
    /// panel. Otherwise ←/→ keep their dial / filter-edge bindings (and a
    /// docked chip must not steal them just by being on screen).
    pub(in crate::app) keys_capture: bool,
    /// Centered confirm before removing a bird from the Watching list.
    pub(in crate::app) confirm_unwatch: Option<(u64, String)>,
}

impl Default for SatWinState {
    fn default() -> Self {
        Self {
            search: String::new(),
            selected: None,
            watch_selected: None,
            scroll_selected_into_view: false,
            list_scroll_y: None,
            passes_scroll_y: None,
            link_idx: 0,
            sent: None,
            list: None,
            lock_curve: None,
            pick_curve: None,
            watch_timeline: None,
            watch_span_h: 24,
            watch_peak_pin: None,
            passes: PassTable::default(),
            // Name is stable while pass memos fill one-per-frame. Pass sort
            // is operator-chosen — applying it at startup makes the list jump
            // until every memo is ready.
            list_sort: ListSort::Name,
            list_sort_asc: true,
            keys_capture: false,
            confirm_unwatch: None,
        }
    }
}

/// Drop search / column sort so a fresh list does not reshuffle as pass memos
/// fill. Watching (in `SatConfig`) and the live lock (`sent` / engine track)
/// are left alone.
fn sat_reset_list_chrome(win: &mut SatWinState) {
    win.search.clear();
    win.list_sort = ListSort::Name;
    win.list_sort_asc = true;
    win.list_scroll_y = None;
    win.scroll_selected_into_view = false;
}

impl SatWinState {
    /// Drop the cached catalogue so the next draw rebuilds from a new TLE listing.
    pub(in crate::app) fn invalidate_list(&mut self) {
        self.list = None;
    }
}

/// Column the SATELLITES list is ordered by.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum ListSort {
    /// Alphabetical — the load default; does not move as pass memos arrive.
    #[default]
    Name,
    Norad,
    /// Up-now first, then next AOS — chosen by clicking the PASS heading.
    Pass,
}

/// Column the PASSES table is ordered by.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum PassSort {
    Name,
    #[default]
    Aos,
    /// Countdown / "up now" — same key as AOS.
    When,
    Length,
    Peak,
    Az,
}

/// The PASSES page's table, filled a few satellites per frame so a long list
/// never stalls the window and the browser build needs no thread.
struct PassTable {
    /// What it was computed for: the list (by its build time), the QTH, the
    /// window and the filters. Any change starts it over.
    key: Option<(i64, (i64, i64), u32, i32, bool)>,
    /// When it was started, which is also where the search window opens.
    from_unix: i64,
    /// The next satellite of the list to search.
    next: usize,
    rows: Vec<PassRow>,
    span_h: u32,
    min_el: f64,
    all: bool,
    sort: PassSort,
    /// `true` = ascending (AOS soonest-first is the default).
    sort_asc: bool,
}

impl Default for PassTable {
    fn default() -> Self {
        Self {
            key: None,
            from_unix: 0,
            next: 0,
            rows: Vec::new(),
            span_h: 0,
            min_el: 0.0,
            all: false,
            sort: PassSort::Aos,
            sort_asc: true,
        }
    }
}

struct PassRow {
    norad_id: u64,
    name: String,
    pass: sdroxide_solar::satellites::Pass,
}

/// How long a pass table is trusted: elements and the clock barely move the
/// passes within an hour, and the ones already over are dropped as they end.
const PASS_TABLE_FRESH_S: i64 = 3600;

/// The parsed satellite list — kept across tab changes and only rebuilt when
/// the *catalogue* changes (pasted TLEs / subscriptions), not when Watching
/// toggles or the panel is reopened.
struct SatList {
    cfg: std::sync::Arc<sdroxide_types::SatConfig>,
    /// Fingerprint of catalogue sources (excludes `watching`).
    source_key: u64,
    built_unix: i64,
    entries: Vec<SatEntry>,
}

struct SatEntry {
    norad_id: u64,
    name: String,
    /// The element lines, handed to the engine with the lock so it needs no
    /// lookup of its own. `None` for a curated name whose elements this
    /// client does not hold — the engine still resolves those from its side.
    tle: Option<(String, String)>,
    sat: Option<sdroxide_solar::Satellite>,
    /// Next-pass memo: when it was computed and what it said.
    pass: Option<(i64, PassMemo)>,
}

#[derive(Clone)]
enum PassMemo {
    Pass(sdroxide_types::SatPass),
    Always,
    Never,
}

/// How long a *pass memo* is trusted before recomputing that one row.
/// The catalogue itself is not aged out — see [`sat_ensure_list`].
const MEMO_FRESH_S: i64 = 600;

/// Catalogue identity: pasted TLEs + live subscription URLs/`only` filters +
/// a fingerprint of the amalgamated listing (remote/wasm fill). Watching is
/// deliberately omitted so a WATCH chip does not rebuild the list.
fn sat_list_source_key(cfg: &sdroxide_types::SatConfig, listing: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    cfg.tle_text().hash(&mut h);
    for sub in cfg.live_subs() {
        sub.url.hash(&mut h);
        sub.only.hash(&mut h);
    }
    listing.len().hash(&mut h);
    listing.as_bytes().get(..64).hash(&mut h);
    listing.as_bytes().get(listing.len().saturating_sub(64)..).hash(&mut h);
    h.finish()
}

/// Keep / refresh the cached catalogue. No-op when the source key matches —
/// tab switches and watch-list edits must not wipe rows or pass memos.
fn sat_ensure_list(
    win: &mut SatWinState,
    cfg: &std::sync::Arc<sdroxide_types::SatConfig>,
    listing: &str,
    now: i64,
) {
    let key = sat_list_source_key(cfg, listing);
    if let Some(list) = win.list.as_mut() {
        if list.source_key == key {
            // Keep the Arc current (watching / freqs may have changed) without
            // rebuilding entries.
            list.cfg = std::sync::Arc::clone(cfg);
            return;
        }
    }
    let prev = win.list.take();
    let mut new = build_list(cfg, listing, now, key);
    if let Some(old) = prev.as_ref() {
        for e in &mut new.entries {
            if let Some(prev_e) = old.entries.iter().find(|o| o.norad_id == e.norad_id) {
                e.pass = prev_e.pass.clone();
            }
        }
    }
    win.list = Some(new);
    // Fresh catalogue (first open or TLE/sub change) — clear search/sort.
    // Watching and the live lock are not touched here.
    sat_reset_list_chrome(win);
}

impl SatEntry {
    fn matches(&self, q: &str) -> bool {
        q.is_empty()
            || self.name.to_ascii_lowercase().contains(&q.to_ascii_lowercase())
            || self.norad_id.to_string().contains(q)
    }
}

fn build_list(
    cfg: &std::sync::Arc<sdroxide_types::SatConfig>,
    listing: &str,
    now: i64,
    source_key: u64,
) -> SatList {
    let mut entries: Vec<SatEntry> = Vec::new();
    let add_text = |text: &str, entries: &mut Vec<SatEntry>| {
        for t in sdroxide_types::parse_tle_block(text) {
            let Some(id) = t.norad_id() else { continue };
            if entries.iter().any(|e| e.norad_id == id) {
                continue;
            }
            let Some(sat) = sdroxide_solar::satellites::parse_pasted_tles(&format!(
                "{}\n{}\n{}",
                t.name, t.line1, t.line2
            ))
            .pop() else {
                continue;
            };
            entries.push(SatEntry {
                norad_id: id,
                name: sat.name.clone(),
                tle: Some((t.line1.clone(), t.line2.clone())),
                sat: Some(sat),
                pass: None,
            });
        }
    };
    add_text(&cfg.tle_text(), &mut entries);
    // Remote / wasm: engine-pushed amalgamated listing (pasted + subs). Native
    // can also use it when present; otherwise fall back to the disk cache.
    if !listing.is_empty() {
        add_text(listing, &mut entries);
    } else {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let cache = sdroxide_solar::cache::Cache::open();
            for sub in cfg.live_subs() {
                if let Some(text) = sdroxide_solar::tlesub::cached_text(&cache, sub) {
                    add_text(&text, &mut entries);
                }
            }
        }
    }
    // The curated names always appear, elements or not: a row that says "no
    // elements" tells the operator what to fix, an absent row says nothing.
    for (id, name) in sdroxide_solar::satellites::POPULAR {
        if !entries.iter().any(|e| e.norad_id == *id) {
            entries.push(SatEntry {
                norad_id: *id,
                name: (*name).to_string(),
                tle: None,
                sat: None,
                pass: None,
            });
        }
    }
    SatList { cfg: std::sync::Arc::clone(cfg), source_key, built_unix: now, entries }
}

/// How many points the elevation arc is sampled at. Far too many sgp4 runs to
/// do per repaint, which is the whole reason a [`PassCurve`] is kept.
const CURVE_SAMPLES: usize = 121;
/// The lead-in and run-out the diagram shows either side of the pass: a minute
/// of each, so the arc is seen crossing the horizon rather than starting on it.
const PASS_MARGIN_S: i64 = 60;
/// Height of the diagram under the lock pane, and of the shorter one the
/// picker draws for whatever satellite is selected.
const DIAGRAM_H: f32 = 92.0;
const DIAGRAM_H_SMALL: f32 = 76.0;
/// Height of the Watching-tab multi-pass timeline.
const TIMELINE_H: f32 = 110.0;
/// Samples along each pass arc on the timeline (cheaper than a full diagram).
const TIMELINE_SAMPLES: usize = 41;
/// How long a built timeline is trusted before the next search.
const TIMELINE_FRESH_S: i64 = 600;
/// Horizons offered on the Watching timeline chips.
const WATCH_SPANS_H: &[u32] = &[1, 3, 6, 12, 24, 48];

fn normalize_watch_span_h(h: u32) -> u32 {
    if WATCH_SPANS_H.contains(&h) { h } else { 24 }
}

/// One pass sampled into an elevation arc: the pass diagram's data, held
/// across frames.
struct PassCurve {
    norad_id: u64,
    /// Which pass this is, identified by its *set* time. A search that starts
    /// mid-pass reports the search start as the rise — so `rise_unix` creeps
    /// forward with the clock every time the engine recomputes and cannot key
    /// a memo. The set is a real horizon crossing and stays put.
    key_set_unix: i64,
    /// The window drawn: the true AOS less [`PASS_MARGIN_S`], to LOS plus it.
    t0: i64,
    t1: i64,
    rise_unix: i64,
    set_unix: i64,
    /// Elevation and azimuth in degrees at [`CURVE_SAMPLES`] evenly spaced
    /// instants from `t0` to `t1`.
    pts: Vec<(f32, f32)>,
}

impl PassCurve {
    fn build(
        norad_id: u64,
        sat: &sdroxide_solar::Satellite,
        lat: f64,
        lon: f64,
        p: &sdroxide_types::SatPass,
    ) -> Option<PassCurve> {
        let el_at = |t: i64| sat.at(t as f64).map(|s| s.az_el_from(lat, lon).1);
        let rise = true_rise(&el_at, p.rise_unix);
        let (t0, t1) = (rise - PASS_MARGIN_S, p.set_unix + PASS_MARGIN_S);
        if t1 - t0 < 2 {
            return None;
        }
        let span = (t1 - t0) as f64;
        let mut pts = Vec::with_capacity(CURVE_SAMPLES);
        for i in 0..CURVE_SAMPLES {
            let t = t0 as f64 + span * i as f64 / (CURVE_SAMPLES - 1) as f64;
            let (az, el) = sat.at(t)?.az_el_from(lat, lon);
            pts.push((el as f32, az as f32));
        }
        Some(PassCurve {
            norad_id,
            key_set_unix: p.set_unix,
            t0,
            t1,
            rise_unix: rise,
            set_unix: p.set_unix,
            pts,
        })
    }

    /// Where an instant falls across the window, 0 at `t0` and 1 at `t1`.
    fn frac(&self, unix: f64) -> f64 {
        (unix - self.t0 as f64) / (self.t1 - self.t0).max(1) as f64
    }

    /// Elevation and azimuth at an instant inside the window, interpolated
    /// between samples so the marker sits *on* the drawn arc rather than near
    /// it. `None` outside the window — a pass still hours away has no present
    /// position on it.
    fn sample(&self, unix: f64) -> Option<(f32, f32)> {
        let f = self.frac(unix);
        if !(0.0..=1.0).contains(&f) || self.pts.len() < 2 {
            return None;
        }
        let x = f * (self.pts.len() - 1) as f64;
        let i = (x.floor() as usize).min(self.pts.len() - 2);
        let k = (x - i as f64) as f32;
        let (a, b) = (self.pts[i], self.pts[i + 1]);
        // Azimuth wraps: interpolate the short way round, or a pass crossing
        // north swings the compass letter through south on the way.
        let mut daz = b.1 - a.1;
        if daz > 180.0 {
            daz -= 360.0;
        } else if daz < -180.0 {
            daz += 360.0;
        }
        Some((a.0 + (b.0 - a.0) * k, (a.1 + daz * k).rem_euclid(360.0)))
    }

    /// The highest sample: its index, elevation and azimuth. Taken from the
    /// samples rather than the pass summary so the label and the drawn apex
    /// cannot disagree.
    fn apex(&self) -> (usize, f32, f32) {
        let mut best = (0usize, f32::MIN, 0.0);
        for (i, &(el, az)) in self.pts.iter().enumerate() {
            if el > best.1 {
                best = (i, el, az);
            }
        }
        best
    }
}

/// The horizon crossing that really began the pass.
///
/// A search that starts mid-pass reports its own start time as the rise — the
/// satellite was already up when it looked — so a diagram drawn from that
/// would begin part-way up the arc and, worse, shift leftwards every time the
/// engine recomputed. Walk back to where the elevation last crossed zero and
/// bisect it to the second. Returns `reported` untouched in the normal case,
/// where it *is* the crossing, for the cost of one propagation.
fn true_rise(el_at: &impl Fn(i64) -> Option<f64>, reported: i64) -> i64 {
    if el_at(reported).unwrap_or(-90.0) <= 0.1 {
        return reported;
    }
    /// Coarse step back, then bisect. A pass never turns over inside this.
    const STEP: i64 = 15;
    /// An hour covers any low-Earth pass several times; a bird that stays up
    /// longer than that is drawn from wherever the search picked it up.
    const LIMIT: i64 = 3600;
    let mut up = reported;
    while reported - up < LIMIT {
        let below = up - STEP;
        if el_at(below).unwrap_or(-90.0) <= 0.0 {
            let (mut lo, mut hi) = (below, up);
            while hi - lo > 1 {
                let mid = lo + (hi - lo) / 2;
                if el_at(mid).unwrap_or(-90.0) > 0.0 { hi = mid } else { lo = mid }
            }
            return hi;
        }
        up = below;
    }
    up
}

/// Bring `slot` up to date for this satellite and pass and hand back the arc
/// to draw. `None` when the element set will not propagate that far.
fn curve_for<'a>(
    slot: &'a mut Option<PassCurve>,
    norad_id: u64,
    sat: &sdroxide_solar::Satellite,
    lat: f64,
    lon: f64,
    p: &sdroxide_types::SatPass,
) -> Option<&'a PassCurve> {
    let fresh =
        slot.as_ref().is_some_and(|c| c.norad_id == norad_id && c.key_set_unix == p.set_unix);
    if !fresh {
        *slot = PassCurve::build(norad_id, sat, lat, lon, p);
    }
    slot.as_ref()
}

/// One pass on the Watching timeline: rise/set, peak, and elevation samples
/// across the arc so the bump can be drawn on a shared 24/48 h axis.
struct TimelinePass {
    rise_unix: i64,
    set_unix: i64,
    max_el: f32,
    max_el_unix: i64,
    max_el_az: f32,
    /// Elevations at [`TIMELINE_SAMPLES`] evenly spaced instants from rise to set.
    pts: Vec<f32>,
}

enum TimelineKind {
    Passes(Vec<TimelinePass>),
    Always { el: f32 },
    Never,
}

/// Cached multi-pass elevation timeline for one satellite over a fixed horizon.
struct WatchTimeline {
    norad_id: u64,
    span_h: u32,
    /// QTH rounded to 0.01° — tiny GPS jitter must not thrash the cache.
    qth_key: (i32, i32),
    built_unix: i64,
    t0: i64,
    t1: i64,
    kind: TimelineKind,
}

impl WatchTimeline {
    fn build(
        norad_id: u64,
        sat: &sdroxide_solar::Satellite,
        lat: f64,
        lon: f64,
        span_h: u32,
        now: i64,
    ) -> WatchTimeline {
        let span_h = normalize_watch_span_h(span_h);
        let t0 = now;
        let t1 = now + span_h as i64 * 3600;
        let kind = match sat.next_passes(lat, lon, now as f64, (span_h as f64) * 3600.0, 48) {
            sdroxide_solar::PassSearch::AlwaysVisible { elevation, .. } => {
                TimelineKind::Always { el: elevation as f32 }
            }
            sdroxide_solar::PassSearch::NeverVisible => TimelineKind::Never,
            sdroxide_solar::PassSearch::Passes(passes) => {
                let mut out = Vec::with_capacity(passes.len());
                for p in passes {
                    let span = (p.set_unix - p.rise_unix).max(1) as f64;
                    let mut pts = Vec::with_capacity(TIMELINE_SAMPLES);
                    for i in 0..TIMELINE_SAMPLES {
                        let t =
                            p.rise_unix as f64 + span * i as f64 / (TIMELINE_SAMPLES - 1) as f64;
                        let el = sat.at(t).map(|s| s.az_el_from(lat, lon).1 as f32).unwrap_or(0.0);
                        pts.push(el);
                    }
                    out.push(TimelinePass {
                        rise_unix: p.rise_unix,
                        set_unix: p.set_unix,
                        max_el: p.max_el as f32,
                        max_el_unix: p.max_el_unix,
                        max_el_az: p.max_el_az as f32,
                        pts,
                    });
                }
                TimelineKind::Passes(out)
            }
        };
        WatchTimeline {
            norad_id,
            span_h,
            qth_key: ((lat * 100.0).round() as i32, (lon * 100.0).round() as i32),
            built_unix: now,
            t0,
            t1,
            kind,
        }
    }
}

/// Refresh `slot` when the satellite, span, QTH, or freshness window changes.
fn timeline_for<'a>(
    slot: &'a mut Option<WatchTimeline>,
    norad_id: u64,
    sat: &sdroxide_solar::Satellite,
    lat: f64,
    lon: f64,
    span_h: u32,
    now: i64,
) -> &'a WatchTimeline {
    let span_h = normalize_watch_span_h(span_h);
    let qth_key = ((lat * 100.0).round() as i32, (lon * 100.0).round() as i32);
    let fresh = slot.as_ref().is_some_and(|t| {
        t.norad_id == norad_id
            && t.span_h == span_h
            && t.qth_key == qth_key
            && now - t.built_unix < TIMELINE_FRESH_S
    });
    if !fresh {
        *slot = Some(WatchTimeline::build(norad_id, sat, lat, lon, span_h, now));
    }
    slot.as_ref().expect("just built")
}

/// `12:03` — the clock on the diagram's axis, UTC like every other time here.
fn hhmm(unix: i64) -> String {
    let (_, _, _, h, mi, _) = sdroxide_types::utc_ymd_hms(unix);
    format!("{h:02}:{mi:02}")
}

/// `13:08 UTC (14:08 local)` — a pass time in both zones. Pass times are UTC,
/// and read as local they are wrong by the operator's offset (#635), so the
/// local clock goes beside it wherever the two differ.
fn utc_and_local(unix: i64) -> String {
    let offset = crate::time::local_offset_at(unix);
    if offset == 0 {
        format!("{} UTC", hhmm(unix))
    } else {
        format!("{} UTC ({} local)", hhmm(unix), hhmm(unix + offset))
    }
}

/// `17 Oct 2026 13:08 UTC (14:08 local)` — date plus clocks for the Watching
/// timeline scrubber and peak tips, where a bare hour alone is ambiguous across
/// a multi-day window.
fn utc_date_and_local(unix: i64) -> String {
    let date = sdroxide_solar::timefmt::dmy(unix);
    let offset = crate::time::local_offset_at(unix);
    if offset == 0 {
        format!("{date} {} UTC", hhmm(unix))
    } else {
        let local = unix + offset;
        let local_date = sdroxide_solar::timefmt::dmy(local);
        if local_date == date {
            format!("{date} {} UTC ({} local)", hhmm(unix), hhmm(local))
        } else {
            // Near a day boundary the local calendar date can disagree.
            format!("{date} {} UTC ({local_date} {} local)", hhmm(unix), hhmm(local))
        }
    }
}

/// Paint the pass profile: elevation up the side, the clock and the compass
/// along the bottom, and a marker down the arc at the satellite's present
/// position.
///
/// A painted panel rather than a plot widget, because it has to read as part
/// of this window's chrome: night sky over a lit ground band, the arc glowing
/// out of the horizon and back into it, and a hot marker at *now*.
fn pass_diagram(ui: &mut egui::Ui, curve: &PassCurve, now: i64, height: f32) {
    use eframe::egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, epaint::Mesh};
    use egui::{pos2, vec2};

    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    if !ui.is_rect_visible(rect) || curve.pts.len() < 2 {
        return;
    }
    let p = ui.painter_at(rect);
    let font = FontId::proportional(8.5);
    let dim = theme::CYAN_DIM();
    let green = theme::GREEN();
    let ghost = |c: Color32, a: u8| Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a);

    // Three bands: the caption strip, the sky, and the ground the axis labels
    // are set into. The ground is deeper than one line of text because the
    // minute of run-in either side dips the arc `DIP` below the horizon, and
    // that dip must not land on the lettering.
    const HEAD_H: f32 = 13.0;
    const GROUND_H: f32 = 17.0;
    const DIP: f32 = 5.0;
    let plot = Rect::from_min_max(
        pos2(rect.left() + 1.0, rect.top() + HEAD_H),
        pos2(rect.right() - 1.0, rect.bottom() - 1.0),
    );
    let horizon_y = plot.bottom() - GROUND_H;
    let sky_top = plot.top() + 1.0;
    if plot.width() < 60.0 || horizon_y - sky_top < 16.0 {
        return; // nothing legible would fit
    }

    // An axis scaled to the pass. A 12° grazer flattened onto a 90° axis says
    // nothing at all, so the top of the scale is the next step above the peak
    // — and labelled, so the grazer is never mistaken for an overhead pass.
    let (apex_i, apex_el, apex_az) = curve.apex();
    let top_el = match apex_el {
        e if e <= 12.0 => 15.0f32,
        e if e <= 25.0 => 30.0,
        e if e <= 40.0 => 45.0,
        e if e <= 55.0 => 60.0,
        _ => 90.0,
    };
    let ey = |el: f32| {
        if el >= 0.0 {
            horizon_y - (el / top_el).clamp(0.0, 1.0) * (horizon_y - sky_top)
        } else {
            // The sub-horizon band has its own compressed scale — it exists to
            // show the crossing, not to be measured.
            horizon_y + (-el / (top_el / 9.0)).clamp(0.0, 1.0) * DIP
        }
    };
    let ex = |unix: f64| plot.left() + plot.width() * curve.frac(unix).clamp(0.0, 1.0) as f32;

    // --- ground and sky ---------------------------------------------------
    p.rect_filled(rect, 0.0, theme::INPUT_BG());
    let mut sky = Mesh::default();
    sky.colored_vertex(pos2(plot.left(), sky_top), Color32::TRANSPARENT);
    sky.colored_vertex(pos2(plot.right(), sky_top), Color32::TRANSPARENT);
    sky.colored_vertex(pos2(plot.right(), horizon_y), ghost(theme::CYAN(), 22));
    sky.colored_vertex(pos2(plot.left(), horizon_y), ghost(theme::CYAN(), 22));
    sky.add_triangle(0, 1, 2);
    sky.add_triangle(0, 2, 3);
    p.add(Shape::mesh(sky));
    p.rect_filled(
        Rect::from_min_max(pos2(plot.left(), horizon_y), pos2(plot.right(), plot.bottom())),
        0.0,
        theme::FILL(),
    );

    // --- grid -------------------------------------------------------------
    let grid = ghost(theme::LINE_LIT(), 150);
    for frac in [1.0f32, 2.0 / 3.0, 1.0 / 3.0] {
        let (y, el) = (ey(top_el * frac), top_el * frac);
        p.extend(Shape::dashed_line(
            &[pos2(plot.left(), y), pos2(plot.right(), y)],
            Stroke::new(1.0, grid),
            3.0,
            4.0,
        ));
        p.text(
            pos2(plot.left() + 4.0, y + 1.0),
            Align2::LEFT_TOP,
            format!("{el:.0}°"),
            font.clone(),
            dim,
        );
    }
    // Clock divisions, picked so the pass gets a handful of them whether it is
    // eight minutes long or eight hours.
    let span = (curve.t1 - curve.t0) as f64;
    let step = [60.0, 120.0, 300.0, 600.0, 1800.0, 3600.0, 10800.0]
        .into_iter()
        .find(|s| span / s <= 8.0)
        .unwrap_or(21600.0);
    let mut tick = (curve.t0 as f64 / step).ceil() * step;
    while tick <= curve.t1 as f64 {
        let x = ex(tick);
        p.extend(Shape::dashed_line(
            &[pos2(x, sky_top), pos2(x, horizon_y)],
            Stroke::new(1.0, ghost(theme::LINE_LIT(), 90)),
            2.0,
            5.0,
        ));
        tick += step;
    }

    // --- the arc ----------------------------------------------------------
    let xs: Vec<f32> = (0..curve.pts.len())
        .map(|i| plot.left() + plot.width() * i as f32 / (curve.pts.len() - 1) as f32)
        .collect();
    let line: Vec<Pos2> = curve.pts.iter().zip(&xs).map(|(&(el, _), &x)| pos2(x, ey(el))).collect();
    // Fill the sky under the arc, fading out towards the horizon.
    let mut under = Mesh::default();
    for i in 0..line.len() - 1 {
        let (a, b) = (line[i], line[i + 1]);
        let base = under.vertices.len() as u32;
        under.colored_vertex(pos2(a.x, a.y.min(horizon_y)), ghost(green, 64));
        under.colored_vertex(pos2(b.x, b.y.min(horizon_y)), ghost(green, 64));
        under.colored_vertex(pos2(b.x, horizon_y), ghost(green, 6));
        under.colored_vertex(pos2(a.x, horizon_y), ghost(green, 6));
        under.add_triangle(base, base + 1, base + 2);
        under.add_triangle(base, base + 2, base + 3);
    }
    p.add(Shape::mesh(under));
    p.add(Shape::line(line.clone(), Stroke::new(4.0, ghost(green, 34))));
    p.add(Shape::line(line, Stroke::new(1.6, green)));

    // The horizon itself, over the arc: the one line on the diagram that is a
    // hard boundary rather than a reading.
    p.line_segment(
        [pos2(plot.left(), horizon_y), pos2(plot.right(), horizon_y)],
        Stroke::new(1.2, theme::CYAN()),
    );

    // --- the apex ---------------------------------------------------------
    let apex_p = pos2(xs[apex_i], ey(apex_el));
    p.circle_filled(apex_p, 2.0, ghost(green, 200));
    let apex_label =
        format!("{apex_el:.0}° {}", sdroxide_solar::satellites::compass(apex_az as f64));
    let (align, dy) = if apex_p.y - sky_top < 12.0 {
        (Align2::CENTER_TOP, 4.0)
    } else {
        (Align2::CENTER_BOTTOM, -4.0)
    };
    p.text(pos2(apex_p.x, apex_p.y + dy), align, apex_label, font.clone(), theme::TEXT_STRONG());

    // --- horizon crossings, on the axis where they happen ------------------
    let compass = |unix: i64| {
        curve
            .sample(unix as f64)
            .map(|(_, az)| sdroxide_solar::satellites::compass(az as f64))
            .unwrap_or("")
    };
    for t in [curve.rise_unix, curve.set_unix] {
        let x = ex(t as f64);
        p.line_segment(
            [pos2(x, horizon_y - 3.0), pos2(x, horizon_y + 3.0)],
            Stroke::new(1.2, theme::CYAN()),
        );
    }
    let aos = format!("AOS {} {}", hhmm(curve.rise_unix), compass(curve.rise_unix));
    let los = format!("LOS {} {}", hhmm(curve.set_unix), compass(curve.set_unix));
    let baseline = plot.bottom() - 1.0;
    let aos_r =
        p.text(pos2(plot.left() + 5.0, baseline), Align2::LEFT_BOTTOM, &aos, font.clone(), dim);
    if plot.right() - 5.0 - crate::chrome::text_width(ui, &los, font.clone()) > aos_r.right() + 6.0
    {
        p.text(pos2(plot.right() - 5.0, baseline), Align2::RIGHT_BOTTOM, &los, font.clone(), dim);
    }

    // --- now ---------------------------------------------------------------
    if let Some((el, _)) = curve.sample(now as f64) {
        let pink = theme::PINK();
        let x = ex(now as f64);
        p.line_segment(
            [pos2(x, plot.top()), pos2(x, plot.bottom())],
            Stroke::new(1.0, ghost(pink, 110)),
        );
        p.line_segment([pos2(x, sky_top), pos2(x, horizon_y)], Stroke::new(1.4, pink));
        p.add(Shape::convex_polygon(
            vec![pos2(x - 3.5, plot.top()), pos2(x + 3.5, plot.top()), pos2(x, plot.top() + 5.0)],
            pink,
            Stroke::NONE,
        ));
        let dot = pos2(x, ey(el));
        p.circle_filled(dot, 4.5, ghost(pink, 70));
        p.circle_filled(dot, 2.2, pink);
        // The marker is the only thing on here that moves; keep it moving even
        // when nothing else asks for a frame.
        crate::repaint::after(ui.ctx(), std::time::Duration::from_secs(1));
    }

    // --- caption and frame -------------------------------------------------
    p.text(
        pos2(rect.left() + 7.0, rect.top() + 2.0),
        Align2::LEFT_TOP,
        "PASS PROFILE",
        font.clone(),
        dim,
    );
    p.text(
        pos2(rect.right() - 13.0, rect.top() + 2.0),
        Align2::RIGHT_TOP,
        format!(
            "{} above the horizon",
            sdroxide_solar::timefmt::age(curve.set_unix - curve.rise_unix)
        ),
        font,
        dim,
    );
    crate::chrome::paint_cut_border(&p, rect, theme::LINE_LIT(), theme::PANEL());
}

/// Peak hit target collected while painting the timeline, so the interactive
/// layer can sit on top of the drawn arcs.
struct TimelinePeakHit {
    apex: egui::Pos2,
    max_el_unix: i64,
    tip: String,
}

fn peak_tip(pass: &TimelinePass) -> String {
    format!(
        "Peak {:.0}° {} at {}\nAOS {}\nLOS {}",
        pass.max_el,
        sdroxide_solar::satellites::compass(pass.max_el_az as f64),
        utc_date_and_local(pass.max_el_unix),
        utc_date_and_local(pass.rise_unix),
        utc_date_and_local(pass.set_unix),
    )
}

/// Multi-pass elevation timeline over the next 24 or 48 hours: every rise/set
/// bump on one shared clock axis, with peak elevation and time at each apex.
///
/// Hovering the plot scrubs a UTC readout; clicking a peak pins its detail
/// tooltip (click again or elsewhere on the plot to clear).
fn pass_timeline(
    ui: &mut egui::Ui,
    tl: &WatchTimeline,
    now: i64,
    height: f32,
    pinned_peak: &mut Option<i64>,
) {
    use eframe::egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, epaint::Mesh};
    use egui::{pos2, vec2};

    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let p = ui.painter_at(rect);
    let font = FontId::proportional(8.5);
    let dim = theme::CYAN_DIM();
    let green = theme::GREEN();
    let ghost = |c: Color32, a: u8| Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a);

    const HEAD_H: f32 = 13.0;
    const GROUND_H: f32 = 17.0;
    let plot = Rect::from_min_max(
        pos2(rect.left() + 1.0, rect.top() + HEAD_H),
        pos2(rect.right() - 1.0, rect.bottom() - 1.0),
    );
    let horizon_y = plot.bottom() - GROUND_H;
    let sky_top = plot.top() + 1.0;
    if plot.width() < 80.0 || horizon_y - sky_top < 20.0 {
        return;
    }

    let span = (tl.t1 - tl.t0).max(1) as f64;
    let frac = |unix: f64| ((unix - tl.t0 as f64) / span).clamp(0.0, 1.0);
    let ex = |unix: f64| plot.left() + plot.width() * frac(unix) as f32;
    let unix_at = |x: f32| {
        let f = ((x - plot.left()) / plot.width()).clamp(0.0, 1.0) as f64;
        tl.t0 as f64 + span * f
    };

    let peak_el = match &tl.kind {
        TimelineKind::Passes(ps) => ps.iter().map(|p| p.max_el).fold(0.0f32, f32::max),
        TimelineKind::Always { el } => *el,
        TimelineKind::Never => 0.0,
    };
    let top_el = match peak_el {
        e if e <= 12.0 => 15.0f32,
        e if e <= 25.0 => 30.0,
        e if e <= 40.0 => 45.0,
        e if e <= 55.0 => 60.0,
        _ => 90.0,
    };
    let ey = |el: f32| horizon_y - (el.max(0.0) / top_el).clamp(0.0, 1.0) * (horizon_y - sky_top);

    // --- ground and sky ---------------------------------------------------
    p.rect_filled(rect, 0.0, theme::INPUT_BG());
    let mut sky = Mesh::default();
    sky.colored_vertex(pos2(plot.left(), sky_top), Color32::TRANSPARENT);
    sky.colored_vertex(pos2(plot.right(), sky_top), Color32::TRANSPARENT);
    sky.colored_vertex(pos2(plot.right(), horizon_y), ghost(theme::CYAN(), 22));
    sky.colored_vertex(pos2(plot.left(), horizon_y), ghost(theme::CYAN(), 22));
    sky.add_triangle(0, 1, 2);
    sky.add_triangle(0, 2, 3);
    p.add(Shape::mesh(sky));
    p.rect_filled(
        Rect::from_min_max(pos2(plot.left(), horizon_y), pos2(plot.right(), plot.bottom())),
        0.0,
        theme::FILL(),
    );

    // --- elevation grid ---------------------------------------------------
    let grid = ghost(theme::LINE_LIT(), 150);
    for frac_el in [1.0f32, 2.0 / 3.0, 1.0 / 3.0] {
        let (y, el) = (ey(top_el * frac_el), top_el * frac_el);
        p.extend(Shape::dashed_line(
            &[pos2(plot.left(), y), pos2(plot.right(), y)],
            Stroke::new(1.0, grid),
            3.0,
            4.0,
        ));
        p.text(
            pos2(plot.left() + 4.0, y + 1.0),
            Align2::LEFT_TOP,
            format!("{el:.0}°"),
            font.clone(),
            dim,
        );
    }

    // Time ticks across the horizon — denser for short windows.
    let step_s: i64 = match tl.span_h {
        1 => 10 * 60,
        3 => 30 * 60,
        6 => 3600,
        12 => 2 * 3600,
        24 => 3 * 3600,
        _ => 6 * 3600,
    };
    let mut tick = ((tl.t0 + step_s - 1) / step_s) * step_s;
    while tick <= tl.t1 {
        let x = ex(tick as f64);
        p.extend(Shape::dashed_line(
            &[pos2(x, sky_top), pos2(x, horizon_y)],
            Stroke::new(1.0, ghost(theme::LINE_LIT(), 90)),
            2.0,
            5.0,
        ));
        let label = if step_s < 3600 { hhmm(tick) } else { format!("{:02}", (tick / 3600) % 24) };
        p.text(pos2(x, plot.bottom() - 1.0), Align2::CENTER_BOTTOM, label, font.clone(), dim);
        tick += step_s;
    }

    p.line_segment(
        [pos2(plot.left(), horizon_y), pos2(plot.right(), horizon_y)],
        Stroke::new(1.2, theme::CYAN()),
    );

    let mut peak_hits: Vec<TimelinePeakHit> = Vec::new();
    let caption_right = match &tl.kind {
        TimelineKind::Never => {
            p.text(
                pos2(plot.center().x, (sky_top + horizon_y) * 0.5),
                Align2::CENTER_CENTER,
                "not visible from your QTH in this window",
                font.clone(),
                dim,
            );
            "0 passes".to_string()
        }
        TimelineKind::Always { el } => {
            let y = ey(*el);
            p.line_segment([pos2(plot.left(), y), pos2(plot.right(), y)], Stroke::new(1.6, green));
            p.text(
                pos2(plot.center().x, y - 4.0),
                Align2::CENTER_BOTTOM,
                format!("always visible · {el:.0}°"),
                font.clone(),
                theme::TEXT_STRONG(),
            );
            "always up".to_string()
        }
        TimelineKind::Passes(passes) => {
            // Drop a pin that no longer belongs to this window.
            if pinned_peak.is_some_and(|k| !passes.iter().any(|p| p.max_el_unix == k)) {
                *pinned_peak = None;
            }
            let mut label_xs: Vec<f32> = Vec::new();
            for pass in passes {
                if pass.pts.len() < 2 || pass.set_unix <= tl.t0 || pass.rise_unix >= tl.t1 {
                    continue;
                }
                let n = pass.pts.len();
                let span_p = (pass.set_unix - pass.rise_unix).max(1) as f64;
                let line: Vec<Pos2> = pass
                    .pts
                    .iter()
                    .enumerate()
                    .map(|(i, &el)| {
                        let t = pass.rise_unix as f64 + span_p * i as f64 / (n - 1) as f64;
                        pos2(ex(t), ey(el))
                    })
                    .collect();

                let mut under = Mesh::default();
                for i in 0..line.len() - 1 {
                    let (a, b) = (line[i], line[i + 1]);
                    let base = under.vertices.len() as u32;
                    under.colored_vertex(pos2(a.x, a.y.min(horizon_y)), ghost(green, 70));
                    under.colored_vertex(pos2(b.x, b.y.min(horizon_y)), ghost(green, 70));
                    under.colored_vertex(pos2(b.x, horizon_y), ghost(green, 8));
                    under.colored_vertex(pos2(a.x, horizon_y), ghost(green, 8));
                    under.add_triangle(base, base + 1, base + 2);
                    under.add_triangle(base, base + 2, base + 3);
                }
                p.add(Shape::mesh(under));
                p.add(Shape::line(line, Stroke::new(1.5, green)));

                let apex = pos2(ex(pass.max_el_unix as f64), ey(pass.max_el));
                let pinned = *pinned_peak == Some(pass.max_el_unix);
                p.circle_filled(apex, if pinned { 4.0 } else { 2.4 }, ghost(green, 220));
                if pinned {
                    p.circle_stroke(apex, 6.0, Stroke::new(1.2, theme::CYAN()));
                }
                peak_hits.push(TimelinePeakHit {
                    apex,
                    max_el_unix: pass.max_el_unix,
                    tip: peak_tip(pass),
                });

                // Peak marker + label (skip when it would collide with a neighbour).
                let crowded = label_xs.iter().any(|&x| (x - apex.x).abs() < 36.0);
                if !crowded && apex.x > plot.left() + 8.0 && apex.x < plot.right() - 8.0 {
                    let label = format!(
                        "{:.0}° {} {}",
                        pass.max_el,
                        sdroxide_solar::satellites::compass(pass.max_el_az as f64),
                        hhmm(pass.max_el_unix),
                    );
                    let (align, dy) = if apex.y - sky_top < 14.0 {
                        (Align2::CENTER_TOP, 5.0)
                    } else {
                        (Align2::CENTER_BOTTOM, -4.0)
                    };
                    p.text(
                        pos2(apex.x, apex.y + dy),
                        align,
                        label,
                        font.clone(),
                        theme::TEXT_STRONG(),
                    );
                    label_xs.push(apex.x);
                }
            }
            format!("{} pass{}", passes.len(), if passes.len() == 1 { "" } else { "es" })
        }
    };

    // --- now ---------------------------------------------------------------
    if (tl.t0..=tl.t1).contains(&now) {
        let pink = theme::PINK();
        let x = ex(now as f64);
        p.line_segment([pos2(x, sky_top), pos2(x, horizon_y)], Stroke::new(1.4, pink));
        p.add(Shape::convex_polygon(
            vec![pos2(x - 3.5, plot.top()), pos2(x + 3.5, plot.top()), pos2(x, plot.top() + 5.0)],
            pink,
            Stroke::NONE,
        ));
        crate::repaint::after(ui.ctx(), std::time::Duration::from_secs(1));
    }

    // --- hover scrubber: UTC at the pointer --------------------------------
    // Peak hit radius — shared by the scrubber exclusion and the interact layer.
    const PEAK_HIT: f32 = 14.0;
    let hover_pos = response.hover_pos();
    let near_peak = hover_pos
        .is_some_and(|pos| peak_hits.iter().any(|h| h.apex.distance(pos) <= PEAK_HIT * 0.6));
    if let Some(pos) = hover_pos {
        if !near_peak
            && plot.x_range().contains(pos.x)
            && pos.y >= sky_top
            && pos.y <= plot.bottom()
        {
            let unix = unix_at(pos.x).round() as i64;
            let cyan = theme::CYAN();
            p.line_segment(
                [pos2(pos.x, sky_top), pos2(pos.x, horizon_y)],
                Stroke::new(1.0, ghost(cyan, 180)),
            );
            // Scrubber readout: calendar date + UTC (and local when it differs).
            let text = utc_date_and_local(unix);
            let tw = crate::chrome::text_width(ui, &text, font.clone());
            let lx = (pos.x - tw * 0.5).clamp(plot.left() + 2.0, plot.right() - tw - 2.0);
            let label_r = Rect::from_min_size(pos2(lx - 3.0, sky_top + 2.0), vec2(tw + 6.0, 12.0));
            p.rect_filled(label_r, 0.0, theme::PANEL().gamma_multiply(0.92));
            p.text(
                pos2(lx + tw * 0.5, sky_top + 3.0),
                Align2::CENTER_TOP,
                text,
                font.clone(),
                theme::TEXT_STRONG(),
            );
        }
    }

    p.text(
        pos2(rect.left() + 7.0, rect.top() + 2.0),
        Align2::LEFT_TOP,
        format!("PASS TIMELINE · {} h", tl.span_h),
        font.clone(),
        dim,
    );
    p.text(
        pos2(rect.right() - 13.0, rect.top() + 2.0),
        Align2::RIGHT_TOP,
        caption_right,
        font.clone(),
        dim,
    );
    crate::chrome::paint_cut_border(&p, rect, theme::LINE_LIT(), theme::PANEL());

    // Peak hit targets — after the paint so they sit on top for hover/click.
    let mut peak_clicked = false;
    let mut pinned_resp: Option<(egui::Response, &str)> = None;
    for (i, hit) in peak_hits.iter().enumerate() {
        let hit_r = Rect::from_center_size(hit.apex, vec2(PEAK_HIT, PEAK_HIT));
        let r = ui.interact(hit_r, ui.id().with(("tl-peak", tl.norad_id, i)), Sense::click());
        r.clone().on_hover_text(&hit.tip);
        if r.hovered() {
            ui.painter().circle_stroke(hit.apex, 5.5, Stroke::new(1.2, theme::YELLOW()));
        }
        if r.clicked() {
            peak_clicked = true;
            if *pinned_peak == Some(hit.max_el_unix) {
                *pinned_peak = None;
            } else {
                *pinned_peak = Some(hit.max_el_unix);
            }
        }
        if *pinned_peak == Some(hit.max_el_unix) {
            pinned_resp = Some((r, hit.tip.as_str()));
        }
    }

    // Clicking empty plot clears a pinned peak tip.
    if response.clicked() && !peak_clicked {
        *pinned_peak = None;
    }

    // Dispose the pin when the pointer leaves the timeline or the window
    // loses focus — a stuck tip after looking away is worse than re-clicking.
    let over_timeline = ui.input(|i| i.pointer.hover_pos().is_some_and(|p| rect.contains(p)));
    let window_focused = ui.input(|i| i.focused);
    if pinned_peak.is_some() && (!over_timeline || !window_focused) {
        *pinned_peak = None;
        pinned_resp = None;
    }

    if let Some((r, tip)) = pinned_resp {
        r.show_tooltip_text(tip);
    }
}

/// The mode a link's published emission maps onto. Satellite SSB convention
/// is USB on the downlink, whatever the band. An SSTV downlink is FM on V/UHF,
/// and tuning it as plain NFM (or USB) would leave the picture undecoded (#622).
///
/// SatNOGS often publishes `mode: "SSTV"` with no "FM" in the string — that
/// still means FM SSTV on the bird, not SSB.
fn mode_for_link(l: &sdroxide_types::SatLink) -> Mode {
    let m = l.mode.to_ascii_uppercase();
    let label = l.label.to_ascii_uppercase();
    let sstv = m.contains("SSTV") || label.contains("SSTV");
    if sstv {
        // Rare HF/SSB SSTV on a satellite; everything else is the FM decoder.
        if m.contains("SSB") || m.contains("USB") || m.contains("LSB") {
            Mode::Sstv
        } else {
            Mode::SstvFm
        }
    } else if m.contains("FM") || m.contains("APT") {
        Mode::Nfm
    } else if m.contains("SSB") || m.contains("BPSK") || m.contains("GMSK") || m.contains("AX.25") {
        Mode::Usb
    } else if m.contains("CW") {
        Mode::Cw
    } else {
        Mode::Usb
    }
}

/// The downlink dial a link starts on: the current dial if it already sits
/// inside the passband (re-locking must not yank a tuned-in operator to the
/// centre), else the passband centre.
fn lock_downlink_hz(l: &sdroxide_types::SatLink, dial_hz: f64) -> f64 {
    match l.downlink {
        Some(pb) => {
            let (lo, hi) = (pb.lo_mhz * 1e6, pb.hi_mhz * 1e6);
            if (lo..=hi).contains(&dial_hz) { dial_hz } else { pb.centre_mhz() * 1e6 }
        }
        None => dial_hz,
    }
}

fn fmt_hz_signed(hz: f64) -> String {
    if hz.abs() >= 1000.0 { format!("{:+.2} kHz", hz / 1000.0) } else { format!("{hz:+.0} Hz") }
}

fn fmt_mhz(hz: f64) -> String {
    sdroxide_types::fmt_sat_mhz(hz / 1e6)
}

/// "in 12 min" / "in 3 h 05 min" — the countdown side of `timefmt::age`.

/// Paint truncated `text` left-aligned inside `rect` (no hit target of its own).
fn sat_paint_cell_text(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    text: RichText,
    fallback: egui::Color32,
) {
    use egui::{FontSelection, WidgetText};
    let galley = WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        (rect.width() - 4.0).max(1.0),
        FontSelection::Default,
    );
    let text_pos = egui::pos2(rect.left() + 2.0, rect.center().y - galley.size().y * 0.5);
    ui.painter().galley(text_pos, galley, fallback);
}

/// Rise compass on the left edge, set on the right — fills the FROM → TO column
/// instead of a single left-aligned `N → SE` string.
fn sat_paint_az_stretch(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    rise: &str,
    set: &str,
    color: egui::Color32,
) {
    use egui::{FontSelection, WidgetText};
    let pad = 2.0;
    let mid = "→";
    let rise_g = WidgetText::from(RichText::new(rise).size(10.0).color(color)).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        (rect.width() * 0.4).max(8.0),
        FontSelection::Default,
    );
    let set_g = WidgetText::from(RichText::new(set).size(10.0).color(color)).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        (rect.width() * 0.4).max(8.0),
        FontSelection::Default,
    );
    let arrow_g = WidgetText::from(RichText::new(mid).size(10.0).color(color)).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        rect.width(),
        FontSelection::Default,
    );
    let cy = |g: &egui::Galley| rect.center().y - g.size().y * 0.5;
    ui.painter().galley(egui::pos2(rect.left() + pad, cy(&rise_g)), rise_g, color);
    ui.painter().galley(egui::pos2(rect.right() - pad - set_g.size().x, cy(&set_g)), set_g, color);
    ui.painter().galley(
        egui::pos2(rect.center().x - arrow_g.size().x * 0.5, cy(&arrow_g)),
        arrow_g,
        color,
    );
}

/// PASSES table column widths: LENGTH / PEAK stay narrow so AOS and IN can
/// breathe; FROM → TO keeps a mid share for the stretch compass pair.
fn pass_col_widths(avail: f32, gap: f32) -> [f32; 6] {
    // name · aos · in · length · peak · az
    const W: [f32; 6] = [1.35, 1.85, 1.35, 0.50, 0.42, 1.15];
    let sum: f32 = W.iter().sum();
    let inner = (avail - gap * 5.0).max(0.0);
    let mut out = [0.0_f32; 6];
    for (i, w) in W.iter().enumerate() {
        out[i] = (inner * w / sum).max(if i == 3 || i == 4 { 28.0 } else { 40.0 });
    }
    out
}

/// Full-width row with per-column widths. `az_stretch` paints the last cell as
/// rise‥set stretched across its slot (passes FROM → TO).
fn sat_table_row_ws(
    ui: &mut egui::Ui,
    selected: bool,
    col_ws: &[f32],
    col_gap: f32,
    cells: &[RichText],
    az_stretch: Option<(&str, &str)>,
) -> egui::Response {
    let n = col_ws.len().max(1);
    let row_h = ui.spacing().interact_size.y.max(18.0);
    let row_w = (col_ws.iter().sum::<f32>() + col_gap * (n as f32 - 1.0)).max(ui.available_width());
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(row_w, row_h), egui::Sense::click());
    let visuals = ui.style().interact_selectable(&resp, selected);
    if selected {
        ui.painter().rect_filled(rect, visuals.corner_radius, theme::CYAN());
    } else if resp.hovered() || resp.highlighted() {
        ui.painter().rect_filled(rect, visuals.corner_radius, visuals.bg_fill);
    }
    let ink = if selected { theme::INK_ON_CYAN() } else { visuals.text_color() };
    let mut x = rect.left();
    for (i, &col_w) in col_ws.iter().enumerate() {
        let cell = egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(col_w, row_h));
        let last = i + 1 == n;
        if last {
            if let Some((rise, set)) = az_stretch {
                let color = if selected { theme::INK_ON_CYAN() } else { ink };
                sat_paint_az_stretch(ui, cell, rise, set, color);
                break;
            }
        }
        if let Some(text) = cells.get(i) {
            let text =
                if selected { text.clone().color(theme::INK_ON_CYAN()) } else { text.clone() };
            sat_paint_cell_text(ui, cell, text, visuals.text_color());
        }
        x += col_w + col_gap;
    }
    resp
}

/// ↑/↓ (and ←/→) while the SATELLITE panel has keyboard capture and no text
/// field is focused — steps the satellite list. Consumes the key so a
/// panadapter binding does not also fire. Hold `[` / `]` is left alone: those
/// chords nudge filter edges.
fn sat_arrow_delta(ui: &egui::Ui, keys_capture: bool) -> Option<isize> {
    if !keys_capture || ui.ctx().egui_wants_keyboard_input() {
        return None;
    }
    if ui.input(|i| i.key_down(egui::Key::OpenBracket) || i.key_down(egui::Key::CloseBracket)) {
        return None;
    }
    ui.input_mut(|i| {
        if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
            || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)
        {
            Some(1)
        } else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
            || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)
        {
            Some(-1)
        } else {
            None
        }
    })
}

/// Next/previous id in `ids` (display order), wrapping at the ends. No current
/// selection → first on ↓/→, last on ↑/←.
fn sat_step_id(ids: &[u64], current: Option<u64>, delta: isize) -> Option<u64> {
    if ids.is_empty() {
        return None;
    }
    let n = ids.len() as isize;
    let i = match current.and_then(|id| ids.iter().position(|&x| x == id)) {
        Some(i) => (i as isize + delta).rem_euclid(n) as usize,
        None if delta > 0 => 0,
        None => ids.len() - 1,
    };
    Some(ids[i])
}

/// Height of one [`sat_table_row`] — matches its allocate.
fn sat_table_row_h(ui: &egui::Ui) -> f32 {
    ui.spacing().interact_size.y.max(18.0)
}

/// Scroll offset that centres row `idx` (0-based, under a one-row heading) in
/// a viewport of height `view_h`.
fn sat_scroll_y_for_index(ui: &egui::Ui, idx: usize, view_h: f32) -> f32 {
    let row_h = sat_table_row_h(ui);
    // Heading row + small gap under it (see the list builders).
    let header_h = row_h + 4.0;
    let y = header_h + idx as f32 * row_h;
    (y - (view_h * 0.5 - row_h * 0.5)).max(0.0)
}

/// One full-width table row: a single highlight + click target spanning every
/// column, with labels painted into fixed column slots.
fn sat_table_row(
    ui: &mut egui::Ui,
    selected: bool,
    col_w: f32,
    col_gap: f32,
    cells: &[RichText],
) -> egui::Response {
    let n = cells.len().max(1) as f32;
    let row_h = ui.spacing().interact_size.y.max(18.0);
    let row_w = (col_w * n + col_gap * (n - 1.0)).max(ui.available_width());
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(row_w, row_h), egui::Sense::click());
    let visuals = ui.style().interact_selectable(&resp, selected);
    // Selected = cyan fill + dark ink (never the same color). Hover uses the
    // theme's weak fill so accent greens/yellows in the cells stay readable.
    if selected {
        ui.painter().rect_filled(rect, visuals.corner_radius, theme::CYAN());
    } else if resp.hovered() || resp.highlighted() {
        ui.painter().rect_filled(rect, visuals.corner_radius, visuals.bg_fill);
    }
    let mut x = rect.left();
    for text in cells {
        let cell = egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(col_w, row_h));
        let text = if selected {
            // `.color` overrides any green/yellow baked into the RichText so
            // the glyphs cannot disappear into the cyan fill.
            text.clone().color(theme::INK_ON_CYAN())
        } else {
            text.clone()
        };
        sat_paint_cell_text(ui, cell, text, visuals.text_color());
        x += col_w + col_gap;
    }
    resp
}

/// Clickable sort heading; returns whether this column was clicked.
/// `active` + `asc` drive the ▲/▼ marker on the current sort column.
fn sat_sort_heading(ui: &mut egui::Ui, col_w: f32, label: &str, active: bool, asc: bool) -> bool {
    let mark = if !active {
        ""
    } else if asc {
        " ▲"
    } else {
        " ▼"
    };
    let text = RichText::new(format!("{label}{mark}")).size(9.5).color(if active {
        theme::CYAN()
    } else {
        theme::CYAN_DIM()
    });
    let row_h = ui.spacing().interact_size.y.max(18.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(col_w, row_h), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 2.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    sat_paint_cell_text(ui, rect, text, theme::CYAN_DIM());
    resp.on_hover_text("Click to sort").clicked()
}

fn in_words(s: i64) -> String {
    let s = s.max(0);
    if s < 60 {
        format!("in {s} s")
    } else if s < 3600 {
        format!("in {} min", s / 60)
    } else {
        format!("in {} h {:02} min", s / 3600, (s % 3600) / 60)
    }
}

impl SdroxideApp {
    pub(in crate::app) fn sat_window(&mut self, ctx: &egui::Context, cmds: &mut Vec<Command>) {
        crate::layout::arm_center_on_open(
            self.show_sat,
            &mut self.system_was_open.sat,
            &mut self.system_center_pending.sat,
        );
        if !self.show_sat && !self.system_docked_as(crate::app::SystemDock::Sat) {
            self.sat_win.keys_capture = false;
            self.sat_win.confirm_unwatch = None;
            return;
        }
        // SatNOGS freqs live next to the TLE cache; ensure they are in memory
        // even if the solar window has never been opened this session.
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = sdroxide_solar::satnogs::status();
        }
        let mut win = std::mem::take(&mut self.sat_win);
        // The QO-100 page keeps its own waterfall rows and texture between
        // frames, exactly as the picker keeps its element list. Taken here
        // rather than inside the tab arm because `qo100_body` needs `&mut self`
        // and the state lives on `self`.
        let mut qo100 = std::mem::take(&mut self.qo100_win);
        let id = crate::layout::salted_id(ctx, "SATELLITE");
        if self.system_docked_as(crate::app::SystemDock::Sat) {
            if let Some(w) = self.system_dock_host("SATELLITE", id, crate::app::SystemDock::Sat) {
                w.show(ctx, |ui| {
                    crate::chrome::window_body_bg(ui);
                    self.clip_system_dock_body(ui, crate::app::SystemDock::Sat);
                    self.sat_body(ui, &mut win, &mut qo100, cmds);
                });
            }
            // Confirm paints after `raise_floating_system_popups` (see frame.rs).
            self.sat_win = win;
            self.qo100_win = qo100;
            return;
        }
        let mut open = self.show_sat;
        let size =
            egui::vec2(crate::layout::window_w(ctx, 480.0), crate::layout::window_h(ctx, 520.0));
        // Floor the floating window: `default_height` alone lets egui shrink to
        // content, so the catalogue / pass-memo fill (one row per frame) and the
        // pass-diagram pop-in after a pick make the window keep resizing.
        let min_h = size.y;
        let pending = self.system_center_pending.sat;
        let resp = crate::layout::center_on_open(
            egui::Window::new("SATELLITE")
                .id(id)
                .open(&mut open)
                .frame(crate::chrome::window_frame())
                .resizable(true)
                .default_width(size.x)
                .default_height(size.y)
                .min_height(min_h),
            ctx,
            pending,
            size,
        )
        .show(ctx, |ui| {
            crate::chrome::window_body_bg(ui);
            ui.set_min_height(min_h);
            self.sat_body(ui, &mut win, &mut qo100, cmds)
        });
        if let Some(r) = &resp {
            crate::chrome::paint_window_border(ctx, &r.response);
            crate::layout::settle_center_pending(ctx, id, &mut self.system_center_pending.sat);
        }
        if resp.as_ref().is_some_and(|r| self.system_title_pin(ctx, &r.response, "SATELLITE")) {
            self.dock_system(crate::app::SystemDock::Sat);
        }
        self.show_sat = open;
        // Confirm paints after `raise_floating_system_popups` (see frame.rs).
        self.sat_win = win;
        self.qo100_win = qo100;
    }

    /// Centered confirm for removing a satellite from the Watching list.
    ///
    /// Call after [`crate::app::SdroxideApp::raise_floating_system_popups`].
    pub(in crate::app) fn sat_resolve_unwatch(
        &mut self,
        ctx: &egui::Context,
        cmds: &mut Vec<Command>,
    ) {
        let Some((id, name)) = self.sat_win.confirm_unwatch.clone() else {
            return;
        };
        let body = format!(
            "Unwatch {name}? It leaves the Watching tab. You can watch it again from SATELLITES.",
        );
        match crate::chrome::confirm_modal(ctx, "sat-unwatch", &body, "Unwatch") {
            Some(crate::chrome::ConfirmChoice::Confirm) => {
                self.sat_toggle_watching(id, false, cmds);
                if self.sat_win.watch_selected == Some(id) {
                    self.sat_win.watch_selected = None;
                    self.sat_win.watch_peak_pin = None;
                    self.sat_win.watch_timeline = None;
                }
                self.sat_win.confirm_unwatch = None;
            }
            Some(crate::chrome::ConfirmChoice::Cancel) => self.sat_win.confirm_unwatch = None,
            None => {}
        }
    }

    fn sat_body(
        &mut self,
        ui: &mut egui::Ui,
        win: &mut SatWinState,
        qo100: &mut crate::app::qo100::Qo100WinState,
        cmds: &mut Vec<Command>,
    ) {
        // Click inside this panel captures the arrow keys for the list;
        // a click anywhere else (waterfall, another dock chip) releases them.
        let area = ui.max_rect();
        if ui.input(|i| i.pointer.any_click()) {
            if let Some(pos) = ui.ctx().pointer_interact_pos() {
                win.keys_capture = area.contains(pos);
            }
        }

        // A dot on the tab whose work is running, so the *other* page's state
        // is not invisible while this one is open. The same argument the
        // System chip's green accent makes one level up — here as a mark rather
        // than a colour, because a tab's colour already means "selected".
        let live = |on: bool, label: &str| {
            if on { format!("{label} •") } else { label.to_string() }
        };
        let locked = self.sat_track.is_some();
        let hunting = self.state.qo100.enabled;
        let watching_n = self.sat_cfg.watching.len();
        crate::chrome::tab_bar(ui, |ui, bar| {
            for (tab, label) in [
                (SatTab::Satellites, live(locked, "SATELLITES")),
                (
                    SatTab::Watching,
                    if watching_n > 0 {
                        format!("WATCHING ({watching_n})")
                    } else {
                        "WATCHING".to_string()
                    },
                ),
                (SatTab::Passes, "PASSES".to_string()),
                (SatTab::Qo100, live(hunting, "QO-100")),
            ] {
                if bar.tab(ui, self.sat_tab == tab, label).clicked() {
                    self.sat_tab = tab;
                }
            }
        });
        // Extra air under the tab strip — especially noticeable when docked.
        ui.add_space(14.0);

        match self.sat_tab {
            SatTab::Satellites => self.sat_page(ui, win, cmds, false),
            SatTab::Watching => self.sat_page(ui, win, cmds, true),
            SatTab::Passes => self.sat_passes_page(ui, win),
            // Its own scroll area: the beacon page is a 130 pt waterfall plus a
            // grid plus the telemetry frame, which is taller than this window
            // opens at, and the satellite page below sizes itself.
            SatTab::Qo100 => {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show_themed(ui, |ui| self.qo100_body(ui, qo100, cmds));
            }
        }
    }

    /// The satellite page: the live lock, then the picker.
    ///
    /// `watching_only` filters the list to catalogue numbers on the persisted
    /// watch list (Watching tab).
    fn sat_page(
        &mut self,
        ui: &mut egui::Ui,
        win: &mut SatWinState,
        cmds: &mut Vec<Command>,
        watching_only: bool,
    ) {
        // Catalogue is cached on `win.list` — only rebuilt when TLEs / subs
        // change. Tab switches and Watching edits leave it alone.
        let now = crate::time::now_unix();
        sat_ensure_list(win, &self.sat_cfg, &self.sat_tle_listing, now);
        if let Some(track) = self.sat_track.clone() {
            self.sat_locked_pane(ui, win, cmds, &track);
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);
        }
        // No outer scroll here: the picker sizes its list from
        // `available_height` so it fills this dock slot (and stops at the
        // clip for other dock chips). Selection detail scrolls on its own.
        self.sat_picker(ui, win, cmds, watching_only);
    }

    /// Every upcoming pass of every workable satellite over the next day or
    /// two, soonest first (#623).
    ///
    /// A click selects that satellite (one at a time) and highlights every
    /// pass it has in the table; a second click on the same bird opens it on
    /// SATELLITES with the list scrolled to the row.
    fn sat_passes_page(&mut self, ui: &mut egui::Ui, win: &mut SatWinState) {
        let dim = |s: &str| RichText::new(s).size(9.5).color(theme::CYAN_DIM());
        let now = crate::time::now_unix();
        sat_ensure_list(win, &self.sat_cfg, &self.sat_tle_listing, now);
        let Some((lat, lon)) = sdroxide_types::grid_to_latlon(&self.my_grid()) else {
            ui.label(
                RichText::new("Set your grid for passes (Settings ▸ General).")
                    .color(theme::YELLOW()),
            );
            return;
        };
        let t = &mut win.passes;
        if t.span_h == 0 {
            t.span_h = 24;
            t.min_el = 10.0;
        }
        // Top-aligned row only — `left_to_right(Align::Center)` would expand to
        // the full dock height and park these controls in the vertical middle,
        // leaving the pass table with no room below.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(dim("NEXT"));
            for h in [24u32, 48] {
                if ui.selectable_label(t.span_h == h, format!("{h} h")).clicked() {
                    t.span_h = h;
                }
            }
            ui.add_space(16.0);
            ui.label(dim("PEAK ≥"));
            ui.add(egui::DragValue::new(&mut t.min_el).range(0.0..=60.0).speed(1.0).suffix("°"));
            ui.add_space(16.0);
            crate::chrome::checkbox(ui, &mut t.all, "all satellites").on_hover_text(
                "Off: only satellites with frequencies on file — the ones you can work — and \
                 any you pasted in yourself. On: every satellite in the list, which can be \
                 thousands with a subscription and takes a while to search.",
            );
        });
        ui.add_space(12.0);

        let list = win.list.as_ref().expect("built above");
        let key = (
            list.built_unix,
            ((lat * 1e3) as i64, (lon * 1e3) as i64),
            t.span_h,
            t.min_el.round() as i32,
            t.all,
        );
        if t.key != Some(key) || now - t.from_unix > PASS_TABLE_FRESH_S {
            t.key = Some(key);
            t.from_unix = now;
            t.next = 0;
            t.rows.clear();
        }
        // A few milliseconds of searching per frame, then the rest next frame.
        let started = crate::time::now_unix_f64();
        let pasted = self.sat_cfg.tle_text();
        while t.next < list.entries.len() && crate::time::now_unix_f64() - started < 0.006 {
            let e = &list.entries[t.next];
            t.next += 1;
            let Some(sat) = e.sat.as_ref() else { continue };
            let workable = sdroxide_solar::satfreq::has_freqs(&self.sat_cfg, e.norad_id)
                || e.tle.as_ref().is_some_and(|(l1, _)| pasted.contains(l1.as_str()));
            if !t.all && !workable {
                continue;
            }
            let span = f64::from(t.span_h) * 3600.0;
            if let sdroxide_solar::PassSearch::Passes(p) =
                sat.next_passes(lat, lon, t.from_unix as f64, span, 64)
            {
                for pass in p.into_iter().filter(|p| p.max_el >= t.min_el) {
                    t.rows.push(PassRow { norad_id: e.norad_id, name: e.name.clone(), pass });
                }
            }
        }
        let searching = t.next < list.entries.len();
        if searching {
            ui.ctx().request_repaint();
        }
        let sort = t.sort;
        let asc = t.sort_asc;
        t.rows.sort_by(|a, b| {
            let ord = match sort {
                PassSort::Name => a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()),
                PassSort::Aos | PassSort::When => a.pass.rise_unix.cmp(&b.pass.rise_unix),
                PassSort::Length => {
                    (a.pass.set_unix - a.pass.rise_unix).cmp(&(b.pass.set_unix - b.pass.rise_unix))
                }
                PassSort::Peak => {
                    a.pass.max_el.partial_cmp(&b.pass.max_el).unwrap_or(std::cmp::Ordering::Equal)
                }
                PassSort::Az => {
                    a.pass.rise_az.partial_cmp(&b.pass.rise_az).unwrap_or(std::cmp::Ordering::Equal)
                }
            };
            if asc { ord } else { ord.reverse() }
        });

        let shown_idx: Vec<usize> = t
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.pass.set_unix > now)
            .map(|(i, _)| i)
            .collect();
        if searching {
            ui.label(dim(&format!("searching… {} of {}", t.next, list.entries.len())));
        } else if shown_idx.is_empty() {
            ui.label(dim("No passes in this window. Lower the peak, or widen the window."));
        }
        // Unique satellites in table order — arrows step between birds, not
        // between every pass row of the same one.
        let mut sat_ids: Vec<u64> = Vec::new();
        for &i in &shown_idx {
            let id = t.rows[i].norad_id;
            if !sat_ids.contains(&id) {
                sat_ids.push(id);
            }
        }
        // First visible-row index per NORAD — arrows / clicks scroll to that.
        let first_row_of: Vec<(u64, usize)> = {
            let mut v = Vec::new();
            for (row_i, &i) in shown_idx.iter().enumerate() {
                let id = t.rows[i].norad_id;
                if !v.iter().any(|(x, _)| *x == id) {
                    v.push((id, row_i));
                }
            }
            v
        };
        let mut arrow_scroll_row: Option<usize> = None;
        if let Some(delta) = sat_arrow_delta(ui, win.keys_capture) {
            if let Some(id) = sat_step_id(&sat_ids, win.selected, delta) {
                win.selected = Some(id);
                win.link_idx = 0;
                if let Some(&(_, idx)) = first_row_of.iter().find(|(x, _)| *x == id) {
                    arrow_scroll_row = Some(idx);
                }
            }
        }

        // One satellite at a time: every pass row of that NORAD paints selected.
        let selected_id = win.selected;
        let related_n = selected_id
            .map(|id| shown_idx.iter().filter(|&&i| t.rows[i].norad_id == id).count())
            .unwrap_or(0);
        if let Some(id) = selected_id {
            if related_n > 0 {
                let name = t
                    .rows
                    .iter()
                    .find(|r| r.norad_id == id)
                    .map(|r| r.name.as_str())
                    .unwrap_or("satellite")
                    .to_string();
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(dim(&format!(
                        "{name} · {related_n} pass{} highlighted",
                        if related_n == 1 { "" } else { "es" },
                    )));
                    if crate::chrome::chip(ui, false, "OPEN ON SATELLITES")
                        .on_hover_text("Open this satellite on the SATELLITES tab")
                        .clicked()
                    {
                        win.link_idx = 0;
                        win.search.clear();
                        win.scroll_selected_into_view = true;
                        win.list_scroll_y = None;
                        self.sat_tab = SatTab::Satellites;
                    }
                });
                ui.add_space(2.0);
            }
        }

        let mut picked = None;
        let mut sort_click: Option<PassSort> = None;
        let cur_sort = t.sort;
        let cur_asc = t.sort_asc;
        let passes_view_h = ui.available_height().max(80.0);
        if let Some(idx) = arrow_scroll_row {
            win.passes_scroll_y = Some(sat_scroll_y_for_index(ui, idx, passes_view_h));
        }
        let mut passes_scroll =
            egui::ScrollArea::vertical().id_salt("sat-passes-list").auto_shrink([false, false]);
        if let Some(y) = win.passes_scroll_y.take() {
            passes_scroll = passes_scroll.vertical_scroll_offset(y);
        }
        passes_scroll.show(ui, |ui| {
            const COL_GAP: f32 = 10.0;
            let col_ws = pass_col_widths(ui.available_width(), COL_GAP);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = COL_GAP;
                let heads = [
                    ("SATELLITE", PassSort::Name),
                    ("AOS", PassSort::Aos),
                    ("IN", PassSort::When),
                    ("LENGTH", PassSort::Length),
                    ("PEAK", PassSort::Peak),
                    ("FROM → TO", PassSort::Az),
                ];
                for (i, (label, col)) in heads.iter().enumerate() {
                    if sat_sort_heading(ui, col_ws[i], label, cur_sort == *col, cur_asc) {
                        sort_click = Some(*col);
                    }
                }
            });
            ui.add_space(2.0);
            for (row_i, &i) in shown_idx.iter().enumerate() {
                let r = &t.rows[i];
                let p = &r.pass;
                let up = p.rise_unix <= now;
                // Every pass of the selected satellite — not only the row clicked.
                let selected = selected_id == Some(r.norad_id);
                let name = {
                    let n = RichText::new(&r.name).size(11.0);
                    if up && !selected { n.color(theme::GREEN()).strong() } else { n.strong() }
                };
                let when = if up {
                    let w = RichText::new("● up now").size(10.0);
                    if selected { w } else { w.color(theme::GREEN()) }
                } else {
                    RichText::new(in_words(p.rise_unix - now)).size(10.0)
                };
                let len = p.set_unix - p.rise_unix;
                let peak = {
                    let pk = RichText::new(format!("{:.0}°", p.max_el)).size(10.0);
                    if !selected && p.max_el >= 45.0 { pk.color(theme::GREEN()) } else { pk }
                };
                let rise_az = sdroxide_solar::satellites::compass(p.rise_az);
                let set_az = sdroxide_solar::satellites::compass(p.set_az);
                let cells = [
                    name,
                    RichText::new(utc_and_local(p.rise_unix)).size(10.0),
                    when,
                    RichText::new(format!("{}:{:02}", len / 60, len % 60)).size(10.0),
                    peak,
                ];
                let resp = sat_table_row_ws(
                    ui,
                    selected,
                    &col_ws,
                    COL_GAP,
                    &cells,
                    Some((rise_az, set_az)),
                );
                if resp.clicked() {
                    picked = Some((r.norad_id, row_i));
                }
            }
        });
        if let Some(col) = sort_click {
            if t.sort == col {
                t.sort_asc = !t.sort_asc;
            } else {
                t.sort = col;
                t.sort_asc = matches!(col, PassSort::Name | PassSort::Aos | PassSort::When);
            }
        }
        if let Some((id, row_i)) = picked {
            if win.selected == Some(id) {
                // Second click on the already-selected bird: open it on Satellites
                // with the list scrolled to the highlighted row.
                win.link_idx = 0;
                win.search.clear();
                win.scroll_selected_into_view = true;
                win.list_scroll_y = None;
                self.sat_tab = SatTab::Satellites;
            } else {
                // First click: select this satellite alone and highlight every
                // pass it has in this table — stay here so the group is visible.
                win.selected = Some(id);
                win.link_idx = 0;
                win.passes_scroll_y = Some(sat_scroll_y_for_index(ui, row_i, passes_view_h));
            }
        }
    }

    /// The live pane: what the engine's lock is doing right now.
    fn sat_locked_pane(
        &mut self,
        ui: &mut egui::Ui,
        win: &mut SatWinState,
        cmds: &mut Vec<Command>,
        t: &sdroxide_types::SatTrackStatus,
    ) {
        use sdroxide_solar::satellites::compass;
        let dim = |s: &str| RichText::new(s).size(9.5).color(theme::CYAN_DIM());

        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("● LOCKED — {}", t.name))
                    .size(13.0)
                    .strong()
                    .color(if t.visible { theme::GREEN() } else { theme::YELLOW() }),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::chrome::chip(ui, false, "UNLOCK")
                    .on_hover_text("Release the lock; the dial stays where it is")
                    .clicked()
                {
                    cmds.push(Command::SetSatLock(None));
                    win.sent = None;
                }
            });
        });
        ui.add_space(4.0);

        egui::Grid::new("sat-lock-grid").num_columns(4).spacing([16.0, 3.0]).show(ui, |ui| {
            ui.label(dim("AZIMUTH"));
            ui.label(
                RichText::new(format!("{:.1}°  {}", t.az_deg, compass(t.az_deg)))
                    .size(12.5)
                    .strong(),
            );
            ui.label(dim("ELEVATION"));
            ui.label(
                RichText::new(format!("{:+.1}°", t.el_deg))
                    .size(12.5)
                    .strong()
                    .color(if t.el_deg > 0.0 { theme::GREEN() } else { theme::TEXT() }),
            );
            ui.end_row();

            ui.label(dim("RANGE"));
            ui.label(RichText::new(format!("{:.0} km", t.range_km)).size(11.5));
            ui.label(dim("RATE"));
            let dir = if t.range_rate_km_s < -0.01 {
                "approaching"
            } else if t.range_rate_km_s > 0.01 {
                "receding"
            } else {
                "abeam"
            };
            ui.label(RichText::new(format!("{:+.2} km/s {dir}", t.range_rate_km_s)).size(11.5));
            ui.end_row();

            ui.label(dim("DOWNLINK"));
            ui.label(
                RichText::new(format!("{} MHz", fmt_mhz(t.downlink_hz)))
                    .size(11.5)
                    .color(theme::GREEN()),
            );
            ui.label(dim("DOPPLER RX"));
            ui.label(RichText::new(fmt_hz_signed(t.doppler_rx_hz)).size(11.5));
            ui.end_row();

            if let Some(up) = t.uplink_hz {
                ui.label(dim("UPLINK"));
                ui.label(
                    RichText::new(format!("{} MHz", fmt_mhz(up))).size(11.5).color(theme::YELLOW()),
                );
                ui.label(dim("DOPPLER TX"));
                ui.label(RichText::new(fmt_hz_signed(t.doppler_tx_hz)).size(11.5));
                ui.end_row();
            }
        });
        ui.add_space(4.0);

        // Where the pass stands. `visible` already tells the near half of the
        // story; the memoized prediction tells the rest.
        let now = crate::time::now_unix();
        match (&t.next_pass, t.visible) {
            (_, true) => {
                if let Some(p) = &t.next_pass {
                    if (p.rise_unix..=p.set_unix).contains(&now) {
                        ui.label(dim(&format!(
                            "Pass until {} · max {:.0}°",
                            utc_and_local(p.set_unix),
                            p.max_el
                        )));
                    }
                }
            }
            (Some(p), false) => {
                ui.label(dim(&format!(
                    "Next pass {} {} ({}) · rises {:.0}° {} · max {:.0}°",
                    sdroxide_solar::timefmt::ymd(p.rise_unix),
                    utc_and_local(p.rise_unix),
                    in_words(p.rise_unix - now),
                    p.rise_az,
                    compass(p.rise_az),
                    p.max_el
                )));
            }
            (None, false) => {
                ui.label(dim("No pass inside the next 48 hours from your QTH."));
            }
        }

        // The shape of that pass, drawn from the elements *this* client holds
        // — a curated bird whose element set only the engine has keeps the
        // numbers above and goes without the diagram.
        let qth = win
            .sent
            .as_ref()
            .and_then(|c| c.observer)
            .or_else(|| sdroxide_types::grid_to_latlon(&self.my_grid()));
        if let (Some(pass), Some((lat, lon))) = (&t.next_pass, qth) {
            let sat = win
                .list
                .as_ref()
                .and_then(|l| l.entries.iter().find(|e| e.norad_id == t.norad_id))
                .and_then(|e| e.sat.as_ref());
            if let Some(sat) = sat {
                if let Some(c) = curve_for(&mut win.lock_curve, t.norad_id, sat, lat, lon, pass) {
                    ui.add_space(5.0);
                    pass_diagram(ui, c, now, DIAGRAM_H);
                    ui.add_space(2.0);
                }
            }
        }

        // The warnings, in the order an operator has to act on them.
        if t.stale_elements {
            ui.label(
                RichText::new("⚠ Elements stale — corrections suspended until a TLE refresh")
                    .size(10.5)
                    .color(theme::YELLOW()),
            );
        } else if !t.corrections_active && win.sent.as_ref().is_none_or(|c| c.doppler) {
            ui.label(
                RichText::new("Doppler correction needs an IQ front end — a CAT rig tracks only")
                    .size(10.5)
                    .color(theme::CYAN_DIM()),
            );
        }
        if let Some(s) = &self.rigctld_status {
            if s.running && s.clients > 0 {
                ui.label(
                    RichText::new(format!(
                        "⚠ {} rigctld client{} connected — satellite software steering the dial \
                         there would correct Doppler twice",
                        s.clients,
                        if s.clients == 1 { "" } else { "s" }
                    ))
                    .size(10.5)
                    .color(theme::YELLOW()),
                );
            }
        }
        if let Some((connected, az, el, err)) = &self.rotator_status {
            if win.sent.as_ref().is_some_and(|c| c.rotator) {
                let line = if *connected {
                    RichText::new(format!("Rotator: antenna at {az:.0}° az {el:.0}° el"))
                        .color(theme::CYAN_DIM())
                } else {
                    match err {
                        Some(e) => RichText::new(format!("Rotator: {e}")).color(theme::YELLOW()),
                        None => RichText::new("Rotator: not connected").color(theme::CYAN_DIM()),
                    }
                };
                ui.label(line.size(10.5));
            }
        }

        // The two switches ride the lock config, so flipping one is just the
        // same command again with one field changed.
        if let Some(sent) = win.sent.as_mut() {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if crate::chrome::chip(ui, sent.doppler, "DOPPLER")
                    .on_hover_text("Apply the computed correction to RX and TX")
                    .clicked()
                {
                    sent.doppler = !sent.doppler;
                    cmds.push(Command::SetSatLock(Some(Box::new(sent.clone()))));
                }
                if crate::chrome::chip(ui, sent.rotator, "ROTATOR")
                    .on_hover_text(
                        "Steer the antenna through the rotctld client (Settings ▸ Servers)",
                    )
                    .clicked()
                {
                    sent.rotator = !sent.rotator;
                    cmds.push(Command::SetSatLock(Some(Box::new(sent.clone()))));
                }
            });
        }
    }

    /// The picker: every satellite the station tracks, searchable, with the
    /// published links of the chosen one and the TUNE / LOCK / WATCH actions.
    ///
    /// When `watching_only`, only catalogue numbers on the persisted watch list
    /// are shown (Watching tab).
    fn sat_picker(
        &mut self,
        ui: &mut egui::Ui,
        win: &mut SatWinState,
        cmds: &mut Vec<Command>,
        watching_only: bool,
    ) {
        let dim = |s: &str| RichText::new(s).size(9.5).color(theme::CYAN_DIM());
        let now = crate::time::now_unix();
        let qth = sdroxide_types::grid_to_latlon(&self.my_grid());
        let watching = self.sat_cfg.watching.clone();

        let list = win.list.as_mut().expect("built in sat_body");

        if watching_only && watching.is_empty() {
            ui.label(
                RichText::new(
                    "No satellites on the watch list. Select one on SATELLITES and press WATCH.",
                )
                .size(10.5)
                .color(theme::CYAN_DIM()),
            );
            return;
        }

        // `horizontal_top` — plain `horizontal` centres on the cross axis and
        // expands to the full remaining dock height, which parks the picker
        // (and the Watching timeline under a selection) in the middle.
        ui.horizontal_top(|ui| {
            ui.label(RichText::new("🔍").size(11.0));
            crate::chrome::field(
                ui,
                egui::TextEdit::singleline(&mut win.search)
                    .desired_width(160.0)
                    .hint_text("name or NORAD"),
            );
            if !win.search.is_empty() && ui.small_button("×").clicked() {
                win.search.clear();
            }
            if qth.is_none() {
                ui.label(
                    RichText::new("set your grid for passes (Settings ▸ General)")
                        .size(9.5)
                        .color(theme::YELLOW()),
                );
            }
        });
        ui.add_space(4.0);

        // The tab's selection always stays in the listing — a search string must
        // not hide the row the detail pane is open on (and a Watching pick must
        // still appear even if search no longer matches).
        let keep_id = if watching_only { win.watch_selected } else { win.selected };
        let in_scope = |e: &SatEntry| {
            let listed =
                e.matches(&win.search) && (!watching_only || watching.contains(&e.norad_id));
            listed || keep_id == Some(e.norad_id)
        };

        // Fill at most one pass memo per frame — see the module note.
        if let Some((lat, lon)) = qth {
            if let Some(e) = list
                .entries
                .iter_mut()
                .filter(|e| in_scope(e) && e.sat.is_some())
                .find(|e| e.pass.as_ref().is_none_or(|(at, _)| now - at > MEMO_FRESH_S))
            {
                let sat = e.sat.as_ref().expect("filtered above");
                let memo = match sat.next_passes(lat, lon, now as f64, 48.0 * 3600.0, 1) {
                    sdroxide_solar::PassSearch::Passes(p) => match p.first() {
                        Some(p) => PassMemo::Pass(sdroxide_types::SatPass {
                            rise_unix: p.rise_unix,
                            set_unix: p.set_unix,
                            rise_az: p.rise_az,
                            set_az: p.set_az,
                            max_el: p.max_el,
                            max_el_unix: p.max_el_unix,
                        }),
                        None => PassMemo::Never,
                    },
                    sdroxide_solar::PassSearch::AlwaysVisible { .. } => PassMemo::Always,
                    sdroxide_solar::PassSearch::NeverVisible => PassMemo::Never,
                };
                e.pass = Some((now, memo));
                ui.ctx().request_repaint(); // keep filling the memos
            }
        }

        // Displayed rows: what matches, then ordered by the heading the
        // operator last clicked (load default: name — stable while memos fill).
        let mut rows: Vec<(usize, Option<f64>, i64)> = list
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| in_scope(e))
            .map(|(i, e)| {
                let el = match (&e.sat, qth) {
                    (Some(s), Some((lat, lon))) => {
                        s.at(now as f64).map(|st| st.elevation_from(lat, lon))
                    }
                    _ => None,
                };
                let rise = match &e.pass {
                    Some((_, PassMemo::Pass(p))) => p.rise_unix,
                    Some((_, PassMemo::Always)) => 0,
                    _ => i64::MAX,
                };
                (i, el, rise)
            })
            .collect();
        let list_asc = win.list_sort_asc;
        // Pass sort keys off memos that arrive one per frame — applying it
        // before every in-scope memo is ready would reshuffle the list on
        // every paint. Hold Name until the fill catches up; the PASS heading
        // still shows as selected so the operator's choice is visible.
        let pass_memos_pending = win.list_sort == ListSort::Pass
            && rows
                .iter()
                .any(|(i, _, _)| list.entries[*i].sat.is_some() && list.entries[*i].pass.is_none());
        let list_sort = if pass_memos_pending { ListSort::Name } else { win.list_sort };
        rows.sort_by(|a, b| {
            let ord = match list_sort {
                ListSort::Name => list.entries[a.0]
                    .name
                    .to_ascii_lowercase()
                    .cmp(&list.entries[b.0].name.to_ascii_lowercase()),
                ListSort::Norad => list.entries[a.0].norad_id.cmp(&list.entries[b.0].norad_id),
                ListSort::Pass => {
                    let vis_a = a.1.unwrap_or(-90.0) > 0.0;
                    let vis_b = b.1.unwrap_or(-90.0) > 0.0;
                    vis_b.cmp(&vis_a).then_with(|| {
                        if vis_a && vis_b {
                            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
                        } else {
                            a.2.cmp(&b.2)
                        }
                    })
                }
            };
            let ord = ord.then_with(|| list.entries[a.0].name.cmp(&list.entries[b.0].name));
            if list_asc { ord } else { ord.reverse() }
        });

        // Arrow keys walk the visible list (Satellites / Watching).
        // Height: shrink to content when the list is short so the pass
        // timeline / diagram sits right under the rows; grow into the
        // remaining slot when there are hundreds of rows — never past
        // `available_height`. Window-level `min_height` (undocked) stops
        // the popup resizing while memos fill.
        const LIST_MIN_H: f32 = 80.0;
        let row_ids: Vec<u64> = rows.iter().map(|(i, _, _)| list.entries[*i].norad_id).collect();
        let tab_sel = if watching_only { win.watch_selected } else { win.selected };
        let detail_open = tab_sel.is_some_and(|id| row_ids.contains(&id));
        // Leave a usable strip for timeline / links / the "pick one" hint so
        // the list does not eat the whole slot when a selection is open.
        let detail_reserve = if detail_open {
            if watching_only { TIMELINE_H + 120.0 } else { DIAGRAM_H_SMALL + 140.0 }
        } else {
            28.0
        };
        let row_h = sat_table_row_h(ui);
        let list_content_h = row_h + 4.0 + rows.len() as f32 * row_h;
        // Viewport = remaining height (capped). Content taller than this
        // scrolls inside the ScrollArea — do not set max_height to content_h
        // or the list expands past the slot and scrolling dies.
        let list_budget = (ui.available_height() - detail_reserve).max(LIST_MIN_H);
        let list_overflows = list_content_h > list_budget + 0.5;
        let list_view_h = if list_overflows { list_budget } else { list_content_h };
        if let Some(delta) = sat_arrow_delta(ui, win.keys_capture) {
            let cur = if watching_only { win.watch_selected } else { win.selected };
            if let Some(id) = sat_step_id(&row_ids, cur, delta) {
                if watching_only {
                    win.watch_selected = Some(id);
                    win.watch_peak_pin = None;
                } else {
                    win.selected = Some(id);
                }
                if let Some(idx) = row_ids.iter().position(|&x| x == id) {
                    win.list_scroll_y = Some(sat_scroll_y_for_index(ui, idx, list_view_h));
                }
                win.scroll_selected_into_view = false;
                win.link_idx = 0;
            }
        }
        // A jump that set the flag without an offset (PASSES / 3D) — resolve now.
        if win.scroll_selected_into_view {
            let id = if watching_only { win.watch_selected } else { win.selected };
            if let Some(idx) = id.and_then(|id| row_ids.iter().position(|&x| x == id)) {
                win.list_scroll_y = Some(sat_scroll_y_for_index(ui, idx, list_view_h));
            }
            win.scroll_selected_into_view = false;
        }

        let mut sort_click: Option<ListSort> = None;
        let mut list_scroll = egui::ScrollArea::vertical()
            .id_salt(if watching_only { "sat-watch-list" } else { "sat-list" })
            .max_height(list_view_h)
            // Overflowing lists keep a fixed viewport so the bar can scroll;
            // short lists shrink to their rows so the detail pane sits under them.
            .auto_shrink([false, !list_overflows]);
        if let Some(y) = win.list_scroll_y.take() {
            list_scroll = list_scroll.vertical_scroll_offset(y);
        }
        list_scroll.show(ui, |ui| {
            const COLS: usize = 3;
            const COL_GAP: f32 = 14.0;
            let col_w =
                ((ui.available_width() - COL_GAP * (COLS as f32 - 1.0)) / COLS as f32).max(80.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = COL_GAP;
                for (label, col) in [
                    ("SATELLITE", ListSort::Name),
                    ("NORAD", ListSort::Norad),
                    ("PASS", ListSort::Pass),
                ] {
                    // Highlight the operator's choice even while Pass sort is
                    // held at Name until memos finish filling.
                    if sat_sort_heading(ui, col_w, label, win.list_sort == col, list_asc) {
                        sort_click = Some(col);
                    }
                }
            });
            ui.add_space(2.0);
            for (row_i, (i, el, _)) in rows.iter().enumerate() {
                let e = &list.entries[*i];
                let selected = if watching_only {
                    win.watch_selected == Some(e.norad_id)
                } else {
                    win.selected == Some(e.norad_id)
                };
                let name = {
                    let n = RichText::new(&e.name).size(11.0);
                    match el {
                        Some(el) if *el > 0.0 && !selected => n.color(theme::GREEN()).strong(),
                        _ => n.strong(),
                    }
                };
                let pass = match (el, &e.pass, &e.sat) {
                    (Some(el), _, _) if *el > 0.0 => {
                        let t = RichText::new(format!("● up now, {el:.0}°"));
                        if selected { t } else { t.color(theme::GREEN()) }
                    }
                    (_, Some((_, PassMemo::Pass(p))), _) => {
                        RichText::new(in_words(p.rise_unix - now))
                    }
                    (_, Some((_, PassMemo::Always)), _) => {
                        let t = RichText::new("always visible");
                        if selected { t } else { t.color(theme::GREEN()) }
                    }
                    (_, Some((_, PassMemo::Never)), _) => {
                        let t = RichText::new("not from here");
                        if selected { t } else { t.color(theme::CYAN_DIM()) }
                    }
                    (_, None, None) => {
                        let t = RichText::new("no elements");
                        if selected { t } else { t.color(theme::YELLOW()) }
                    }
                    (_, None, Some(_)) => RichText::new("…"),
                }
                .size(10.0);
                let norad = RichText::new(e.norad_id.to_string()).size(10.0);
                let cells = [name, norad, pass];
                let resp = sat_table_row(ui, selected, col_w, COL_GAP, &cells);
                // Do not force-scroll every frame when the highlight is off-
                // screen — that fights the operator's wheel / drag. Arrow keys
                // and clicks set `list_scroll_y` explicitly.
                if resp.clicked() {
                    if watching_only {
                        win.watch_selected = if selected { None } else { Some(e.norad_id) };
                        win.watch_peak_pin = None;
                    } else {
                        win.selected = if selected { None } else { Some(e.norad_id) };
                    }
                    if !selected {
                        win.list_scroll_y = Some(sat_scroll_y_for_index(ui, row_i, list_view_h));
                    }
                    win.link_idx = 0;
                }
            }
        });
        if let Some(col) = sort_click {
            if win.list_sort == col {
                win.list_sort_asc = !win.list_sort_asc;
            } else {
                win.list_sort = col;
                win.list_sort_asc = true;
            }
        }

        // Detail / graphs belong only to a satellite that is selected *and*
        // still visible in this tab's list — a Satellites pick must not open
        // the Watching timeline, and a filtered-out row keeps no peak diagram.
        let tab_selected = if watching_only { win.watch_selected } else { win.selected };
        let Some(id) =
            tab_selected.filter(|&id| rows.iter().any(|(i, _, _)| list.entries[*i].norad_id == id))
        else {
            ui.add_space(4.0);
            ui.label(dim(if watching_only {
                "Pick a watched satellite to see its timeline, links and lock on."
            } else {
                "Pick a satellite to see its links and lock on."
            }));
            return;
        };
        // Drop a stale Watching pick that was unwatched while this tab was away.
        if watching_only && !self.sat_cfg.is_watching(id) {
            win.watch_selected = None;
            return;
        }
        let Some(entry_idx) = list.entries.iter().position(|e| e.norad_id == id) else {
            if watching_only {
                win.watch_selected = None;
            } else {
                win.selected = None;
            }
            return;
        };

        // Operator override → SatNOGS cache → built-in curated table.
        let freqs = sdroxide_solar::satfreq::for_norad(&self.sat_cfg, id);
        // Selection chrome (timeline / links / actions) fills what the list
        // reserved — fixed height so a late pass-diagram / SatNOGS link table
        // does not grow the undocked popup.
        let detail_h = ui.available_height().max(48.0);
        egui::ScrollArea::vertical()
            .id_salt(if watching_only { "sat-watch-detail" } else { "sat-sat-detail" })
            .max_height(detail_h)
            .auto_shrink([false, false])
            .show_themed(ui, |ui| {
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);

                // The selected bird's next pass, unless the lock pane above is already
                // showing this one's. Ahead of the links on purpose: *when* it can be
                // worked comes before *how*, and a satellite with no frequencies on
                // file returns below this point without ever reaching the links.
                //
                // On the Watching tab the operator wants the whole day (or two): a
                // multi-pass timeline with peak elevation and time on each bump, plus
                // 24 h / 48 h chips — instead of the single next-pass profile.
                let locked_here = self.sat_track.as_ref().is_some_and(|t| t.norad_id == id);
                if watching_only {
                    if qth.is_none() {
                        ui.label(
                            RichText::new(
                                "Set your grid to see the pass timeline (Settings ▸ General).",
                            )
                            .size(9.5)
                            .color(theme::YELLOW()),
                        );
                        ui.add_space(4.0);
                    } else if list.entries[entry_idx].sat.is_none() {
                        ui.label(dim("No elements for this satellite — the timeline needs a TLE."));
                        ui.add_space(4.0);
                    } else if let Some((lat, lon)) = qth {
                        // Top-aligned — `left_to_right(Align::Center)` expands to the
                        // remaining height and centres this strip mid-panel.
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.label(dim("WATCHING"));
                            for &h in WATCH_SPANS_H {
                                if crate::chrome::chip(ui, win.watch_span_h == h, format!("{h}h"))
                                    .clicked()
                                {
                                    win.watch_span_h = h;
                                    win.watch_peak_pin = None;
                                }
                            }
                        });
                        ui.add_space(4.0);
                        let sat = list.entries[entry_idx].sat.as_ref().expect("checked above");
                        let span_h = win.watch_span_h;
                        timeline_for(&mut win.watch_timeline, id, sat, lat, lon, span_h, now);
                        let tl = win.watch_timeline.as_ref().expect("just built");
                        pass_timeline(ui, tl, now, TIMELINE_H, &mut win.watch_peak_pin);
                        if let TimelineKind::Passes(passes) = &tl.kind {
                            if let Some(next) = passes.iter().find(|p| p.max_el_unix >= now) {
                                ui.add_space(2.0);
                                ui.label(dim(&format!(
                                    "Next peak {:.0}° {} · AOS {} · LOS {}",
                                    next.max_el,
                                    utc_date_and_local(next.max_el_unix),
                                    utc_date_and_local(next.rise_unix),
                                    utc_date_and_local(next.set_unix),
                                )));
                            }
                        }
                        ui.add_space(6.0);
                    }
                } else if !locked_here {
                    let e = &list.entries[entry_idx];
                    if let (Some(sat), Some((_, PassMemo::Pass(pass))), Some((lat, lon))) =
                        (e.sat.as_ref(), e.pass.as_ref(), qth)
                    {
                        if let Some(c) = curve_for(&mut win.pick_curve, id, sat, lat, lon, pass) {
                            pass_diagram(ui, c, now, DIAGRAM_H_SMALL);
                            ui.add_space(6.0);
                        }
                    }
                }

                let links: Vec<sdroxide_types::SatLink> =
                    freqs.as_ref().map(|f| f.usable_links().cloned().collect()).unwrap_or_default();
                if links.is_empty() {
                    ui.label(dim(
                        "No frequencies on file for this one — add them in Settings ▸ TLE, \
                         or fetch SatNOGS transmitters there.",
                    ));
                    return;
                }
                win.link_idx = win.link_idx.min(links.len() - 1);

                // Gutter for the detail ScrollArea's vertical bar — MODE sits on
                // the right edge and otherwise paints under it (same idea as the
                // WEFAX live chart fitting). Fixed column widths so a long LINK
                // name cannot shove FM / SSTV / … under the bar.
                let gut = ui.spacing().scroll.bar_width + 4.0;
                let usable = (ui.available_width() - gut).max(200.0);
                const LINK_GAP: f32 = 14.0;
                let mode_w = 56.0_f32;
                let freq_w = 88.0_f32;
                let link_w = (usable - mode_w - 2.0 * freq_w - 3.0 * LINK_GAP).max(80.0);
                let row_h = ui.spacing().interact_size.y.max(18.0);
                ui.set_max_width(usable);
                egui::Grid::new("sat-links").num_columns(4).spacing([LINK_GAP, 2.0]).show(
                    ui,
                    |ui| {
                        for (label, w) in [
                            ("LINK", link_w),
                            ("DOWNLINK (MHz)", freq_w),
                            ("UPLINK (MHz)", freq_w),
                            ("MODE", mode_w),
                        ] {
                            ui.allocate_ui_with_layout(
                                egui::vec2(w, row_h),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.set_min_width(w);
                                    ui.add(egui::Label::new(dim(label)).truncate());
                                },
                            );
                        }
                        ui.end_row();
                        for (i, l) in links.iter().enumerate() {
                            let sel = i == win.link_idx;
                            // The selected row sits on the cyan selection fill and
                            // keeps the label's own color, so that color has to be
                            // the dark on-cyan ink or it vanishes into the fill.
                            let mut label = RichText::new(&l.label).size(10.5);
                            if sel {
                                label = label.color(theme::INK_ON_CYAN()).strong();
                            }
                            // Left-aligned — `Button::selectable` / `add_sized` centres
                            // the name in the wide LINK column.
                            let resp = ui
                                .allocate_ui_with_layout(
                                    egui::vec2(link_w, row_h),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.set_max_width(link_w);
                                        ui.selectable_label(sel, label)
                                    },
                                )
                                .inner;
                            if !l.note.is_empty() {
                                resp.clone().on_hover_text(&l.note);
                            }
                            if resp.clicked() {
                                win.link_idx = i;
                            }
                            ui.add_sized(
                                egui::vec2(freq_w, row_h),
                                egui::Label::new(
                                    RichText::new(
                                        l.downlink.map_or_else(|| "—".into(), |b| b.to_string()),
                                    )
                                    .size(10.5)
                                    .color(theme::GREEN()),
                                )
                                .truncate(),
                            );
                            ui.add_sized(
                                egui::vec2(freq_w, row_h),
                                egui::Label::new(
                                    RichText::new(
                                        l.uplink.map_or_else(|| "—".into(), |b| b.to_string()),
                                    )
                                    .size(10.5)
                                    .color(theme::YELLOW()),
                                )
                                .truncate(),
                            );
                            let mode = if l.inverting {
                                format!("{} · inv", l.mode)
                            } else {
                                l.mode.clone()
                            };
                            ui.add_sized(
                                egui::vec2(mode_w, row_h),
                                egui::Label::new(RichText::new(mode).size(10.5)).truncate(),
                            );
                            ui.end_row();
                        }
                    },
                );
                let link = &links[win.link_idx];
                if !link.note.is_empty() {
                    ui.label(dim(&format!("{} — {}", link.label, link.note)));
                }
                ui.add_space(6.0);

                let watching_here = self.sat_cfg.is_watching(id);
                ui.horizontal_top(|ui| {
                    if crate::chrome::chip(ui, false, "TUNE")
                        .on_hover_text(
                            "Set the dial and decoder to this link — no lock, no correction",
                        )
                        .clicked()
                    {
                        // Mode first so a decoder that parks on a convention
                        // frequency (APRS, …) cannot overwrite the downlink we
                        // set next — same order as a memory recall.
                        cmds.push(Command::SetMode {
                            rx: sdroxide_types::RxId::Main,
                            mode: mode_for_link(link),
                        });
                        if link.downlink.is_some() {
                            cmds.push(Command::SetVfo {
                                vfo: self.state.active_vfo,
                                hz: lock_downlink_hz(link, self.state.active_freq_hz()),
                            });
                        }
                    }
                    // Green fill needs [`theme::INK_ON_BRIGHT`], not the cyan-chip ink —
                    // several themes' greens are mid-bright and wash out dark `INK_ON_CYAN`.
                    if crate::chrome::chip_accent(
                        ui,
                        locked_here,
                        if locked_here { " LOCKED " } else { " LOCK ON " },
                        theme::GREEN(),
                        theme::INK_ON_BRIGHT(),
                    )
                    .on_hover_text(
                        "Track this satellite: Doppler correction on RX and TX, transponder-mapped \
                 uplink, and the 3D view follows it",
                    )
                    .clicked()
                        && !locked_here
                    {
                        let e = &list.entries[entry_idx];
                        let cfg = SatLockConfig {
                            norad_id: id,
                            name: e.name.clone(),
                            tle: e.tle.clone(),
                            observer: qth,
                            downlink_hz: lock_downlink_hz(link, self.state.active_freq_hz()),
                            uplink: SatUplink::from_link(link),
                            doppler: true,
                            rotator: win.sent.as_ref().map(|c| c.rotator).unwrap_or(true),
                        };
                        cmds.push(Command::SetMode {
                            rx: sdroxide_types::RxId::Main,
                            mode: mode_for_link(link),
                        });
                        cmds.push(Command::SetSatLock(Some(Box::new(cfg.clone()))));
                        win.sent = Some(cfg);
                    }
                    if crate::chrome::chip(
                        ui,
                        watching_here,
                        if watching_here { "WATCHING" } else { "WATCH" },
                    )
                    .on_hover_text(if watching_here {
                        "Remove this satellite from the Watching tab"
                    } else {
                        "Add this satellite to the Watching tab"
                    })
                    .clicked()
                    {
                        if watching_here {
                            win.confirm_unwatch = Some((id, list.entries[entry_idx].name.clone()));
                        } else {
                            self.sat_toggle_watching(id, true, cmds);
                        }
                    }
                });
            }); // selection detail ScrollArea
    }

    /// Persist a watch-list change the same way Settings writes satellite
    /// config — through `SetSatConfig`, so the engine keeps it across restarts.
    fn sat_toggle_watching(&mut self, norad_id: u64, watch: bool, cmds: &mut Vec<Command>) {
        if !self.sat_cfg_seeded {
            return;
        }
        let mut edit = (*self.sat_cfg).clone();
        if !edit.set_watching(norad_id, watch) {
            return;
        }
        self.sat_cfg_edit = edit.clone();
        self.sat_cfg = std::sync::Arc::new(edit.clone());
        cmds.push(Command::SetSatConfig(edit));
    }

    /// Lock on from outside the window — the 3D view's LOCK button lands
    /// here. Picks the satellite's first two-way link (or first link at all),
    /// and opens the window on it either way so the operator can refine.
    /// Native-only caller (the 3D viewport), hence the wasm allowance.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(in crate::app) fn sat_lock_request(&mut self, norad_id: u64, cmds: &mut Vec<Command>) {
        self.show_sat = true;
        self.sat_tab = SatTab::Satellites;
        self.sat_win.selected = Some(norad_id);
        self.sat_win.link_idx = 0;
        self.sat_win.search.clear();
        self.sat_win.scroll_selected_into_view = true;
        let freqs = sdroxide_solar::satfreq::for_norad(&self.sat_cfg, norad_id);
        let links: Vec<sdroxide_types::SatLink> =
            freqs.as_ref().map(|f| f.usable_links().cloned().collect()).unwrap_or_default();
        // Prefer the first link that can be worked both ways; a beacon-only
        // satellite still locks for RX.
        let Some((idx, link)) = links
            .iter()
            .enumerate()
            .find(|(_, l)| l.uplink.is_some() && l.downlink.is_some())
            .or_else(|| links.first().map(|l| (0, l)))
        else {
            // No frequencies at all: open the window; the picker explains.
            return;
        };
        self.sat_win.link_idx = idx;
        let entry = self
            .sat_win
            .list
            .as_ref()
            .and_then(|l| l.entries.iter().find(|e| e.norad_id == norad_id));
        let name = entry
            .map(|e| e.name.clone())
            .or_else(|| freqs.as_ref().map(|f| f.name.clone()).filter(|n| !n.is_empty()))
            .unwrap_or_else(|| format!("NORAD {norad_id}"));
        let tle = entry.and_then(|e| e.tle.clone());
        let cfg = SatLockConfig {
            norad_id,
            name,
            tle,
            observer: sdroxide_types::grid_to_latlon(&self.my_grid()),
            downlink_hz: lock_downlink_hz(link, self.state.active_freq_hz()),
            uplink: SatUplink::from_link(link),
            doppler: true,
            rotator: true,
        };
        cmds.push(Command::SetMode { rx: sdroxide_types::RxId::Main, mode: mode_for_link(link) });
        cmds.push(Command::SetSatLock(Some(Box::new(cfg.clone()))));
        self.sat_win.sent = Some(cfg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdroxide_types::{Passband, SatLink};

    /// Issue #622: TUNE on the ISS SSTV downlink lands in SSTV-FM, where the
    /// picture is decoded, rather than plain NFM or USB.
    #[test]
    fn an_sstv_downlink_tunes_in_sstv_fm() {
        let sstv = SatLink::down("SSTV 70 cm", "SSTV FM / PD120", Passband::at(437.550));
        assert_eq!(mode_for_link(&sstv), Mode::SstvFm);
        // An operator's own entry may say SSTV only in the label.
        let own = SatLink::down("SSTV", "FM", Passband::at(145.800));
        assert_eq!(mode_for_link(&own), Mode::SstvFm);
        // SatNOGS publishes mode "SSTV" with no "FM" — still the FM decoder.
        let satnogs = SatLink::down("Mode U - SSTV", "SSTV", Passband::at(437.800));
        assert_eq!(mode_for_link(&satnogs), Mode::SstvFm);
        let robot = SatLink::down("Mode U - SSTV - Robot-36", "SSTV", Passband::at(437.550));
        assert_eq!(mode_for_link(&robot), Mode::SstvFm);
        // Voice stays voice.
        let voice = SatLink::down("FM voice", "FM", Passband::at(145.800));
        assert_eq!(mode_for_link(&voice), Mode::Nfm);
    }
}
