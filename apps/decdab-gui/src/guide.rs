// made by CasualArclamp
//! The Guide panel: the programme schedules the ensemble's SPI service has
//! sent (TS 102 818), for the playing service or the one picked on the
//! left, with what is on now marked.

use decdab_data::time::MotTime;
use decdab_engine::Programme;
use decdab_fic::{Ensemble, Transport};
use eframe::egui::{self, Color32, RichText, Ui};

use crate::logos::{Logos, fit};

const NOW: Color32 = Color32::from_rgb(110, 220, 130);

#[derive(Default)]
pub struct GuideView {
    /// The service picked here; the playing one when `None`.
    pub picked: Option<u32>,
}

/// "Now", Unix seconds: the ensemble's clock (FIG 0/10), which also holds
/// for a recording, or the computer's.
fn now(ens: &Ensemble) -> i64 {
    match ens.time {
        Some(t) => MotTime {
            mjd: t.mjd,
            hours: t.hours,
            minutes: t.minutes,
            seconds: t.seconds.unwrap_or(0),
            millis: 0,
            long_form: t.seconds.is_some(),
            lto_half_hours: None,
        }
        .to_unix(),
        None => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64),
    }
}

/// HH:MM in the ensemble's local time (FIG 0/9's offset).
fn clock(t: i64, ens: &Ensemble) -> String {
    let local = t + i64::from(ens.lto_half_hours.unwrap_or(0)) * 1800;
    let m = local.rem_euclid(86_400) / 60;
    format!("{:02}:{:02}", m / 60, m % 60)
}

fn day(t: i64, ens: &Ensemble) -> String {
    let local = t + i64::from(ens.lto_half_hours.unwrap_or(0)) * 1800;
    let (y, mo, d) = MotTime::from_unix(local).date();
    format!("{y:04}-{mo:02}-{d:02}")
}

impl GuideView {
    pub fn show(&mut self, ui: &mut Ui, ens: &Ensemble, playing: Option<u32>, logos: &mut Logos) {
        let ctx = ui.ctx().clone();
        let spi = std::sync::Arc::clone(&logos.spi);
        let audio: Vec<_> = ens
            .services_by_name()
            .into_iter()
            .filter(|s| matches!(s.primary().map(|c| c.transport), Some(Transport::Audio { .. })))
            .collect();
        let sid = self.picked.or(playing).or(audio.first().map(|s| s.sid));
        let t_now = now(ens);
        ui.horizontal_top(|ui| {
            // The services, with their logos; those with a schedule bright.
            ui.vertical(|ui| {
                ui.set_width(210.0);
                egui::ScrollArea::vertical()
                    .id_salt("guide_services")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for s in &audio {
                            let has = !spi.schedule(s.sid).is_empty();
                            ui.horizontal(|ui| {
                                match logos.get(&ctx, s.sid, 32) {
                                    Some(t) => {
                                        fit(ui, &t, 22.0, 18.0);
                                    }
                                    None => {
                                        ui.add_space(26.0);
                                    }
                                }
                                let mut name = RichText::new(s.name());
                                if !has {
                                    name = name.weak();
                                }
                                if playing == Some(s.sid) {
                                    name = name.color(NOW);
                                }
                                if ui.selectable_label(sid == Some(s.sid), name).clicked() {
                                    self.picked = Some(s.sid);
                                }
                            });
                        }
                    });
            });
            ui.separator();
            ui.vertical(|ui| {
                let Some(sid) = sid else {
                    ui.label(RichText::new("no services yet").weak());
                    return;
                };
                let name = ens.services.get(&sid).map_or(format!("{sid:04X}"), |s| s.name());
                let progs = spi.schedule(sid);
                ui.horizontal(|ui| {
                    if let Some(t) = logos.get(&ctx, sid, 128) {
                        fit(ui, &t, 48.0, 36.0);
                    }
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&name).size(16.0).strong());
                        let status = if spi.objects == 0 {
                            "nothing from the ensemble's SPI service yet".to_string()
                        } else {
                            let mut s = format!(
                                "SPI: {} logos, {} schedules",
                                spi.logo_count(),
                                spi.schedule_count()
                            );
                            if spi.listed > 0 {
                                s += &format!(
                                    " of {} objects; the carousel comes round every {:.0} s or so",
                                    spi.listed, spi.carousel_s
                                );
                            }
                            s
                        };
                        ui.label(RichText::new(status).size(11.0).weak());
                    });
                });
                ui.separator();
                if progs.is_empty() {
                    ui.label(
                        RichText::new(if spi.objects == 0 {
                            "Waiting for the guide."
                        } else {
                            "No schedule for this service yet: the schedules follow the logos in the carousel."
                        })
                        .weak(),
                    );
                    return;
                }
                list(ui, &progs, t_now, ens);
            });
        });
    }
}

/// The schedule from the programme before the one on now, a day heading at
/// each change of date.
fn list(ui: &mut Ui, progs: &[Programme], t_now: i64, ens: &Ensemble) {
    let first = progs
        .iter()
        .position(|p| p.end() > t_now)
        .unwrap_or(progs.len().saturating_sub(1))
        .saturating_sub(1);
    let mut last_day = String::new();
    egui::ScrollArea::vertical()
        .id_salt("guide_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("guide_rows")
                .num_columns(2)
                .spacing([12.0, 4.0])
                .striped(true)
                .show(ui, |ui| {
                    for p in &progs[first..] {
                        let d = day(p.start, ens);
                        if d != last_day {
                            ui.label(RichText::new(&d).size(11.0).weak());
                            ui.label("");
                            ui.end_row();
                            last_day = d;
                        }
                        let on = p.start <= t_now && t_now < p.end();
                        let mut time = RichText::new(format!(
                            "{}–{}",
                            clock(p.start, ens),
                            clock(p.end(), ens)
                        ))
                        .monospace();
                        let mut title = RichText::new(&p.name).strong();
                        if on {
                            time = time.color(NOW);
                            title = title.color(NOW);
                        }
                        ui.label(time);
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(title);
                                if on {
                                    ui.label(RichText::new("on now").size(11.0).color(NOW));
                                }
                            });
                            if !p.description.is_empty() {
                                ui.add(
                                    egui::Label::new(RichText::new(&p.description).size(12.0).weak())
                                        .wrap(),
                                );
                            }
                        });
                        ui.end_row();
                    }
                });
        });
}
