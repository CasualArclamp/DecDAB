// made by CasualArclamp
//! The signal card: the receiver's headline, at the top of the right-hand
//! panel. From DecDVB-T's card (after DecSAT's DVB card): a title bar in the
//! standard's colour with the frequency and block, the ensemble's label big,
//! and the DQPSK MER on a bar; the ensemble's figures under it.

use eframe::egui::{self, Color32, CornerRadius, RichText, Sense, Ui, pos2, vec2};

// The standard's colour (DecSAT's palette has none for DAB: a sky blue,
// clear of DVB-T's yellow and DVB-T2's green), and a margin's health.
const DAB: Color32 = Color32::from_rgb(110, 190, 255);
const HEALTH_OK: Color32 = Color32::from_rgb(110, 220, 110);
const HEALTH_WARN: Color32 = Color32::from_rgb(240, 200, 80);
const HEALTH_BAD: Color32 = Color32::from_rgb(230, 110, 110);
/// The bar before there is a mode to give it a health.
const NEUTRAL: Color32 = Color32::from_rgb(100, 170, 255);
/// Clear of the side panel's scroll bar, which egui floats over the content.
const RIGHT_PAD: f32 = 6.0;

/// The MER scale of the bar, dB: fixed, so it does not jump as the reading
/// moves. DQPSK with the FIC's rate-1/3 code decodes from about 5 dB.
const BAR_DB: (f32, f32) = (0.0, 25.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standard {
    Dab,
}

impl Standard {
    fn name(self) -> &'static str {
        match self {
            Standard::Dab => "DAB · mode I",
        }
    }

    fn colour(self) -> Color32 {
        match self {
            Standard::Dab => DAB,
        }
    }
}

/// What the card shows.
#[derive(Default)]
pub struct Card {
    /// `None` until the receiver knows which.
    pub standard: Option<Standard>,
    /// The mode's other parameters, as label / value pairs (FFT size, guard
    /// interval, bit rate…), shown under the bar; empty until they are known.
    pub details: Vec<(String, String)>,
    /// The mode is the user's assumption, not read from the signal.
    pub assumed: bool,
    /// "64-QAM 3/4", the constellation and rate being received.
    pub name: Option<String>,
    /// The C/N that mode needs, dB.
    pub qef_db: Option<f32>,
    pub cn_db: Option<f32>,
    /// Where the C/N came from: "spectrum", or "MER" once measured.
    pub cn_source: &'static str,
    pub channel: Option<String>,
    pub freq_hz: Option<f64>,
    /// A source is running.
    pub active: bool,
}

/// A margin's colour: red below 0 dB, amber within 1 dB, else green.
fn margin_colour(margin: Option<f32>) -> Color32 {
    match margin {
        Some(m) if m < 0.0 => HEALTH_BAD,
        Some(m) if m < 1.0 => HEALTH_WARN,
        _ => HEALTH_OK,
    }
}

