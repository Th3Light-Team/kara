//! A cancel that lands while one `readdir` page is on its way is honoured at
//! once, not after the page arrives (mutation S13 survived the per-page check).

mod support;

use std::fs;
use std::io;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use kara_vfs::{Backend, BackendErrorKind, Cancel};
use support::{Fixture, rp};

#[test]
fn a_cancel_during_a_slow_page_returns_without_waiting_for_it() -> io::Result<()> {
    let fixture = Fixture::standard()?;
    let dir = fixture.server.root().join("few");
    fs::create_dir(&dir)?;
    for i in 0..5 {
        fs::write(dir.join(format!("f{i}")), b"")?;
    }
    // Every request takes 2 s: opendir answers at ~2 s, the first readdir at ~4 s.
    fixture.server.faults.delay_ms.store(2000, Ordering::SeqCst);
    let cancel = Cancel::new();
    let trigger = cancel.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(2700));
        trigger.cancel();
        Instant::now()
    });
    let outcome = fixture.backend.list(&rp("/few")?, &cancel);
    let returned = Instant::now();
    let cancelled_at = canceller.join().map_err(|_| io::Error::other("join"))?;
    let error = outcome.err().ok_or_else(|| io::Error::other("the listing completed"))?;
    assert_eq!(error.kind, BackendErrorKind::Cancelled);
    let late = returned.saturating_duration_since(cancelled_at);
    assert!(late < Duration::from_millis(600), "returned {late:?} after the cancel");
    Ok(())
}
