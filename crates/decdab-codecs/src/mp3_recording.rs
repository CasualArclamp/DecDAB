// Selected-service MP3 recording, ported from a contributor patch (OM5NA, issue #2).
//! Streaming MP3 output for one decoded DAB audio service.
//!
//! The encoder (`rusty_mp3`, pure Rust) runs on its own thread: the receiver
//! thread only clones the PCM block into a bounded channel, so a 192 kbit/s
//! encode never adds to the decode loop's CPU, and a slow disk cannot stall
//! the radio. If the encoder falls behind, whole blocks are dropped and
//! counted rather than blocking.
//!
//! Rust note: the worker owns the encoder and the file; `finish` closes the
//! channel (by dropping the sender) and joins the thread, which is what flushes
//! the last frames and hands back any error.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread::JoinHandle;

use rusty_mp3::{Error as Mp3Error, Mp3Encoder, Mp3EncoderConfig};

use crate::Pcm;

/// PCM blocks queued for the encoder (a block is one audio frame, ~24-26 ms).
const QUEUE: usize = 512;
const BITRATE_KBPS: u32 = 192;

/// Writes the selected service's decoded PCM into an MP3 file.
pub struct Mp3Recorder {
    path: PathBuf,
    tx: SyncSender<Pcm>,
    worker: JoinHandle<Result<(), String>>,
    format: Option<(u32, u16)>,
    dropped: u64,
}

impl Mp3Recorder {
    pub fn create(path: impl AsRef<Path>) -> Result<Mp3Recorder, String> {
        let path = path.as_ref().to_path_buf();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let file = File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let (tx, rx) = mpsc::sync_channel::<Pcm>(QUEUE);
        let worker = std::thread::Builder::new()
            .name("mp3-encoder".into())
            .spawn(move || encode(rx, BufWriter::new(file)))
            .map_err(|e| format!("cannot start the MP3 encoder: {e}"))?;
        Ok(Mp3Recorder {
            path,
            tx,
            worker,
            format: None,
            dropped: 0,
        })
    }

    /// Queue a block. An error means the recording cannot go on (a bad
    /// layout, a format change, or the encoder thread has stopped); `finish`
    /// then returns the encoder's own error if it has one.
    pub fn push(&mut self, pcm: &Pcm) -> Result<(), String> {
        if pcm.channels == 0 || !pcm.samples.len().is_multiple_of(pcm.channels as usize) {
            return Err("decoded audio has an invalid channel layout".into());
        }
        match self.format {
            None => self.format = Some((pcm.rate, pcm.channels)),
            Some(f) if f != (pcm.rate, pcm.channels) => {
                return Err(format!(
                    "service audio format changed during MP3 recording ({} Hz/{} ch to {} Hz/{} ch)",
                    f.0, f.1, pcm.rate, pcm.channels
                ));
            }
            Some(_) => {}
        }
        match self.tx.try_send(pcm.clone()) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.dropped += 1;
                Ok(())
            }
            Err(TrySendError::Disconnected(_)) => Err("the MP3 encoder stopped".into()),
        }
    }

    /// Blocks dropped because the encoder fell behind.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Finish the file. With no audio ever pushed the empty file is removed.
    pub fn finish(self) -> Result<(), String> {
        let Mp3Recorder {
            path, tx, worker, ..
        } = self;
        drop(tx);
        let result = worker
            .join()
            .map_err(|_| "the MP3 encoder thread panicked".to_string())?;
        if result == Err(NO_AUDIO.to_string()) {
            let _ = std::fs::remove_file(&path);
        }
        result
    }
}

const NO_AUDIO: &str = "no decoded audio arrived; empty MP3 removed";

fn encode(rx: mpsc::Receiver<Pcm>, mut file: BufWriter<File>) -> Result<(), String> {
    let mut encoder: Option<Mp3Encoder> = None;
    for pcm in rx {
        let enc = encoder.get_or_insert_with(|| {
            Mp3Encoder::new(Mp3EncoderConfig {
                bitrate_kbps: BITRATE_KBPS,
                vbr_quality: None,
            })
        });
        enc.push_pcm_s16(&pcm.samples, pcm.channels, pcm.rate)
            .map_err(|e| format!("MP3 encoder: {e}"))?;
        drain(enc, &mut file)?;
    }
    let Some(mut enc) = encoder else {
        return Err(NO_AUDIO.into());
    };
    enc.finish();
    drain(&mut enc, &mut file)?;
    file.flush().map_err(|e| e.to_string())
}

/// Write every packet the encoder has ready (`Again` = needs more PCM,
/// `Eof` = finished and drained; both just end the loop).
fn drain(enc: &mut Mp3Encoder, file: &mut BufWriter<File>) -> Result<(), String> {
    loop {
        match enc.next_packet() {
            Ok(packet) => file.write_all(&packet).map_err(|e| e.to_string())?,
            Err(Mp3Error::Again | Mp3Error::Eof) => return Ok(()),
            Err(e) => return Err(format!("MP3 encoder: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize) -> Pcm {
        let samples = (0..n * 2)
            .map(|i| ((i / 2) as f32 * 0.05).sin().mul_add(8000.0, 0.0) as i16)
            .collect();
        Pcm {
            rate: 48_000,
            channels: 2,
            samples,
            concealed: false,
        }
    }

    #[test]
    fn writes_a_decodable_file() {
        let path = std::env::temp_dir().join("decdab_mp3_test.mp3");
        let mut r = Mp3Recorder::create(&path).unwrap();
        for _ in 0..100 {
            r.push(&tone(1152)).unwrap();
        }
        r.finish().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(bytes.len() > 10_000, "{} bytes", bytes.len());
        // An MPEG audio frame sync (or an ID3 tag) opens the file.
        assert!(bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0 || bytes.starts_with(b"ID3"));
    }

    #[test]
    fn empty_recording_is_removed() {
        let path = std::env::temp_dir().join("decdab_mp3_empty.mp3");
        let r = Mp3Recorder::create(&path).unwrap();
        assert!(r.finish().is_err());
        assert!(!path.exists());
    }

    #[test]
    fn format_change_is_refused() {
        let path = std::env::temp_dir().join("decdab_mp3_fmt.mp3");
        let mut r = Mp3Recorder::create(&path).unwrap();
        r.push(&tone(1152)).unwrap();
        let mut other = tone(1152);
        other.rate = 24_000;
        assert!(r.push(&other).is_err());
        let _ = r.finish();
        let _ = std::fs::remove_file(&path);
    }
}
