// made by CasualArclamp
//! The band view: the spectrum over the waterfall, one frequency axis for
//! both, the Band III blocks marked, and the one VFO (the ensemble being
//! received) bracketed. From DecDVB-T's band view (after DecSAT's).

use decdab_core::channel::{BAND_III, Block, block_at};

use crate::mux_map::{CIF_CUS, Segment, UNUSED};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Mesh, Pos2, Rect, Sense, Shape,
    Stroke, TextureId, pos2, vec2,
};

/// Half of a DAB ensemble's occupied bandwidth (1536 carriers, 1 kHz apart).
const OCCUPIED_HALF_HZ: f64 = 768e3;
/// Width reserved left of the plot for the dB scale.
const LEFT: f32 = 48.0;
/// Height of the frequency scale under the spectrum.
const AXIS_H: f32 = 20.0;

const AMBER: Color32 = Color32::from_rgb(255, 176, 40);
const TRACE: Color32 = Color32::from_rgb(150, 205, 255);

/// What to draw.
pub struct BandInput<'a> {
    /// Smoothed spectrum, dBFS, the most negative frequency first.
    pub spectrum_db: &'a [f32],
    pub center_hz: f64,
    pub span_hz: f64,
    /// Colour range of the waterfall, which the spectrum's scale follows.
    pub levels: (f32, f32),
    pub waterfall: Option<(TextureId, Rect)>,
    /// The ensemble being received, if one is picked.
    pub vfo_hz: Option<f64>,
    /// Its multiplex, drawn across its bracket at the top of the waterfall.
    pub mux: &'a [Segment],
}

pub enum Action {
    /// A click on a block in the view.
    Tune(Block),
    /// A click on a sub-channel of the multiplex strip: play its service.
    Play(u32),
}

