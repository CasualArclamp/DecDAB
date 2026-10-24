// made by CasualArclamp
//! `--record-video FILE`: the app records itself, for documentation and
//! demos. Each frame drawn is taken through egui's own screenshot path (the
//! picture as rendered, whatever covers the window on the screen) and piped
//! raw to ffmpeg, which must be on the PATH. A test run like `--screenshot`:
//! no sound, nothing kept in the prefs, and the app closes when done.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use eframe::egui;

pub struct Recorder {
    path: PathBuf,
    /// Seconds after the start to begin, and how long to record.
    from: f64,
    secs: f64,
    fps: f64,
    pipe: Option<(Child, ChildStdin, [usize; 2])>,
    frames: u64,
    pub done: bool,
}

impl Recorder {
    pub fn new(path: PathBuf, from: f64, secs: f64, fps: f64) -> Recorder {
        Recorder {
            path,
            from,
            secs,
            fps,
            pipe: None,
            frames: 0,
            done: false,
        }
    }

    /// Once a frame: ask for this frame's picture, and pass on the ones that
    /// have come. True when the recording has just finished.
    pub fn tick(&mut self, ctx: &egui::Context, started: Instant) -> bool {
        if self.done || started.elapsed() < Duration::from_secs_f64(self.from) {
            return false;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        let images: Vec<_> = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
                .collect()
        });
        for img in images {
            let size = img.size;
            if self.pipe.is_none() {
                match Command::new("ffmpeg")
                    .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo"])
                    .args(["-pix_fmt", "rgba", "-s", &format!("{}x{}", size[0], size[1])])
                    .args(["-r", &format!("{}", self.fps), "-i", "-"])
                    .args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "14"])
                    .args(["-pix_fmt", "yuv420p", "-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2"])
                    .arg(&self.path)
                    .stdin(Stdio::piped())
                    .spawn()
                {
                    Ok(mut child) => {
                        let stdin = child.stdin.take().expect("piped");
                        self.pipe = Some((child, stdin, size));
                    }
                    Err(e) => {
                        eprintln!("cannot start ffmpeg for --record-video: {e}");
                        self.done = true;
                        return true;
                    }
                }
            }
            let Some((_, stdin, first)) = &mut self.pipe else {
                continue;
            };
            // A window resized mid-recording: the frames that do not fit are
            // left out rather than garbling the stream.
            if size != *first {
                continue;
            }
            let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            if stdin.write_all(&bytes).is_err() {
                self.done = true;
                return true;
            }
            self.frames += 1;
        }
        if self.frames as f64 >= self.secs * self.fps {
            self.finish();
            return true;
        }
        false
    }

    fn finish(&mut self) {
        self.done = true;
        if let Some((mut child, stdin, _)) = self.pipe.take() {
            drop(stdin); // end of input: ffmpeg finishes the file
            let _ = child.wait();
            eprintln!(
                "recorded {} frames to {}",
                self.frames,
                self.path.display()
            );
        }
    }
}
