// made by CasualArclamp
//! Live Airspy R2 front end, in pure Rust over USB.
//!
//! Speaks the Airspy's USB vendor protocol (`airspy_proto`) through `nusb`,
//! the same library the HackRF driver sits on: no libairspy, no libusb, no
//! DLLs. On Windows the Airspy must be bound to the WinUSB driver (Zadig).
//! The requests, their encodings and the gain tables are libairspy's
//! (`airspy.c` in airspyone_host, Copyright (c) 2013-2016 Benjamin Vernoux,
//! Ian Gilmour, Youssef Touil and others; BSD-3-Clause), written out here.
//!
//! **Receive only, antenna power always off.** The bias tee is sent off when
//! the device is opened and at every start, and no code path sends it on: see
//! the note in `airspy_proto`. Nothing in this file builds a command of its
//! own; it only sends what `airspy_proto` makes.
//!
//! The radio streams the R820T's 5 MHz IF as 20 MS/s of real 12-bit samples
//! (40 MB/s). A reader thread keeps [`IN_FLIGHT`] bulk transfers queued and
//! hands the bytes over a bounded channel; if the consumer falls behind,
//! whole buffers are dropped and counted rather than stalling the USB side.
//! The consumer converts them to IQ at 2.048/4.096/8.192 MS/s in
//! [`IqSource::read`] (`airspy_dsp`), and hands the buffers back to the reader
//! to be used again, so nothing is allocated per transfer.

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{JoinHandle, Thread};
use std::time::{Duration, Instant};

use decdab_core::{Error, Iq, Result};
use nusb::transfer::{ControlIn, ControlOut, ControlType, Queue, Recipient, RequestBuffer};

use crate::IqSource;
use crate::airspy_dsp::{IqConverter, RawStream};
use crate::airspy_proto::{
    self as proto, AirspyGains, AirspySettings, CommandData, ENDPOINT_IN, Step, UsbCommand,
};

/// Bulk transfers kept in flight: ~50 ms of samples at 6.5 ms each.
const IN_FLIGHT: usize = 8;
/// Buffers the reader may queue for the consumer before dropping: 64 × 6.5 ms,
/// enough to ride out a GUI hiccup.
const QUEUE: usize = 64;
/// A control transfer that takes longer than this has hung.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(2);
/// How often the reader looks at its stop flag while waiting for data.
const POLL: Duration = Duration::from_millis(100);
/// Polls without a completed transfer before the reader gives up.
const STALL_POLLS: u32 = 20;

fn map_err(e: impl std::fmt::Display) -> Error {
    Error::Other(format!("Airspy: {e}"))
}

/// Wakes a thread parked in [`block_on_timeout`].
struct Unpark(Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// Wait for a future on this thread, for at most `timeout`. Rust note: nusb
/// hands out `Future`s, and a future does nothing until it is polled with a
/// `Waker` that says how to wake the poller; this one unparks the thread.
/// (`futures_lite::block_on`, as the HackRF driver uses, cannot give up.)
/// The future is dropped on a timeout, which cancels the transfer.
fn block_on_timeout<F: Future>(fut: F, timeout: Duration) -> Option<F::Output> {
    let mut fut = std::pin::pin!(fut);
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let deadline = Instant::now() + timeout;
    loop {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return Some(v);
        }
        let left = deadline.checked_duration_since(Instant::now())?;
        std::thread::park_timeout(left);
    }
}

/// The USB device: what the control handle and the reader thread share.
struct Inner {
    // Held so the device stays open as long as the interface is in use.
    _device: nusb::Device,
    interface: nusb::Interface,
    /// Serialises control transfers from the GUI thread and the reader.
    control: Mutex<()>,
}

impl Inner {
    fn open() -> Result<Inner> {
        let info = nusb::list_devices()
            .map_err(|e| map_err(format!("cannot list the USB devices: {e}")))?
            .find(|d| d.vendor_id() == proto::VENDOR_ID && d.product_id() == proto::PRODUCT_ID)
            .ok_or_else(|| {
                Error::Other(
                    "no Airspy found. Is it plugged in, not in use by another program, \
                     and on the WinUSB driver (Zadig)?"
                        .into(),
                )
            })?;
        let device = info.open().map_err(|e| {
            Error::Other(format!(
                "cannot open the Airspy ({e}). Is it in use by another program, \
                 and on the WinUSB driver?"
            ))
        })?;
        // Detaches a kernel driver on Linux (there is none for an Airspy;
        // harmless), and is a plain claim elsewhere.
        let interface = device.detach_and_claim_interface(0).map_err(map_err)?;
        Ok(Inner {
            _device: device,
            interface,
            control: Mutex::new(()),
        })
    }

