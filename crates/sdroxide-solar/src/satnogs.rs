//! SatNOGS transmitter database: published uplinks / downlinks / modes.
//!
//! A TLE only says where a satellite is. Working it also needs frequencies,
//! and those live in SatNOGS's transmitters API — not in the element-set feed.
//! This module fetches that JSON, caches it next to the other solar products,
//! and answers [`cached_for`] so the SAT panel and solar overlay can show a
//! link table for birds the built-in curated list never heard of.
//!
//! Operator overrides in [`sdroxide_types::SatConfig::freqs`] still win; the
//! built-in table in [`crate::satfreq`] is the last fallback. See
//! [`crate::satfreq::for_norad`].

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use sdroxide_types::{Passband, SatFreqs, SatLink};
use serde::{Deserialize, Serialize};

use crate::cache::{Cache, Validators};

/// SatNOGS DB transmitters listing (JSON array, or paginated `{results,next}`).
///
/// `status=active` keeps the dump smaller and matches what Look4Sat / similar
/// clients ask for. Rate limits return HTTP 429 — a TCP "connection refused"
/// is the host being unreachable, not a throttle.
pub const URL: &str =
    "https://db.satnogs.org/api/transmitters/?format=json&status=active&limit=5000";

/// On-disk body: parsed [`SatFreqs`] plus when they were fetched. Stable name
/// so query-string changes to [`URL`] do not orphan a good dump.
const CACHE_NAME: &str = "satnogs_transmitters.json";
/// Index / ETag key — also stable across URL tweaks.
const CACHE_KEY: &str = "satnogs-transmitters";
/// Full dump is a few megabytes; leave headroom for growth.
const BODY_LIMIT: u64 = 16 * 1024 * 1024;
/// Frequencies change slowly; daily is enough beside the six-hour TLE cadence.
pub const PERIOD_S: i64 = 24 * 3600;

/// What the last fetch did — for Settings ▸ TLE.
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub fetched_unix: i64,
    /// Transmitter rows kept after filtering.
    pub transmitters: usize,
    /// Distinct NORAD catalogue numbers those rows cover.
    pub satellites: usize,
    pub error: Option<String>,
}

/// Durable cache file: last good parse survives failed refreshes and restarts.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct CacheFile {
    fetched_unix: i64,
    freqs: Vec<SatFreqs>,
}

#[derive(Default)]
struct Table {
    by_norad: HashMap<u64, SatFreqs>,
    status: Status,
}

fn table() -> &'static Mutex<Table> {
    static T: OnceLock<Mutex<Table>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(Table::default()))
}

fn status_from_freqs(fetched_unix: i64, freqs: &[SatFreqs]) -> Status {
    Status {
        fetched_unix,
        transmitters: freqs.iter().map(|f| f.links.len()).sum(),
        satellites: freqs.len(),
        error: None,
    }
}

fn install(freqs: Vec<SatFreqs>, fetched_unix: i64, error: Option<String>) -> Status {
    let mut status = status_from_freqs(fetched_unix, &freqs);
    status.error = error;
    let mut t = table().lock().unwrap_or_else(|e| e.into_inner());
    t.by_norad = freqs.into_iter().map(|f| (f.norad_id, f)).collect();
    t.status = status.clone();
    status
}

/// Keep the last good table; only update the error string shown in Settings.
fn keep_existing(error: String) -> Status {
    let mut t = table().lock().unwrap_or_else(|e| e.into_inner());
    if t.by_norad.is_empty() {
        drop(t);
        load_cached();
        t = table().lock().unwrap_or_else(|e| e.into_inner());
    }
    t.status.error = Some(error);
    t.status.clone()
}

fn write_cache(freqs: &[SatFreqs], fetched_unix: i64, validators: Validators) {
    let file = CacheFile { fetched_unix, freqs: freqs.to_vec() };
    let Ok(text) = serde_json::to_string(&file) else {
        return;
    };
    let mut cache = Cache::open();
    cache.write(CACHE_NAME, CACHE_KEY, text.as_bytes(), validators);
}

/// Load the on-disk cache into memory (startup / after another process wrote it).
///
/// Never clears a populated in-memory table if the file is missing or corrupt —
/// a failed read must not blank frequencies the session already has.
pub fn load_cached() {
    let cache = Cache::open();
    let Some(text) = cache.read_string(CACHE_NAME) else {
        return;
    };
    let (freqs, fetched_unix) = match read_cache_bytes(&text, cache.fetched_at(CACHE_KEY)) {
        Ok(v) => v,
        Err(e) => {
            let mut t = table().lock().unwrap_or_else(|e| e.into_inner());
            if t.by_norad.is_empty() {
                t.status.error = Some(format!("cache unreadable: {e}"));
            } else {
                t.status.error = Some(format!("cache unreadable (keeping memory): {e}"));
            }
            return;
        }
    };
    install(freqs, fetched_unix, None);
}

