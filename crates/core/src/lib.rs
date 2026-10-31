//! SysCleanX core: cleaning engine and Windows system integration.
//!
//! Everything that touches the system lives here so the GUI and CLI stay thin.
//! Windows-only code sits behind `cfg(windows)`; the rest is tested on any OS.

pub mod bloat;
pub mod broken;
pub mod diskusage;
pub mod duplicates;
pub mod engine;
pub mod fsops;
pub mod guard;
pub mod log;
pub mod paths;
pub mod progress;
#[cfg(windows)]
pub mod reg;
pub mod resolve;
pub mod settings;
pub mod special;
pub mod startup;
pub mod sys;
pub mod targets;
pub mod uninstall;
pub mod util;

pub use engine::{CleanReport, Engine, Options, TargetScan};
pub use progress::Progress;
pub use util::format_bytes;
