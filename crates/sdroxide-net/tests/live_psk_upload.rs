//! Live end-to-end check that a reception report reaches the PSK Reporter
//! collector. Ignored by default (needs network); run with:
//!   cargo test -p sdroxide-net --test live_psk_upload -- --ignored --nocapture

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sdroxide_net::{NetEvent, PskReport, SpotManager};
use sdroxide_types::NetworkConfig;

#[test]
#[ignore = "sends a real UDP report to report.pskreporter.info"]
fn a_queued_report_uploads_to_the_collector() {
    let mut mgr = SpotManager::new();
    let mut cfg = NetworkConfig::default();
    cfg.psk.report = true;
    mgr.set_config(cfg);
    mgr.set_operator("XV4HT", "OL20vx");

    let when = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0);
    mgr.psk_report(PskReport {
        call: "W1AW".into(),
        grid: "FN31".into(),
        freq_hz: 14_074_090,
        snr_db: -12,
        mode: "FT8".into(),
        when_utc: when,
    });
    // Let the worker dequeue before we tear it down.
    std::thread::sleep(Duration::from_millis(200));

    // Disabling report drops the upload handle, which flushes pending reports.
    let mut off = NetworkConfig::default();
    off.psk.report = false;
    mgr.set_config(off);
    std::thread::sleep(Duration::from_millis(500));

    let mut status = None;
    for ev in mgr.poll() {
        if let NetEvent::Status(s) = ev {
            status = s;
        }
    }
    println!("PSK upload status: {status:?}");
    assert!(
        status.as_deref().is_some_and(|s| s.contains("uploaded")),
        "expected a successful PSK Reporter upload status, got {status:?}"
    );
}