/// Accept our [`CacheFile`] shape, or a legacy raw SatNOGS JSON dump.
fn read_cache_bytes(text: &str, index_fetched: i64) -> Result<(Vec<SatFreqs>, i64), String> {
    if let Ok(file) = serde_json::from_str::<CacheFile>(text) {
        if file.freqs.is_empty() && file.fetched_unix == 0 {
            return Err("empty cache file".into());
        }
        let fetched = if file.fetched_unix > 0 { file.fetched_unix } else { index_fetched };
        return Ok((file.freqs, fetched));
    }
    // Older builds wrote the raw SatNOGS body; migrate in place on next success.
    let freqs = parse_and_group(text)?;
    Ok((freqs, index_fetched))
}

/// Status from memory (after [`load_cached`] / [`refresh`]), else from disk.
pub fn status() -> Status {
    {
        let t = table().lock().unwrap_or_else(|e| e.into_inner());
        if t.status.fetched_unix > 0 || t.status.error.is_some() || !t.by_norad.is_empty() {
            return t.status.clone();
        }
    }
    load_cached();
    table().lock().unwrap_or_else(|e| e.into_inner()).status.clone()
}

/// Frequencies for one catalogue number from the SatNOGS cache, if any.
///
/// Uses the last *successful* fetch until a newer one replaces it — a failed
/// UPDATE NOW does not remove links already on file.
pub fn cached_for(norad_id: u64) -> Option<SatFreqs> {
    {
        let t = table().lock().unwrap_or_else(|e| e.into_inner());
        if !t.by_norad.is_empty() {
            return t.by_norad.get(&norad_id).cloned();
        }
    }
    load_cached();
    table().lock().unwrap_or_else(|e| e.into_inner()).by_norad.get(&norad_id).cloned()
}

/// True when a refresh is due (never fetched, or older than [`PERIOD_S`]).
pub fn is_due(now_unix: i64) -> bool {
    let st = status();
    st.fetched_unix == 0 || now_unix.saturating_sub(st.fetched_unix) >= PERIOD_S
}

