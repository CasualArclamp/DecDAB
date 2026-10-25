// made by CasualArclamp
//! Station logos as textures: decoded from the SPI store's PNGs and JPEGs
//! the first time each is drawn, then kept.

use std::collections::HashMap;
use std::sync::Arc;

use decdab_engine::SpiStore;
use eframe::egui::{self, TextureHandle};

#[derive(Default)]
pub struct Logos {
    pub spi: Arc<SpiStore>,
    /// By the logo's object name and size: a texture, or `None` for one
    /// that would not decode (not tried again).
    tex: HashMap<(String, u32), Option<TextureHandle>>,
}

impl Logos {
    /// A new delivery from the engine.
    pub fn update(&mut self, spi: Arc<SpiStore>) {
        self.spi = spi;
    }

    /// Forget everything (another ensemble).
    pub fn clear(&mut self) {
        *self = Logos::default();
    }

    /// The service's logo nearest `width` pixels wide.
    pub fn get(&mut self, ctx: &egui::Context, sid: u32, width: u32) -> Option<TextureHandle> {
        let logo = self.spi.logo(sid, width)?;
        let key = (logo.name.clone(), logo.width);
        self.tex
            .entry(key)
            .or_insert_with(|| {
                let img = image::load_from_memory(&logo.data).ok()?.to_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                let ci = egui::ColorImage::from_rgba_unmultiplied(size, &img);
                Some(ctx.load_texture(format!("logo {}", logo.name), ci, egui::TextureOptions::LINEAR))
            })
            .clone()
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
