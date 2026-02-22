use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Debug, Default)]
pub struct Metrics {
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    stale_served: AtomicU64,
    api_requests: AtomicU64,
    retries: AtomicU64,
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inc_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_stale_served(&self) {
        self.stale_served.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_api_request(&self) {
        self.api_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_retry(&self) {
        self.retries.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> (u64, u64, u64, u64, u64) {
        (
            self.cache_hits.load(Ordering::Relaxed),
            self.cache_misses.load(Ordering::Relaxed),
            self.stale_served.load(Ordering::Relaxed),
            self.api_requests.load(Ordering::Relaxed),
            self.retries.load(Ordering::Relaxed),
        )
    }
}

pub fn spawn_metrics_logger(metrics: Arc<Metrics>, interval: Duration) {
    thread::spawn(move || loop {
        thread::sleep(interval);
        let (hits, misses, stale, api, retries) = metrics.snapshot();
        eprintln!(
            "metrics cache_hit={} cache_miss={} stale_served={} api_requests={} retries={}",
            hits, misses, stale, api, retries
        );
    });
}