/// Fetch the SatNOGS transmitters listing, update cache + memory.
///
/// Blocking HTTPS — call off the UI / engine tick thread. A failed fetch keeps
/// the previous cache on disk and in memory (same rule as TLE subscriptions):
/// the last good dump is used until a newer fetch parses cleanly.
pub fn refresh() -> Status {
    // Prefer IPv4: some networks resolve an AAAA that RSTs on :443 while the
    // A record would work (or time out more honestly).
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(120)))
        .timeout_connect(Some(std::time::Duration::from_secs(20)))
        .ip_family(ureq::config::IpFamily::Ipv4Only)
        .user_agent(concat!("sdroxide/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut cache = Cache::open();
    let now = now_unix();
    match fetch_all(&agent, &mut cache) {
        Ok(FetchResult::NotModified) => {
            // Still reachable; bump the "fresh as of" stamp on the existing dump.
            let mut t = table().lock().unwrap_or_else(|e| e.into_inner());
            if t.by_norad.is_empty() {
                drop(t);
                load_cached();
                t = table().lock().unwrap_or_else(|e| e.into_inner());
            }
            if t.by_norad.is_empty() {
                t.status.error =
                    Some("SatNOGS not modified, but no transmitters on disk yet".into());
                return t.status.clone();
            }
            t.status.fetched_unix = now;
            t.status.error = None;
            let freqs: Vec<_> = t.by_norad.values().cloned().collect();
            let st = t.status.clone();
            drop(t);
            write_cache(&freqs, now, Validators { fetched_unix: now, ..Validators::default() });
            st
        }
        Ok(FetchResult::Body { text, validators }) => match parse_and_group(&text) {
            Ok(freqs) if !freqs.is_empty() => {
                let mut v = validators;
                v.fetched_unix = now;
                write_cache(&freqs, now, v);
                install(freqs, now, None)
            }
            Ok(_) => keep_existing("SatNOGS returned no usable transmitters".into()),
            Err(e) => keep_existing(e),
        },
        Err(e) => keep_existing(explain_fetch_err(&e)),
    }
}

/// Turn ureq/IO noise into something an operator can act on.
fn explain_fetch_err(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("429") || lower.contains("too many requests") {
        return format!(
            "SatNOGS rate-limited the request (HTTP 429) — wait a bit, then UPDATE NOW again. ({raw})"
        );
    }
    if lower.contains("connection refused")
        || lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("failed to connect")
        || lower.contains("network is unreachable")
    {
        return format!(
            "db.satnogs.org unreachable from this network ({raw}). \
             Not a rate limit — TCP never connected. Try later, another network, or a VPN. \
             Last good transmitters (if any) are still in use."
        );
    }
    format!("db.satnogs.org: {raw}")
}

enum FetchResult {
    Body { text: String, validators: Validators },
    NotModified,
}

fn fetch_all(agent: &ureq::Agent, cache: &mut Cache) -> Result<FetchResult, String> {
    // SatNOGS sometimes returns a bare array for `?limit=…`, sometimes a
    // paginated object. Walk `next` when present; otherwise one shot is enough.
    // Disk is *not* written here — only after a successful parse in [`refresh`].
    let mut url = URL.to_string();
    let mut all: Vec<serde_json::Value> = Vec::new();
    let mut pages = 0;
    loop {
        pages += 1;
        if pages > 40 {
            return Err("too many SatNOGS transmitter pages".into());
        }
        let validators = cache.validators(if pages == 1 { CACHE_KEY } else { &url });
        match crate::feed::http_get(agent, &url, &validators, BODY_LIMIT) {
            Ok(None) => return Ok(FetchResult::NotModified),
            Ok(Some((bytes, v, _))) => {
                let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
                let (batch, next) = split_page(&text)?;
                all.extend(batch);
                match next {
                    Some(n) => url = n,
                    None if pages == 1 => {
                        return Ok(FetchResult::Body { text, validators: v });
                    }
                    None => {
                        let merged = serde_json::Value::Array(all);
                        let text =
                            serde_json::to_string(&merged).map_err(|e| e.to_string())?;
                        return Ok(FetchResult::Body { text, validators: v });
                    }
                }
            }
            Err(e) => return Err(e),
        }
    }
}

fn split_page(text: &str) -> Result<(Vec<serde_json::Value>, Option<String>), String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    match v {
        serde_json::Value::Array(a) => Ok((a, None)),
        serde_json::Value::Object(map) => {
            let results = map
                .get("results")
                .and_then(|r| r.as_array())
                .cloned()
                .ok_or_else(|| "SatNOGS page missing results[]".to_string())?;
            let next = map.get("next").and_then(|n| n.as_str()).map(str::to_string);
            Ok((results, next))
        }
        _ => Err("SatNOGS transmitters: unexpected JSON root".into()),
    }
}

#[derive(Debug, Deserialize)]
struct RawTx {
    #[serde(default)]
    description: String,
    #[serde(default)]
    alive: bool,
    #[serde(default)]
    status: String,
    #[serde(default)]
    unconfirmed: bool,
    #[serde(default)]
    invert: bool,
    mode: Option<String>,
    norad_cat_id: Option<u64>,
    uplink_low: Option<u64>,
    uplink_high: Option<u64>,
    downlink_low: Option<u64>,
    downlink_high: Option<u64>,
}

/// Parse a SatNOGS transmitters body into one [`SatFreqs`] per NORAD id.
pub fn parse_and_group(text: &str) -> Result<Vec<SatFreqs>, String> {
    let (rows, _) = split_page(text)?;
    let mut by: HashMap<u64, SatFreqs> = HashMap::new();
    for row in rows {
        let tx: RawTx = match serde_json::from_value(row) {
            Ok(t) => t,
            Err(_) => continue,
        };
        if !tx.alive || tx.unconfirmed {
            continue;
        }
        let status = tx.status.to_ascii_lowercase();
        if !status.is_empty() && status != "active" {
            continue;
        }
        let Some(norad) = tx.norad_cat_id.filter(|&id| id > 0) else {
            continue;
        };
        let Some(link) = raw_to_link(&tx) else {
            continue;
        };
        let entry = by
            .entry(norad)
            .or_insert_with(|| SatFreqs::new(norad, &format!("NORAD {norad}"), Vec::new()));
        entry.links.push(link);
    }
    let mut out: Vec<_> = by.into_values().collect();
    out.sort_by_key(|f| f.norad_id);
    Ok(out)
}