    /// Send one command, returning what an IN transfer read.
    fn send(&self, cmd: &UsbCommand) -> Result<Vec<u8>> {
        let _one_at_a_time = self.control.lock().unwrap();
        let timed_out = || map_err(format!("request {} timed out", cmd.request));
        match &cmd.data {
            CommandData::Out(data) => {
                let done = block_on_timeout(
                    self.interface.control_out(ControlOut {
                        control_type: ControlType::Vendor,
                        recipient: Recipient::Device,
                        request: cmd.request,
                        value: cmd.value,
                        index: cmd.index,
                        data,
                    }),
                    CONTROL_TIMEOUT,
                )
                .ok_or_else(timed_out)?
                .into_result()
                .map_err(map_err)?;
                if done.actual_length() != data.len() {
                    return Err(map_err(format!("request {} was cut short", cmd.request)));
                }
                Ok(Vec::new())
            }
            CommandData::In(len) => {
                let reply = block_on_timeout(
                    self.interface.control_in(ControlIn {
                        control_type: ControlType::Vendor,
                        recipient: Recipient::Device,
                        request: cmd.request,
                        value: cmd.value,
                        index: cmd.index,
                        length: *len,
                    }),
                    CONTROL_TIMEOUT,
                )
                .ok_or_else(timed_out)?
                .into_result()
                .map_err(map_err)?;
                Ok(reply)
            }
        }
    }

    /// A "set" call: an IN transfer whose one byte is ignored but whose
    /// length is checked (libairspy treats anything else as an error).
    fn set(&self, cmd: &UsbCommand) -> Result<()> {
        let n = self.send(cmd)?.len();
        let want = match cmd.data {
            CommandData::In(l) => l as usize,
            CommandData::Out(_) => 0,
        };
        if n == want {
            Ok(())
        } else {
            Err(map_err(format!("short reply to request {}", cmd.request)))
        }
    }

    fn run(&self, steps: &[Step]) -> Result<()> {
        for step in steps {
            match step {
                Step::Command(c) => self.set(c)?,
                Step::ClearHalt => self
                    .interface
                    .clear_halt(ENDPOINT_IN)
                    .map_err(|e| map_err(format!("clear halt: {e}")))?,
            }
        }
        Ok(())
    }

    /// The rates the device offers, Hz.
    fn sample_rates(&self) -> Result<Vec<u32>> {
        let count = proto::parse_sample_rate_count(&self.send(&proto::read_sample_rate_count())?)?;
        proto::parse_sample_rates(&self.send(&proto::read_sample_rates(count))?, count)
    }

    fn stop_rx(&self) -> Result<()> {
        self.set(&proto::receiver_mode(false))
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        // The last owner: make sure the radio is not left streaming.
        let _ = self.stop_rx();
    }
}

/// What the board says about itself.
#[derive(Debug, Clone)]
pub struct AirspyInfo {
    pub board_id: u8,
    pub board_name: &'static str,
    pub firmware: String,
    /// The sample rates it offers, Hz (the R2: 10 000 000 and 2 500 000).
    pub sample_rates: Vec<u32>,
}

/// Open the first Airspy, read its board id, firmware string and sample
/// rates, and close it again. It is not started.
pub fn airspy_info() -> Result<AirspyInfo> {
    let dev = Inner::open()?;
    let id = *dev
        .send(&proto::read_board_id())?
        .first()
        .ok_or_else(|| map_err("no board id"))?;
    let firmware = proto::parse_version(&dev.send(&proto::read_version())?);
    Ok(AirspyInfo {
        board_id: id,
        board_name: proto::board_name(id),
        firmware,
        sample_rates: dev.sample_rates()?,
    })
}

/// Live controls on a running Airspy, usable from the GUI thread while the
/// engine reads samples.
#[derive(Clone)]
pub struct AirspyControl {
    dev: Arc<Inner>,
    settings: Arc<Mutex<AirspySettings>>,
    overflows: Arc<AtomicU64>,
}

