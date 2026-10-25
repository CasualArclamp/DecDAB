// made by CasualArclamp
//! The DecDAB engine.
//!
//! - [`rx`]: the receive thread: source in; waterfall rows, levels, the
//!   receiver's state, sound and a recording out.
//! - [`service`]: one service from its sub-channel to sound and PAD.
//! - [`spectrum`]: the averaged spectrum rows, built a few FFTs a block.
//! - [`waterfall`]: the waterfall's thread: its FFTs, and its rows let out
//!   at an even pace.
//! - [`meter`]: CPU meters, whole-process and per stage.
//! - [`frontend`]: DC and I/Q corrections to the radio's samples.

pub mod frontend;
pub mod meter;
pub mod rx;
pub mod service;
pub mod spectrum;
pub mod waterfall;

pub use frontend::{FrontEndReading, FrontEndSettings};
pub use meter::{CpuGauge, STAGES, Stage, StageLoad, StageMeter, process_cpu_seconds};
pub use rx::{Command, DabStatus, DabUpdate, DropProbe, Recording, Row, Rx, RxOptions, Slide, Stats};
pub use service::{ServiceDecoder, ServiceStats};
pub use spectrum::{RowBuilder, Spectrum};
