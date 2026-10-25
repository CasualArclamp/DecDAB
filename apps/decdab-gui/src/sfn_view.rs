// made by CasualArclamp
//! The SFN panel: the transmitters of the single-frequency network heard
//! here, from their identification (TII) in the null symbols. Their main
//! and sub identifiers, how strong each is, and how late it arrives (with
//! the extra distance that means), drawn on the averaged impulse response
//! against the guard interval. Beside it, the null symbol's 24 combs × 8
//! carrier groups, which is what the detector reads.

use std::sync::Arc;

use decdab_ofdm::TiiReport;
use decdab_ofdm::tii::{CIR_PRE, COMBS, GROUPS, pattern};
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Rect, RichText, Stroke, Ui, pos2, vec2};

use crate::scopes::{border, decimate, frame, nice_step, ticks};

const DIM: Color32 = Color32::from_gray(150);
const AMBER: Color32 = Color32::from_rgb(255, 176, 40);
const RED: Color32 = Color32::from_rgb(255, 90, 80);
const TRACE: Color32 = Color32::from_rgb(110, 200, 255);
/// One colour per transmitter, strongest first.
const TX: [Color32; 6] = [
    Color32::from_rgb(110, 220, 130),
    Color32::from_rgb(255, 176, 40),
    Color32::from_rgb(200, 140, 255),
    Color32::from_rgb(255, 120, 160),
    Color32::from_rgb(120, 220, 230),
    Color32::from_rgb(230, 230, 120),
];
/// Mode I guard interval, µs.
const GUARD_US: f32 = 246.09375;
/// Radio waves go 0.29979 km a microsecond.
const KM_PER_US: f32 = 0.299_792;

#[derive(Default)]
pub struct SfnView {
    report: Option<Arc<TiiReport>>,
}

impl SfnView {
    pub fn reset(&mut self) {
        self.report = None;
    }

    pub fn update(&mut self, r: Option<Arc<TiiReport>>) {
        if r.is_some() {
            self.report = r;
        }
    }

    pub fn show(&mut self, ui: &mut Ui, height: f32) {
        let r = self.report.clone();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(330.0);
                table(ui, r.as_deref());
            });
            let grid_w = 230.0;
            let w = (ui.available_width() - grid_w - 12.0).max(200.0);
            delays_plot(ui, vec2(w, height), r.as_deref());
            comb_grid(ui, vec2(grid_w, height), r.as_deref());
        });
    }
}

fn table(ui: &mut Ui, r: Option<&TiiReport>) {
    ui.label(RichText::new("Transmitters (TII)").strong());
    let Some(r) = r else {
        ui.label(RichText::new("listening to the null symbols…").weak());
        return;
    };
    if r.transmitters.is_empty() {
        ui.label(RichText::new("no transmitter identification heard").weak());
        ui.label(
            RichText::new("(not every ensemble sends TII, and a weak one needs a few seconds)")
                .size(11.0)
                .weak(),
        );
        return;
    }
    egui::Grid::new("tii_rows")
        .num_columns(5)
        .spacing([10.0, 3.0])
        .striped(true)
        .show(ui, |ui| {
            for h in ["TII", "level", "delay", "farther", "path"] {
                ui.label(RichText::new(h).size(11.0).weak());
            }
            ui.end_row();
            for (i, t) in r.transmitters.iter().enumerate() {
                let c = TX[i % TX.len()];
                ui.label(RichText::new(format!("{:02} {:02}", t.main, t.sub)).monospace().color(c))
                    .on_hover_text(format!(
                        "main identifier {} (carrier groups {}), sub identifier {} (the comb)",
                        t.main,
                        pattern(t.main).map_or(String::new(), |p| (0..GROUPS)
                            .filter(|&b| p[b])
                            .map(|b| b.to_string())
                            .collect::<Vec<_>>()
                            .join(" ")),
                        t.sub
                    ));
                ui.label(RichText::new(format!("{:+.1} dB", t.rel_db)).monospace())
                    .on_hover_text(format!("{:.1} dB over the noise between the TII carriers", t.snr_db));
                ui.label(RichText::new(format!("{:.1} µs", t.delay_us)).monospace());
                ui.label(RichText::new(format!("{:.1} km", t.delay_us * KM_PER_US)).monospace())
                    .on_hover_text("the extra path against the first one: delay × the speed of light");
                ui.label(
                    RichText::new(t.path_db.map_or("—".into(), |d| format!("{d:.0} dB"))).monospace(),
                )
                .on_hover_text("the impulse response at that delay, against its peak");
                ui.end_row();
            }
        });
    ui.add_space(4.0);
    let late = r.transmitters.iter().any(|t| t.delay_us > GUARD_US);
    ui.label(
        RichText::new(if late {
            "a transmitter arrives after the guard interval: it interferes instead of helping"
        } else {
            "all within the guard interval (246 µs, 74 km): they add up"
        })
        .size(11.0)
        .color(if late { RED } else { DIM }),
    );
    ui.label(
        RichText::new(format!(
            "averaged over the last ~2 s · {} null symbols · delays from the phase slope, \
             placed on the impulse response's paths",
            r.frames
        ))
        .size(11.0)
        .weak(),
    );
}