pub fn show(ui: &mut egui::Ui, inp: &BandInput) -> Option<Action> {
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click());
    let rect = resp.rect;
    painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgb(12, 12, 14));
    if rect.width() < 200.0 || rect.height() < 160.0 || inp.span_hz <= 0.0 {
        if inp.span_hz <= 0.0 {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "no signal yet — pick a block and press Start, or open an IQ file",
                FontId::monospace(13.0),
                Color32::GRAY,
            );
        }
        return None;
    }

    let spec_h = (rect.height() * 0.36).clamp(110.0, 340.0);
    let plot_l = rect.left() + LEFT;
    let plot_r = rect.right() - 6.0;
    let spec = Rect::from_min_max(
        pos2(plot_l, rect.top() + 4.0),
        pos2(plot_r, rect.top() + spec_h),
    );
    let wf = Rect::from_min_max(
        pos2(plot_l, spec.bottom() + AXIS_H),
        pos2(plot_r, rect.bottom() - 4.0),
    );
    let lo_hz = inp.center_hz - inp.span_hz / 2.0;
    let x_of = |hz: f64| plot_l + ((hz - lo_hz) / inp.span_hz) as f32 * (plot_r - plot_l);
    let hz_of = |x: f32| lo_hz + ((x - plot_l) / (plot_r - plot_l)) as f64 * inp.span_hz;
    // The waterfall's colour range, with room below for the notches and the
    // floor outside the channel and above for the peaks.
    let (lo_db, hi_db) = (inp.levels.0 - 10.0, inp.levels.1 + 5.0);
    let y_of = |db: f32| {
        let f = ((db - lo_db) / (hi_db - lo_db).max(1.0)).clamp(0.0, 1.0);
        spec.bottom() - f * spec.height()
    };
    let font = FontId::monospace(11.0);
    let grid = Color32::from_rgba_unmultiplied(255, 255, 255, 22);
    let dim = Color32::from_gray(150);

    // dB grid and scale.
    let mut db = (lo_db / 10.0).ceil() * 10.0;
    while db < hi_db {
        let y = y_of(db);
        painter.line_segment([pos2(plot_l, y), pos2(plot_r, y)], Stroke::new(1.0, grid));
        painter.text(
            pos2(plot_l - 4.0, y),
            Align2::RIGHT_CENTER,
            format!("{db:.0}"),
            font.clone(),
            dim,
        );
        db += 10.0;
    }
    painter.text(
        pos2(rect.left() + 2.0, spec.top()),
        Align2::LEFT_TOP,
        "dBFS",
        font.clone(),
        dim,
    );

    // The blocks in view: a faint band behind each (the N blocks fainter),
    // its label at the top, and the one being received picked out.
    let selected = inp.vfo_hz.and_then(block_at).map(|b| b.name);
    for b in BAND_III {
        let c = b.hz();
        let (x0, x1) = (
            x_of(c - OCCUPIED_HALF_HZ).max(plot_l),
            x_of(c + OCCUPIED_HALF_HZ).min(plot_r),
        );
        if x1 <= plot_l || x0 >= plot_r || x1 - x0 < 4.0 {
            continue;
        }
        let uk = !b.name.ends_with('N');
        let is_sel = selected == Some(b.name);
        let alpha = if is_sel {
            26
        } else if uk {
            9
        } else {
            4
        };
        for r in [spec, wf] {
            painter.rect_filled(
                Rect::from_min_max(pos2(x0, r.top()), pos2(x1, r.bottom())),
                CornerRadius::ZERO,
                Color32::from_rgba_unmultiplied(120, 170, 255, alpha),
            );
        }
        let cx = x_of(c);
        if uk && cx > plot_l + 14.0 && cx < plot_r - 14.0 {
            painter.text(
                pos2(cx, spec.top() + 2.0),
                Align2::CENTER_TOP,
                b.name,
                font.clone(),
                if is_sel { AMBER } else { dim },
            );
        }
    }

    // Frequency scale.
    let step = if inp.span_hz <= 3e6 {
        0.5e6
    } else if inp.span_hz <= 12e6 {
        1e6
    } else {
        2e6
    };
    let mut f = (lo_hz / step).ceil() * step;
    while f < lo_hz + inp.span_hz {
        let x = x_of(f);
        painter.line_segment(
            [pos2(x, spec.top()), pos2(x, spec.bottom())],
            Stroke::new(1.0, grid),
        );
        painter.text(
            pos2(x, spec.bottom() + 3.0),
            Align2::CENTER_TOP,
            if step < 1e6 {
                format!("{:.1}", f / 1e6)
            } else {
                format!("{:.0}", f / 1e6)
            },
            font.clone(),
            dim,
        );
        f += step;
    }
    painter.text(
        pos2(rect.left() + 2.0, spec.bottom() + 3.0),
        Align2::LEFT_TOP,
        "MHz",
        font.clone(),
        dim,
    );

    // The trace: the strongest bin under each pixel column, filled below.
    let n = inp.spectrum_db.len();
    if n > 1 {
        let cols = (plot_r - plot_l).max(1.0) as usize;
        let mut pts: Vec<Pos2> = Vec::with_capacity(cols + 1);
        for c in 0..cols {
            let (k0, k1) = (
                c * n / cols,
                ((c + 1) * n / cols).max(c * n / cols + 1).min(n),
            );
            let m = inp.spectrum_db[k0..k1]
                .iter()
                .copied()
                .fold(f32::MIN, f32::max);
            pts.push(pos2(plot_l + c as f32 + 0.5, y_of(m)));
        }
        let fill = Color32::from_rgba_unmultiplied(70, 150, 255, 38);
        let mut mesh = Mesh::default();
        for p in &pts {
            let i = mesh.vertices.len() as u32;
            mesh.colored_vertex(*p, fill);
            mesh.colored_vertex(pos2(p.x, spec.bottom()), fill);
            if i >= 2 {
                mesh.add_triangle(i - 2, i - 1, i);
                mesh.add_triangle(i - 1, i + 1, i);
            }
        }
        painter.add(Shape::mesh(mesh));
        painter.add(Shape::line(pts, Stroke::new(1.2, TRACE)));
    }

    // The waterfall, newest row at the top.
    if let Some((tex, uv)) = inp.waterfall {
        painter.image(tex, wf, uv, Color32::WHITE);
    }

    // The VFO: the received channel's occupied band.
    if let Some(v) = inp.vfo_hz {
        let edge = Stroke::new(1.4, AMBER);
        for hz in [v - OCCUPIED_HALF_HZ, v + OCCUPIED_HALF_HZ] {
            let x = x_of(hz);
            if (plot_l..=plot_r).contains(&x) {
                painter.line_segment([pos2(x, spec.top()), pos2(x, wf.bottom())], edge);
            }
        }
    }

    // The multiplex: the CIF's capacity units across the ensemble's bracket,
    // the playing service's lit. They are slots in time (a sub-channel sits
    // in one or a few OFDM symbols, spread over every carrier), drawn here
    // only to sit with the ensemble they belong to.
    let mut strip_hit: Option<&Segment> = None;
    let mut strip: Option<Rect> = None;
    if let (Some(v), false) = (inp.vfo_hz, inp.mux.is_empty()) {
        let (x0, x1) = (x_of(v - OCCUPIED_HALF_HZ), x_of(v + OCCUPIED_HALF_HZ));
        if x0 >= plot_l && x1 <= plot_r && x1 - x0 > 80.0 {
            let r = Rect::from_min_max(pos2(x0, wf.top() + 4.0), pos2(x1, wf.top() + 20.0));
            painter.rect_filled(r.expand(1.0), CornerRadius::same(2), Color32::from_black_alpha(200));
            painter.rect_filled(r, CornerRadius::ZERO, UNUSED);
            let cx = |cu: f32| r.left() + cu / CIF_CUS as f32 * r.width();
            let pointer = resp.hover_pos();
            for s in inp.mux {
                let seg = Rect::from_min_max(
                    pos2(cx(s.start as f32), r.top()),
                    pos2(cx((s.start + s.size) as f32), r.bottom()),
                );
                painter.rect_filled(seg.shrink2(vec2(0.5, 0.0)), CornerRadius::ZERO, s.colour);
                if pointer.is_some_and(|pt| seg.contains(pt)) {
                    strip_hit = Some(s);
                }
            }
            let used: u32 = inp.mux.iter().map(|s| s.size as u32).sum();
            let playing = inp.mux.iter().find(|s| s.playing);
            let text = match playing {
                Some(s) => format!(
                    "CU {}–{} of 864 · {used} used · capacity units (time slots, not frequencies)",
                    s.start,
                    s.start + s.size - 1
                ),
                None => format!("multiplex · {used} of 864 CU · capacity units (time slots, not frequencies)"),
            };
            caption(&painter, pos2(r.left(), r.bottom() + 3.0), text, 10.0);
            strip = Some(r);
        }
    }

    // Pointer: a crosshair line and a readout of what is under it.
    let mut action = None;
    if let (Some(s), Some(r)) = (strip_hit, strip) {
        caption(&painter, pos2(r.left(), r.bottom() + 18.0), s.label.clone(), 11.0);
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        if resp.clicked()
            && let Some(sid) = s.sid
        {
            return Some(Action::Play(sid));
        }
        return None;
    }
    if let Some(p) = resp.hover_pos()
        && (plot_l..=plot_r).contains(&p.x)
        && (spec.top()..=wf.bottom()).contains(&p.y)
    {
        let hz = hz_of(p.x);
        painter.line_segment(
            [pos2(p.x, spec.top()), pos2(p.x, wf.bottom())],
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 90)),
        );
        let bin = (((hz - lo_hz) / inp.span_hz) * n as f64) as usize;
        let level = inp.spectrum_db.get(bin).map(|d| format!("  {d:.1} dBFS"));
        let near = BAND_III
            .iter()
            .filter(|b| !b.name.ends_with('N'))
            .min_by(|a, b| (a.hz() - hz).abs().total_cmp(&(b.hz() - hz).abs()))
            .filter(|b| (b.hz() - hz).abs() <= OCCUPIED_HALF_HZ)
            .copied();
        let chan = near
            .map(|b| format!("  {} {:+.0} kHz", b.name, (hz - b.hz()) / 1e3))
            .unwrap_or_default();
        painter.text(
            pos2(plot_r - 4.0, spec.top() + 16.0),
            Align2::RIGHT_TOP,
            format!("{:.3} MHz{chan}{}", hz / 1e6, level.unwrap_or_default()),
            FontId::monospace(12.0),
            Color32::WHITE,
        );
        if near.is_some() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if resp.clicked()
            && let Some(b) = near
        {
            action = Some(Action::Tune(b));
        }
    }
    action
}

/// Text over the waterfall, on a dark backing so it reads over a bright one.
fn caption(painter: &egui::Painter, at: Pos2, text: String, size: f32) {
    let galley = painter.layout_no_wrap(text, FontId::monospace(size), Color32::from_gray(230));
    let r = Rect::from_min_size(at, galley.size()).expand(2.0);
    painter.rect_filled(r, CornerRadius::same(2), Color32::from_black_alpha(190));
    painter.galley(at, galley, Color32::WHITE);
}
