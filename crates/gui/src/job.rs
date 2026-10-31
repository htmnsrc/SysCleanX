//! Background work that never blocks the UI thread.

use eframe::egui;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use syscleanx_core::Progress;

pub struct Job<T> {
    rx: Receiver<T>,
    pub progress: Arc<Progress>,
}

impl<T: Send + 'static> Job<T> {
    pub fn spawn(ctx: &egui::Context, f: impl FnOnce(&Progress) -> T + Send + 'static) -> Self {
        let (tx, rx) = channel();
        let progress = Progress::new();
        let worker_progress = progress.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = f(&worker_progress);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        Job { rx, progress }
    }

    /// `Some` once finished. A panicking worker is reported as an error.
    pub fn poll(&self) -> Option<Result<T, String>> {
        match self.rx.try_recv() {
            Ok(v) => Some(Ok(v)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("the background task stopped unexpectedly".into())),
        }
    }
}

/// Poll an optional job; returns the result and clears the slot when done.
pub fn take<T: Send + 'static>(slot: &mut Option<Job<T>>) -> Option<Result<T, String>> {
    let done = slot.as_ref()?.poll()?;
    *slot = None;
    Some(done)
}