/// The averaged impulse response with each transmitter marked at its delay.
fn delays_plot(ui: &mut Ui, size: egui::Vec2, r: Option<&TiiReport>) {
    let (resp, p, plot) = frame(ui, size, "impulse response and transmitters, dB (µs)");
    let Some(r) = r.filter(|r| r.cir_db.len() > 2) else {
        p.text(plot.center(), Align2::CENTER_CENTER, "—", FontId::monospace(11.0), DIM);
        return;
    };
    let us = |i: usize| (i as f32 - CIR_PRE as f32) / 2.048;
    // Out to the guard and a little past, or the latest transmitter.
    let latest = r.transmitters.iter().map(|t| t.delay_us).fold(0f32, f32::max);
    let (x0, x1) = (us(0), (GUARD_US * 1.15).max(latest + 30.0).min(us(r.cir_db.len() - 1)));
    let (lo, hi) = (-40.0f32, 0.0f32);
    let to = |d: f32, v: f32| {
        pos2(
            plot.left() + (d - x0) / (x1 - x0) * plot.width(),
            plot.bottom() - (v.clamp(lo, hi) - lo) / (hi - lo) * plot.height(),
        )
    };
    let gx = to(GUARD_US, 0.0).x;
    if gx < plot.right() {
        p.rect_filled(
            Rect::from_min_max(pos2(gx, plot.top()), plot.right_bottom()),
            CornerRadius::ZERO,
            RED.gamma_multiply(0.12),
        );
        p.line_segment([pos2(gx, plot.top()), pos2(gx, plot.bottom())], Stroke::new(1.0, AMBER));
    }
    ticks(&p, plot, false, lo, hi, 10.0, |v| format!("{v:.0}"));
    ticks(&p, plot, true, x0, x1, nice_step(x1 - x0), |v| format!("{v:.0}"));
    let pts = r
        .cir_db
        .iter()
        .enumerate()
        .map(|(i, &v)| (us(i), v))
        .filter(|(d, _)| *d <= x1)
        .map(|(d, v)| to(d, v))
        .collect::<Vec<_>>();
    p.add(egui::Shape::line(decimate(plot.width(), pts.into_iter()), Stroke::new(1.2, TRACE)));
    let font = FontId::monospace(10.0);
    for (i, t) in r.transmitters.iter().enumerate() {
        let c = TX[i % TX.len()];
        let x = to(t.delay_us, 0.0).x;
        if !(plot.left()..=plot.right()).contains(&x) {
            continue;
        }
        p.line_segment([pos2(x, plot.top()), pos2(x, plot.bottom())], Stroke::new(1.0, c.gamma_multiply(0.7)));
        p.text(
            pos2(x + 3.0, plot.top() + 2.0 + 12.0 * i as f32),
            Align2::LEFT_TOP,
            format!("{:02}/{:02}", t.main, t.sub),
            font.clone(),
            c,
        );
    }
    border(&p, plot);
    if let Some(pt) = resp.hover_pos()
        && plot.contains(pt)
    {
        let d = x0 + (pt.x - plot.left()) / plot.width() * (x1 - x0);
        p.text(
            plot.left_bottom() + vec2(4.0, -3.0),
            Align2::LEFT_BOTTOM,
            format!("{d:+.1} µs · {:.1} km", d * KM_PER_US),
            font,
            Color32::WHITE,
        );
    }
}

/// Every comb's eight groups as cells, brighter for more power over the
/// noise; the groups of the transmitters found outlined in their colour.
fn comb_grid(ui: &mut Ui, size: egui::Vec2, r: Option<&TiiReport>) {
    let (resp, p, _) = frame(ui, size, "null symbol: comb × group, dB");
    let rect = resp.rect;
    let area = Rect::from_min_max(
        pos2(rect.left() + 22.0, rect.top() + 18.0),
        pos2(rect.right() - 6.0, rect.bottom() - 14.0),
    );
    let Some(r) = r.filter(|r| r.groups_db.len() == COMBS) else {
        p.text(area.center(), Align2::CENTER_CENTER, "—", FontId::monospace(11.0), DIM);
        return;
    };
    let cw = area.width() / GROUPS as f32;
    let ch = area.height() / COMBS as f32;
    let font = FontId::monospace(9.0);
    for (c, g) in r.groups_db.iter().enumerate() {
        for (b, &db) in g.iter().enumerate() {
            let cell = Rect::from_min_size(
                pos2(area.left() + b as f32 * cw, area.top() + c as f32 * ch),
                vec2(cw - 1.0, ch - 1.0),
            );
            let f = (db / 12.0).clamp(0.0, 1.0);
            p.rect_filled(cell, CornerRadius::ZERO, Color32::from_rgb(
                (20.0 + 90.0 * f) as u8,
                (24.0 + 176.0 * f) as u8,
                (30.0 + 225.0 * f) as u8,
            ));
        }
        if c % 4 == 0 {
            p.text(
                pos2(area.left() - 3.0, area.top() + (c as f32 + 0.5) * ch),
                Align2::RIGHT_CENTER,
                c.to_string(),
                font.clone(),
                DIM,
            );
        }
    }
    for b in 0..GROUPS {
        p.text(
            pos2(area.left() + (b as f32 + 0.5) * cw, area.bottom() + 1.0),
            Align2::CENTER_TOP,
            b.to_string(),
            font.clone(),
            DIM,
        );
    }
    for (i, t) in r.transmitters.iter().enumerate() {
        let Some(bits) = pattern(t.main) else { continue };
        for b in (0..GROUPS).filter(|&b| bits[b]) {
            let cell = Rect::from_min_size(
                pos2(area.left() + b as f32 * cw, area.top() + f32::from(t.sub) * ch),
                vec2(cw - 1.0, ch - 1.0),
            );
            p.rect_stroke(cell, CornerRadius::ZERO, Stroke::new(1.5, TX[i % TX.len()]), egui::StrokeKind::Inside);
        }
    }
}
