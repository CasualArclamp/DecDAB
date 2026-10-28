// Window snapshot, ported from a contributor patch (OM5NA, issue #2).
//! A PNG of the live window on demand (F12 or the camera button). The
//! `--screenshot` option is the unattended version of the same thing; this
//! one tags its request so the two never take each other's picture.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;

const TAG: &str = "decdab-snapshot";
const WAIT: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct Snapshot {
    pending: Option<(PathBuf, Instant)>,
}

impl Snapshot {
    pub fn request(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.pending = Some((path, Instant::now()));
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(TAG)));
        ctx.request_repaint();
    }

    /// Call every frame: `Some` once the picture is saved (or failed).
    pub fn poll(&mut self, ctx: &egui::Context) -> Option<Result<PathBuf, String>> {
        let (_, asked) = self.pending.as_ref()?;
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                // Rust note: `UserData` holds an `Arc<dyn Any>`; we get our
                // tag back by downcasting to the type we put in.
                egui::Event::Screenshot {
                    image, user_data, ..
                } if user_data
                    .data
                    .as_ref()
                    .and_then(|d| d.downcast_ref::<&'static str>())
                    .is_some_and(|t| *t == TAG) =>
                {
                    Some(image.clone())
                }
                _ => None,
            })
        });
        let Some(image) = image else {
            if asked.elapsed() > WAIT {
                self.pending = None;
                return Some(Err("the window did not return a screenshot".into()));
            }
            return None;
        };
        let (path, _) = self.pending.take()?;
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
        let (w, h) = (image.size[0] as u32, image.size[1] as u32);
        Some(
            image::save_buffer(&path, &rgba, w, h, image::ColorType::Rgba8)
                .map(|()| path.clone())
                .map_err(|e| format!("{}: {e}", path.display())),
        )
    }
}
