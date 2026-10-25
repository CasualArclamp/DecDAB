// made by CasualArclamp
//! Station logos as textures: decoded from the SPI store's PNGs and JPEGs
//! the first time each is drawn, then kept.
//!
//! Logos matched to a service are also saved to `logos\` in the app's
//! folder, named by SId and size (`C1C0_128x128.png`), and a service with no
//! logo from the air yet is drawn from there. An SPI carousel can take half
//! an hour to come round (Arqiva's on 12C, at 8 kbit/s), and it may carry
//! other ensembles' logos too, so they are there next time, on any
//! multiplex.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use decdab_engine::SpiStore;
use eframe::egui::{self, TextureHandle};

/// A logo kept on disk: its size and file.
#[derive(Clone)]
struct Cached {
    width: u32,
    height: u32,
    path: PathBuf,
}

#[derive(Default)]
pub struct Logos {
    pub spi: Arc<SpiStore>,
    /// By the logo's object name (or file) and size: a texture, or `None`
    /// for one that would not decode (not tried again).
    tex: HashMap<(String, u32), Option<TextureHandle>>,
    /// The cache on disk, by SId, read the first time it is needed; the
    /// files written this run.
    disk: Option<HashMap<u32, Vec<Cached>>>,
    written: HashSet<String>,
}

fn cache_dir() -> Option<PathBuf> {
    Some(crate::prefs::dir()?.join("logos"))
}

/// "C1C0_128x128.png" → (0xC1C0, 128, 128).
fn parse_name(name: &str) -> Option<(u32, u32, u32)> {
    let stem = name.rsplit_once('.')?.0;
    let (sid, size) = stem.split_once('_')?;
    let (w, h) = size.split_once('x')?;
    Some((u32::from_str_radix(sid, 16).ok()?, w.parse().ok()?, h.parse().ok()?))
}

/// The logo to draw `width` wide among `sizes`: square or 4:3 ones first
/// (the 112 × 32 banner is for one-line displays), the smallest at least
/// that wide, else the largest.
fn pick(sizes: &[(u32, u32)], width: u32) -> Option<usize> {
    let shaped = |&(w, h): &(u32, u32)| w <= h * 2;
    let any_shaped = sizes.iter().any(shaped);
    let pool: Vec<usize> = (0..sizes.len())
        .filter(|&i| !any_shaped || shaped(&sizes[i]))
        .collect();
    pool.iter()
        .copied()
        .filter(|&i| sizes[i].0 >= width)
        .min_by_key(|&i| sizes[i].0)
        .or_else(|| pool.iter().copied().max_by_key(|&i| sizes[i].0))
}

impl Logos {
    /// A new delivery from the engine: logos matched to services go to disk.
    pub fn update(&mut self, spi: Arc<SpiStore>) {
        self.spi = spi;
        let Some(dir) = cache_dir() else { return };
        // Rust note: a second handle on the store, so the loop's borrow of
        // it does not lock `self` (the index below is changed as it goes).
        let spi = Arc::clone(&self.spi);
        for (sid, logos) in spi.matched() {
            for l in logos {
                let ext = if l.mime == "image/png" { "png" } else { "jpg" };
                let name = format!("{sid:04X}_{}x{}.{ext}", l.width, l.height);
                if !self.written.insert(name.clone()) {
                    continue;
                }
                let path = dir.join(&name);
                // A cache that cannot be written is only a slower start.
                if std::fs::create_dir_all(&dir).is_ok() && std::fs::write(&path, l.data.as_slice()).is_ok() {
                    let list = self.disk_index().entry(sid).or_default();
                    list.retain(|c| c.path != path);
                    list.push(Cached {
                        width: l.width,
                        height: l.height,
                        path,
                    });
                }
            }
        }
    }

    /// Forget the last ensemble's (the cache on disk stays).
    pub fn clear(&mut self) {
        self.spi = Arc::default();
        self.tex.clear();
    }

    fn disk_index(&mut self) -> &mut HashMap<u32, Vec<Cached>> {
        self.disk.get_or_insert_with(|| {
            let mut m: HashMap<u32, Vec<Cached>> = HashMap::new();
            let files = cache_dir().and_then(|d| std::fs::read_dir(d).ok());
            for e in files.into_iter().flatten().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if let Some((sid, width, height)) = parse_name(&name) {
                    m.entry(sid).or_default().push(Cached {
                        width,
                        height,
                        path: e.path(),
                    });
                }
            }
            m
        })
    }

    /// The service's logo nearest `width` pixels wide: from the air, or else
    /// from the cache.
    pub fn get(&mut self, ctx: &egui::Context, sid: u32, width: u32) -> Option<TextureHandle> {
        let (key, bytes) = if let Some(logo) = self.spi.logo(sid, width) {
            ((logo.name.clone(), logo.width), None)
        } else {
            let list = self.disk_index().get(&sid)?.clone();
            let sizes: Vec<(u32, u32)> = list.iter().map(|c| (c.width, c.height)).collect();
            let c = &list[pick(&sizes, width)?];
            ((c.path.to_string_lossy().into_owned(), c.width), Some(c.path.clone()))
        };
        if let Some(t) = self.tex.get(&key) {
            return t.clone();
        }
        let data: Option<Vec<u8>> = match bytes {
            Some(path) => std::fs::read(path).ok(),
            None => self.spi.logo(sid, width).map(|l| l.data.to_vec()),
        };
        let tex = data.and_then(|d| {
            let img = image::load_from_memory(&d).ok()?.to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let ci = egui::ColorImage::from_rgba_unmultiplied(size, &img);
            Some(ctx.load_texture(format!("logo {}", key.0), ci, egui::TextureOptions::LINEAR))
        });
        self.tex.insert(key, tex.clone());
        tex
    }
}

/// Draw a texture fitted into a `w` × `h` box, its aspect kept.
pub fn fit(ui: &mut egui::Ui, tex: &TextureHandle, w: f32, h: f32) -> egui::Response {
    let [tw, th] = tex.size();
    let s = (w / tw.max(1) as f32).min(h / th.max(1) as f32);
    let size = egui::vec2(tw as f32 * s, th as f32 * s);
    ui.allocate_ui(egui::vec2(w, h), |ui| {
        ui.centered_and_justified(|ui| ui.add(egui::Image::new((tex.id(), size))))
            .inner
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_names() {
        assert_eq!(parse_name("C1C0_128x128.png"), Some((0xC1C0, 128, 128)));
        assert_eq!(parse_name("E1C000D7_320x240.jpg"), Some((0xE1C0_00D7, 320, 240)));
        assert_eq!(parse_name("notes.txt"), None);
    }

    #[test]
    fn picks_square_at_least_as_wide() {
        let sizes = [(112, 32), (32, 32), (128, 128), (320, 240)];
        assert_eq!(pick(&sizes, 20), Some(1));
        assert_eq!(pick(&sizes, 100), Some(2));
        assert_eq!(pick(&sizes, 600), Some(3));
        assert_eq!(pick(&[(112, 32)], 32), Some(0));
    }
}