pub fn show(ui: &mut Ui, c: &Card) {
    let grey = ui.visuals().weak_text_color();
    let strong = ui.visuals().strong_text_color();
    let colour = c.standard.map_or(grey, Standard::colour);
    let margin = c.cn_db.zip(c.qef_db).map(|(cn, q)| cn - q);
    let health = margin.map(|m| margin_colour(Some(m)));

    // ---- the title bar: standard · mode, and where it is
    let title = match c.standard {
        Some(s) if c.assumed => format!("{} · assumed", s.name()),
        Some(s) => s.name().to_string(),
        None => "DAB / DAB+".to_string(),
    };
    let bar = egui::Frame::new()
        .fill(colour.gamma_multiply(0.15))
        .corner_radius(CornerRadius {
            nw: 4,
            ne: 4,
            sw: 0,
            se: 0,
        })
        .inner_margin(egui::Margin {
            left: 8,
            right: 8 + RIGHT_PAD as i8,
            top: 3,
            bottom: 3,
        })
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                // The right-hand figures are laid out first (right to left)
                // in what is left, so the title is the one to give way.
                let right = ui.available_width().min(190.0);
                ui.scope(|ui| {
                    ui.set_max_width((ui.available_width() - right).max(40.0));
                    ui.add(
                        egui::Label::new(
                            RichText::new(title)
                                .monospace()
                                .size(14.0)
                                .strong()
                                .color(colour),
                        )
                        .truncate(),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(ch) = &c.channel {
                        ui.label(
                            RichText::new(ch)
                                .monospace()
                                .size(20.0)
                                .color(strong),
                        );
                    }
                    if let Some(f) = c.freq_hz {
                        ui.add_space(10.0);
                        ui.label(RichText::new("MHz").size(11.0).weak());
                        ui.label(
                            RichText::new(format!("{:.3}", f / 1e6))
                                .monospace()
                                .size(16.0)
                                .color(if c.name.is_some() { strong } else { grey }),
                        );
                    }
                });
            });
        })
        .response;
    ui.painter().hline(
        bar.rect.x_range(),
        bar.rect.bottom(),
        egui::Stroke::new(2.0, colour),
    );
    ui.add_space(4.0);

    // ---- the mode and its margin
    let resp = ui
        .horizontal(|ui| {
            ui.label(
                RichText::new(c.name.as_deref().unwrap_or("–"))
                    .monospace()
                    .size(26.0)
                    .color(if c.name.is_some() { colour } else { grey }),
            );
            if let (Some(m), Some(health)) = (margin, health) {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(RIGHT_PAD);
                    ui.label(
                        RichText::new(format!("{m:+5.1} dB"))
                            .monospace()
                            .size(18.0)
                            .color(health),
                    );
                });
            }
        })
        .response;
    resp.on_hover_text(
        "The ensemble's label. The bar is the MER of the differential (DQPSK) \
         decisions over a frame: the figure the Viterbi decoders see.",
    );

    // ---- the C/N bar, ticked at what the mode needs
    let (lo, hi) = BAR_DB;
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width() - RIGHT_PAD, 10.0), Sense::hover());
    let x_of = |db: f32| rect.left() + rect.width() * ((db - lo) / (hi - lo)).clamp(0.0, 1.0);
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(2), ui.visuals().extreme_bg_color);
    if let Some(cn) = c.cn_db {
        let fill = egui::Rect::from_min_max(rect.min, pos2(x_of(cn), rect.max.y));
        p.rect_filled(
            fill,
            CornerRadius::same(2),
            health.unwrap_or(NEUTRAL.gamma_multiply(0.8)),
        );
    }
    if let Some(q) = c.qef_db {
        let x = x_of(q);
        p.rect_filled(
            egui::Rect::from_min_max(
                pos2(x - 1.0, rect.top() - 3.0),
                pos2(x + 1.0, rect.bottom() + 3.0),
            ),
            CornerRadius::ZERO,
            strong,
        );
    }
    ui.horizontal(|ui| {
        let left = match (c.cn_db, c.active) {
            (Some(cn), _) => format!("{} {cn:.1} dB", c.cn_source),
            (None, true) => "no ensemble in this block".into(),
            (None, false) => "no source running".into(),
        };
        ui.label(RichText::new(left).size(12.0).weak());
        if let Some(q) = c.qef_db {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(RIGHT_PAD);
                ui.label(RichText::new(format!("QEF {q:.1} dB")).size(12.0).weak());
            });
        }
    });

    // ---- the rest of the mode: pairs of label and value, two pairs a row
    ui.add_space(6.0);
    if c.details.is_empty() {
        if c.active {
            ui.label(
                RichText::new(
                    "The ensemble's label, services and figures come from the FIC once the receiver locks.",
                )
                .size(12.0)
                .weak(),
            );
        }
        return;
    }
    egui::Grid::new("card_details")
        .num_columns(4)
        .spacing([10.0, 3.0])
        .show(ui, |ui| {
            for pair in c.details.chunks(2) {
                for (k, v) in pair {
                    ui.label(RichText::new(k).size(12.0).weak());
                    ui.label(RichText::new(v).size(12.0).color(strong));
                }
                ui.end_row();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn margin_colours_follow_decsat() {
        assert_eq!(margin_colour(Some(-0.1)), HEALTH_BAD);
        assert_eq!(margin_colour(Some(0.0)), HEALTH_WARN);
        assert_eq!(margin_colour(Some(0.9)), HEALTH_WARN);
        assert_eq!(margin_colour(Some(1.0)), HEALTH_OK);
        assert_eq!(margin_colour(None), HEALTH_OK);
    }
}