fn raw_to_link(tx: &RawTx) -> Option<SatLink> {
    let down = passband_hz(tx.downlink_low, tx.downlink_high);
    let up = passband_hz(tx.uplink_low, tx.uplink_high);
    if down.is_none() && up.is_none() {
        return None;
    }
    let label = if tx.description.trim().is_empty() {
        "Transmitter".into()
    } else {
        tx.description.trim().to_string()
    };
    let mode = tx.mode.as_deref().unwrap_or("").trim().to_string();
    let link = SatLink {
        label,
        mode,
        uplink: up,
        downlink: down,
        inverting: tx.invert,
        ..SatLink::default()
    };
    if link.is_empty() {
        return None;
    }
    Some(link)
}

fn passband_hz(lo: Option<u64>, hi: Option<u64>) -> Option<Passband> {
    match (lo, hi) {
        (Some(a), Some(b)) if a > 0 && b > 0 => {
            let lo_mhz = (a.min(b) as f64) / 1_000_000.0;
            let hi_mhz = (a.max(b) as f64) / 1_000_000.0;
            if (hi_mhz - lo_mhz).abs() < 1e-9 {
                Some(Passband::at(lo_mhz))
            } else {
                Some(Passband::range(lo_mhz, hi_mhz))
            }
        }
        (Some(a), None) | (None, Some(a)) if a > 0 => Some(Passband::at((a as f64) / 1_000_000.0)),
        _ => None,
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[
      {
        "description": "FM voice",
        "alive": true,
        "status": "active",
        "unconfirmed": false,
        "invert": false,
        "mode": "FM",
        "norad_cat_id": 25544,
        "uplink_low": 145200000,
        "uplink_high": 145200000,
        "downlink_low": 145800000,
        "downlink_high": 145800000
      },
      {
        "description": "dead bird",
        "alive": false,
        "status": "active",
        "norad_cat_id": 1,
        "downlink_low": 145000000,
        "downlink_high": 145000000
      },
      {
        "description": "CW beacon",
        "alive": true,
        "status": "active",
        "mode": "CW",
        "norad_cat_id": 25544,
        "downlink_low": 145825000,
        "downlink_high": 145825000
      },
      {
        "description": "linear",
        "alive": true,
        "status": "active",
        "invert": true,
        "mode": "USB",
        "norad_cat_id": 44909,
        "uplink_low": 145900000,
        "uplink_high": 146000000,
        "downlink_low": 435800000,
        "downlink_high": 435900000
      }
    ]"#;

    #[test]
    fn parse_keeps_active_links_and_groups_by_norad() {
        let v = parse_and_group(SAMPLE).expect("parse");
        assert_eq!(v.len(), 2);
        let iss = v.iter().find(|f| f.norad_id == 25544).unwrap();
        assert_eq!(iss.links.len(), 2);
        assert_eq!(iss.links[0].downlink, Some(Passband::at(145.8)));
        assert_eq!(iss.links[0].uplink, Some(Passband::at(145.2)));
        let rs = v.iter().find(|f| f.norad_id == 44909).unwrap();
        assert!(rs.links[0].inverting);
        assert_eq!(rs.links[0].uplink, Some(Passband::range(145.9, 146.0)));
    }

    #[test]
    fn paginated_root_is_accepted() {
        let text = r#"{"count":1,"next":null,"results":[{
          "description":"beacon","alive":true,"status":"active",
          "norad_cat_id":99,"downlink_low":137100000,"downlink_high":137100000
        }]}"#;
        let v = parse_and_group(text).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].norad_id, 99);
    }

    #[test]
    fn cache_file_round_trips_parsed_freqs() {
        let freqs = parse_and_group(SAMPLE).unwrap();
        let file = CacheFile { fetched_unix: 1_700_000_000, freqs: freqs.clone() };
        let text = serde_json::to_string(&file).unwrap();
        let (back, when) = read_cache_bytes(&text, 0).unwrap();
        assert_eq!(when, 1_700_000_000);
        assert_eq!(back.len(), freqs.len());
        assert_eq!(back[0].norad_id, freqs[0].norad_id);
    }

    #[test]
    fn failed_refresh_keeps_memory_table() {
        let freqs = parse_and_group(SAMPLE).unwrap();
        install(freqs, 1_700_000_000, None);
        assert!(cached_for(25544).is_some());
        let st = keep_existing("simulated failure".into());
        assert_eq!(st.error.as_deref(), Some("simulated failure"));
        assert!(st.satellites >= 2);
        assert!(cached_for(25544).is_some(), "links must survive a failed refresh");
        assert!(cached_for(44909).is_some());
    }
}
