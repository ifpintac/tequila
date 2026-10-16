//! Opt-in timing of the viewport pipeline.
//!
//! Set `TEQUILA_PROFILE` to a file path and every stage wrapped in [`time`]
//! is measured; each second a line per stage is appended to the file — count,
//! mean and worst, in milliseconds. Off, it costs one relaxed atomic load per
//! call, which is why it can stay in the code rather than being added and
//! removed each time the viewport feels slow.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::Instant;

static STATE: AtomicU8 = AtomicU8::new(0); // 0 unknown, 1 off, 2 on

struct Stats {
    stages: BTreeMap<&'static str, (u32, f64, f64)>,
    since: Instant,
    path: std::path::PathBuf,
}

static STATS: Mutex<Option<Stats>> = Mutex::new(None);

fn enabled() -> bool {
    match STATE.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let path = std::env::var_os("TEQUILA_PROFILE").map(std::path::PathBuf::from);
            let on = path.is_some();
            if let Some(path) = path {
                *STATS.lock().unwrap() = Some(Stats { stages: BTreeMap::new(), since: Instant::now(), path });
            }
            STATE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// Run `f`, and when profiling, record how long it took under `stage`.
pub fn time<T>(stage: &'static str, f: impl FnOnce() -> T) -> T {
    if !enabled() {
        return f();
    }
    let started = Instant::now();
    let out = f();
    record(stage, started.elapsed().as_secs_f64() * 1000.0);
    out
}

pub fn record(stage: &'static str, ms: f64) {
    if !enabled() {
        return;
    }
    let mut guard = STATS.lock().unwrap();
    let Some(stats) = guard.as_mut() else { return };
    let entry = stats.stages.entry(stage).or_insert((0, 0.0, 0.0));
    entry.0 += 1;
    entry.1 += ms;
    entry.2 = entry.2.max(ms);
    if stats.since.elapsed().as_secs_f64() >= 1.0 {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&stats.path) {
            let _ = writeln!(file, "--- {:.0} ms window", stats.since.elapsed().as_secs_f64() * 1000.0);
            for (name, (count, total, worst)) in &stats.stages {
                let _ = writeln!(file, "{name:<24} n={count:<5} mean={:>7.2} max={worst:>7.2}", total / *count as f64);
            }
        }
        stats.stages.clear();
        stats.since = Instant::now();
    }
}
