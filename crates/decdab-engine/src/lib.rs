// made by CasualArclamp
//! The DecDAB engine.
//!
//! - [`rx`]: the receive thread: source in; waterfall rows, levels, the
//!   receiver's state, sound and a recording out.
//! - [`service`]: one service from its sub-channel to sound and PAD.
//! - [`data_service`]: one data component to MOT objects (packet mode).
//! - [`spi`]: the logos and schedules the SPI services have sent.
//! - [`spectrum`]: the averaged spectrum rows, built a few FFTs a block.
//! - [`waterfall`]: the waterfall's thread: its FFTs, and its rows let out
//!   at an even pace.
//! - [`meter`]: CPU meters, whole-process and per stage.
//! - [`delay_doppler`]: the sounder's maps (from DecDVB-T, after DecDRM).
//! - [`frontend`]: DC and I/Q corrections to the radio's samples.

pub mod data_service;
pub mod delay_doppler;
pub mod frontend;
pub mod meter;
pub mod rx;
pub mod service;
pub mod spi;
pub mod spectrum;
pub mod waterfall;

pub use frontend::{FrontEndReading, FrontEndSettings};
pub use meter::{CpuGauge, STAGES, Stage, StageLoad, StageMeter, process_cpu_seconds};
pub use rx::{Command, DabStatus, DabUpdate, DropProbe, Recording, Row, Rx, RxOptions, Slide, Stats};
pub use data_service::DataServiceDecoder;
pub use service::{ServiceDecoder, ServiceStats};
pub use spi::{Logo, Programme, SpiStore};
pub use spectrum::{RowBuilder, Spectrum};