impl AirspyControl {
    pub fn settings(&self) -> AirspySettings {
        *self.settings.lock().unwrap()
    }

    /// Retune. Takes effect within a USB transfer or two (see
    /// [`AirspySource::retune`] for what is thrown away).
    pub fn set_center(&self, hz: u64) -> Result<()> {
        self.dev.set(&proto::set_freq(hz)?)?;
        self.settings.lock().unwrap().center_hz = hz;
        Ok(())
    }

    /// The three gain stages (clamped to the hardware's ranges).
    pub fn set_gains(&self, gains: AirspyGains) -> Result<()> {
        let g = gains.clamped();
        for c in proto::set_gains(g) {
            self.dev.set(&c)?;
        }
        self.settings.lock().unwrap().gains = g;
        Ok(())
    }

    /// Buffers dropped because the consumer fell behind.
    pub fn overflows(&self) -> u64 {
        self.overflows.load(Ordering::Relaxed)
    }
}

/// A live Airspy receive stream.
pub struct AirspySource {
    stream: RawStream,
    control: AirspyControl,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// Wait for the queue's next transfer to complete, for at most `timeout`.
/// `Queue::poll_next` panics with nothing pending, so there must be.
fn next_complete(
    queue: &mut Queue<RequestBuffer>,
    timeout: Duration,
) -> Option<nusb::transfer::Completion<Vec<u8>>> {
    block_on_timeout(queue.next_complete(), timeout)
}

/// The reader thread: keeps the transfers in flight and hands each completed
/// one on.
fn reader(
    dev: Arc<Inner>,
    tx: SyncSender<Vec<u8>>,
    recycled: Receiver<Vec<u8>>,
    stop: Arc<AtomicBool>,
    overflows: Arc<AtomicU64>,
    transfer: usize,
) {
    let mut queue = dev.interface.bulk_in_queue(ENDPOINT_IN);
    // Buffers that were dropped for want of room in the channel come back
    // here.
    let mut spare: Vec<Vec<u8>> = Vec::new();
    for _ in 0..IN_FLIGHT {
        queue.submit(RequestBuffer::new(transfer));
    }
    let mut idle = 0;
    while !stop.load(Ordering::Relaxed) {
        let Some(done) = next_complete(&mut queue, POLL) else {
            idle += 1;
            if idle >= STALL_POLLS {
                tracing::warn!("Airspy delivered nothing for {:?}", POLL * STALL_POLLS);
                break;
            }
            continue;
        };
        idle = 0;
        if let Err(e) = done.status {
            tracing::warn!("Airspy read failed: {e}");
            break;
        }
        // The next transfer goes in before this one is handed on, so the radio
        // never waits.
        let fresh = recycled.try_recv().ok().or_else(|| spare.pop());
        queue.submit(RequestBuffer::reuse(fresh.unwrap_or_default(), transfer));
        match tx.try_send(done.data) {
            Ok(()) => {}
            Err(TrySendError::Full(b)) => {
                overflows.fetch_add(1, Ordering::Relaxed);
                spare.push(b);
            }
            Err(TrySendError::Disconnected(_)) => break,
        }
    }
    // Cancelled transfers still come back through the queue.
    queue.cancel_all();
    while queue.pending() > 0 {
        if next_complete(&mut queue, Duration::from_millis(500)).is_none() {
            break;
        }
    }
    let _ = dev.stop_rx();
}

impl AirspySource {
    /// Open the first Airspy, configure it and start receiving.
    pub fn open(settings: AirspySettings) -> Result<Self> {
        // Everything that can be wrong without a radio is checked first.
        let converter = IqConverter::new(settings.sample_rate, settings.packed)?;
        proto::start_sequence(&settings, 0)?;
        let gains = settings.gains.clamped();

        let dev = Arc::new(Inner::open()?);
        let rates = dev.sample_rates()?;
        let steps = proto::start_sequence(&settings, proto::iq_rate_index(&rates)?)?;
        dev.run(&steps)?;

        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(QUEUE);
        let (recycle_tx, recycle_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let overflows = Arc::new(AtomicU64::new(0));
        let transfer = settings.transfer_bytes();
        let thread = {
            let (dev, stop, overflows) =
                (Arc::clone(&dev), Arc::clone(&stop), Arc::clone(&overflows));
            std::thread::Builder::new()
                .name("airspy-rx".into())
                .spawn(move || reader(dev, tx, recycle_rx, stop, overflows, transfer))
                .map_err(|e| Error::Other(format!("cannot start the Airspy reader: {e}")))?
        };

        Ok(AirspySource {
            stream: RawStream::new(rx, recycle_tx, converter, "Airspy"),
            control: AirspyControl {
                dev,
                settings: Arc::new(Mutex::new(AirspySettings { gains, ..settings })),
                overflows,
            },
            stop,
            thread: Some(thread),
        })
    }

    /// A handle for retuning and gain changes while the engine reads.
    pub fn control(&self) -> AirspyControl {
        self.control.clone()
    }
}

impl IqSource for AirspySource {
    fn read(&mut self, out: &mut Vec<Iq>) -> Result<usize> {
        self.stream.read(out)
    }

