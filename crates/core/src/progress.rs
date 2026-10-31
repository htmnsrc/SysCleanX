//! Lock-free progress + cancellation shared between workers and the UI.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct Progress {
    cancel: AtomicBool,
    files: AtomicU64,
    bytes: AtomicU64,
    done: AtomicUsize,
    total: AtomicUsize,
    label: Mutex<String>,
}

impl Progress {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Relaxed)
    }
    pub fn add(&self, files: u64, bytes: u64) {
        self.files.fetch_add(files, Relaxed);
        self.bytes.fetch_add(bytes, Relaxed);
    }
    pub fn files(&self) -> u64 {
        self.files.load(Relaxed)
    }
    pub fn bytes(&self) -> u64 {
        self.bytes.load(Relaxed)
    }
    pub fn set_total(&self, total: usize) {
        self.total.store(total, Relaxed);
        self.done.store(0, Relaxed);
    }
    pub fn step(&self) {
        self.done.fetch_add(1, Relaxed);
    }
    /// 0.0..=1.0, or `None` when the total is unknown.
    pub fn fraction(&self) -> Option<f32> {
        let total = self.total.load(Relaxed);
        (total > 0).then(|| (self.done.load(Relaxed) as f32 / total as f32).min(1.0))
    }
    pub fn set_label(&self, s: impl Into<String>) {
        if let Ok(mut l) = self.label.lock() {
            *l = s.into();
        }
    }
    pub fn label(&self) -> String {
        self.label.lock().map(|l| l.clone()).unwrap_or_default()
    }
}
