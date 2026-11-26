//! Opt-in performance probe (`SERICA_PERF=1`): times work done on the main
//! (UI) thread, where every WebView2 callback, IPC call and window change
//! runs, and measures how long the main thread takes to pick up a posted
//! task. Writes one line per second to `%TEMP%\serica-perf.log`. Off by
//! default: `span` is then a single relaxed load.
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static ENABLED: AtomicBool = AtomicBool::new(false);
static STATS: Lazy<Mutex<HashMap<&'static str, Stat>>> = Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Default, Clone, Copy)]
struct Stat {
    n: u64,
    total_us: u64,
    max_us: u64,
}

pub struct Span(&'static str, Instant);

impl Drop for Span {
    fn drop(&mut self) {
        record(self.0, self.1.elapsed());
    }
}

/// Times the enclosing scope under `name` while the probe is on.
#[inline]
pub fn span(name: &'static str) -> Option<Span> {
    ENABLED.load(Ordering::Relaxed).then(|| Span(name, Instant::now()))
}

pub fn record(name: &'static str, d: Duration) {
    let us = d.as_micros() as u64;
    let mut s = STATS.lock();
    let e = s.entry(name).or_default();
    e.n += 1;
    e.total_us += us;
    e.max_us = e.max_us.max(us);
}

/// Counts an event without timing it (e.g. bytes sent).
pub fn count(name: &'static str, amount: u64) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let mut s = STATS.lock();
    let e = s.entry(name).or_default();
    e.n += 1;
    e.total_us += amount;
    e.max_us = e.max_us.max(amount);
}

pub fn start(handle: tauri::AppHandle) {
    if std::env::var_os("SERICA_PERF").is_none() {
        return;
    }
    ENABLED.store(true, Ordering::Relaxed);
    let path = std::env::temp_dir().join("serica-perf.log");
    // Main-thread lag: post a task every 20 ms and time until it runs.
    let h = handle.clone();
    std::thread::spawn(move || loop {
        let posted = Instant::now();
        let (tx, rx) = std::sync::mpsc::channel();
        if h.run_on_main_thread(move || {
            let _ = tx.send(posted.elapsed());
        })
        .is_err()
        {
            return;
        }
        if let Ok(d) = rx.recv_timeout(Duration::from_secs(5)) {
            record("main.lag", d);
        }
        std::thread::sleep(Duration::from_millis(20));
    });
    std::thread::spawn(move || {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).ok();
        let t0 = Instant::now();
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let snap: Vec<(&'static str, Stat)> = {
                let mut s = STATS.lock();
                let v = s.iter().map(|(k, v)| (*k, *v)).collect();
                s.clear();
                v
            };
            let mut v = snap;
            v.sort_by(|a, b| b.1.total_us.cmp(&a.1.total_us));
            let line = v
                .iter()
                .map(|(k, s)| format!("{k}={}x/{}ms/max{}ms", s.n, s.total_us / 1000, s.max_us / 1000))
                .collect::<Vec<_>>()
                .join(" ");
            if let Some(f) = f.as_mut() {
                let mem = crate::app::memory_load_percent().unwrap_or(0);
                let _ = writeln!(f, "t={:.0} mem={mem}% {line}", t0.elapsed().as_secs_f64());
            }
        }
    });
}
