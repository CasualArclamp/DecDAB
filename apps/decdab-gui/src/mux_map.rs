// made by CasualArclamp
//! Where a service lives in the multiplex: the CIF's 864 capacity units as
//! a bar under the card, each sub-channel a segment in its service's colour
//! and the playing one lit (EN 300 401 §6.2: a sub-channel is a run of
//! capacity units, start and size from FIG 0/1).

use std::collections::BTreeMap;

use decdab_fic::{Ensemble, Transport};
use eframe::egui::{Color32, CornerRadius, Rect, RichText, Sense, Ui, pos2, vec2};

/// Capacity units in a CIF.
const CUS: usize = 864;

const PLAYING: Color32 = Color32::from_rgb(110, 220, 130);
const DABPLUS: Color32 = Color32::from_rgb(70, 120, 170);
const DAB: Color32 = Color32::from_rgb(160, 130, 60);
const DATA: Color32 = Color32::from_rgb(90, 90, 100);
const EMPTY: Color32 = Color32::from_rgb(18, 18, 22);

/// The label of the service each sub-channel carries (its primary
/// component's), and its kind.
fn owners(e: &Ensemble) -> BTreeMap<u8, (String, u32, Color32)> {
    let mut m = BTreeMap::new();
    for s in e.services.values() {
        for c in &s.components {
            let Some(sc) = e.subchannel_of(c) else {
                continue;
            };
            let colour = match c.transport {
                Transport::Audio { ascty: 63, .. } => DABPLUS,
                Transport::Audio { .. } => DAB,
                _ => DATA,
            };
            // A primary component names its sub-channel before a secondary one.
            if c.primary || !m.contains_key(&sc.id) {
                m.insert(sc.id, (s.name(), s.sid, colour));
            }
        }
    }
    m
}

/// The bar under the card: the CIF's 864 capacity units, each sub-channel a
/// segment in its service's colour, the playing one lit. A click on a
/// segment asks to play its service.
pub fn bar(ui: &mut Ui, e: &Ensemble, playing: Option<u32>) -> Option<u32> {
    if e.subchannels.is_empty() {
        return None;
    }
    let names = owners(e);
    let hot = playing.and_then(|sid| {
        let s = e.services.get(&sid)?;
        e.subchannel_of(s.primary()?).map(|sc| sc.id)
    });
    let used: u32 = e.subchannels.values().map(|s| s.size as u32).sum();
    ui.horizontal(|ui| {
        ui.label(RichText::new("multiplex").size(12.0).weak());
        ui.label(RichText::new(format!("{used} of 864 CU used")).size(12.0).weak());
        if let Some(sc) = hot.and_then(|id| e.subchannels.get(&id)) {
            ui.label(
                RichText::new(format!(
                    "· playing CU {}–{} ({} CU, sub-ch {})",
                    sc.start,
                    sc.start + sc.size - 1,
                    sc.size,
                    sc.id
                ))
                .size(12.0)
                .color(PLAYING),
            );
        }
    });
    let w = ui.available_width() - 6.0;
    // Rust note: allocate_exact_size hands back the rectangle and the
    // response (hover, clicks) as a pair.
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 14.0), Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(2), EMPTY);
    let x_of = |cu: f32| rect.left() + cu / CUS as f32 * rect.width();
    let mut under = None;
    for sc in e.subchannels.values() {
        let seg = Rect::from_min_max(
            pos2(x_of(sc.start as f32), rect.top()),
            pos2(x_of((sc.start + sc.size) as f32), rect.bottom()),
        );
        let base = names.get(&sc.id).map_or(DATA, |o| o.2);
        let c = if Some(sc.id) == hot { PLAYING } else { base };
        p.rect_filled(seg.shrink2(vec2(0.5, 0.0)), CornerRadius::ZERO, c);
        if resp.hover_pos().is_some_and(|pt| seg.contains(pt)) {
            under = Some(sc);
        }
    }
    let mut play = None;
    if let Some(sc) = under {
        let (name, sid) = names
            .get(&sc.id)
            .map_or(("?".to_string(), None), |o| (o.0.clone(), Some(o.1)));
        resp.clone().on_hover_text(format!(
            "{name}\nsub-channel {} · CU {}–{} · {} · {} kbit/s",
            sc.id,
            sc.start,
            sc.start + sc.size - 1,
            sc.protection_name(),
            sc.bitrate()
        ));
        if resp.clicked() {
            play = sid;
        }
    }
    play
}