    fn sample_rate(&self) -> f64 {
        self.stream.sample_rate()
    }

    fn center_freq(&self) -> f64 {
        self.control.settings().center_hz as f64
    }

    fn describe(&self) -> String {
        let s = self.control.settings();
        format!(
            "Airspy @ {:.4} MHz, {:.3} MS/s, LNA {} mixer {} VGA {}",
            s.center_hz as f64 / 1e6,
            s.sample_rate as f64 / 1e6,
            s.gains.lna,
            s.gains.mixer,
            s.gains.vga
        )
    }

    /// Moves the centre and throws away what was captured before it took
    /// effect: the buffers waiting (up to 64, most of half a second if the
    /// consumer was behind) and the transfers the radio was filling as it
    /// retuned (eight in flight; one more for the loop's own settling). The
    /// converter's filters start afresh too. A receiver handed the old
    /// channel's samples as the new one's can lock on them, and stay locked
    /// on a signal that has gone.
    fn retune(&mut self, hz: f64) -> Result<bool> {
        self.control.set_center(hz.round() as u64)?;
        self.stream.restart(IN_FLIGHT as u32 + 1);
        Ok(true)
    }

    fn is_live(&self) -> bool {
        true
    }
}

impl Drop for AirspySource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // The reader polls the flag every 100 ms at most; it cancels its
        // transfers and switches the radio off on the way out.
        self.stream.drain();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = self.control.dev.stop_rx();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_what_is_wrong_before_any_usb_access() {
        for s in [
            AirspySettings {
                sample_rate: 3_000_000,
                ..Default::default()
            },
            AirspySettings {
                center_hz: 5_000_000,
                ..Default::default()
            },
        ] {
            // Checked before the device is opened, so this needs no radio.
            let e = AirspySource::open(s).err().expect("must fail").to_string();
            assert!(!e.contains("no Airspy"), "{e}");
        }
    }

    #[test]
    fn a_future_that_never_finishes_times_out() {
        let t0 = Instant::now();
        assert!(
            block_on_timeout(std::future::pending::<()>(), Duration::from_millis(50)).is_none()
        );
        assert!(t0.elapsed() < Duration::from_secs(1));
        assert_eq!(
            block_on_timeout(std::future::ready(7), Duration::from_secs(1)),
            Some(7)
        );
    }

    /// Live: `cargo test -p decdab-io --release --features airspy live_airspy -- --ignored --nocapture`.
    /// Receive only, antenna power off. Streams 3 s at 12B and reports the
    /// rate, the level and the buffers dropped; then retunes to 11D and back.
    #[test]
    #[ignore = "needs an Airspy attached"]
    fn live_airspy() {
        let info = airspy_info().expect("info");
        eprintln!("{info:?}");
        let mut src = AirspySource::open(AirspySettings::default()).expect("open");
        let mut buf = Vec::new();
        let mut n = 0usize;
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(3) {
            n += src.read(&mut buf).expect("read");
        }
        let rate = n as f64 / t0.elapsed().as_secs_f64();
        let rms = (buf.iter().map(|s| s.norm_sqr()).sum::<f32>() / buf.len() as f32).sqrt();
        eprintln!(
            "{} — {:.3} MS/s delivered, rms {rms:.4}, overflows {}",
            src.describe(),
            rate / 1e6,
            src.control().overflows()
        );
        assert!(rate > 3.9e6, "only {:.3} MS/s", rate / 1e6);
        assert!(src.retune(222_064_000.0).expect("retune"));
        src.read(&mut buf).expect("read after retune");
        assert_eq!(src.control().settings().center_hz, 222_064_000);
    }
}
