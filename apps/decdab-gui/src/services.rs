// made by CasualArclamp
//! The right-hand panel under the signal card: the ensemble's services
//! (click one to play it) and what the playing service sends with its
//! sound: the dynamic label (with DL Plus's title and artist), the
//! slideshow's picture, and how its audio is coded and received.

use decdab_audio::AudioHandle;
use decdab_data::pad::{DynamicLabel, dl_plus_name};
use decdab_engine::ServiceStats;
use decdab_fic::{Ensemble, Service, Transport, pty_name};
use eframe::egui::{self, Color32, RichText, TextureHandle, Ui};

use crate::logos::{Logos, fit};

const PLAYING: Color32 = Color32::from_rgb(110, 220, 130);
const DABPLUS: Color32 = Color32::from_rgb(110, 190, 255);
const DAB: Color32 = Color32::from_rgb(240, 200, 90);

pub enum Action {
    Play(u32),
    Stop,
}

/// "DAB+", "DAB" or "data", and its colour.
fn kind(s: &Service) -> (&'static str, Color32) {
    match s.primary().map(|c| c.transport) {
        Some(Transport::Audio { ascty: 63, .. }) => ("DAB+", DABPLUS),
        Some(Transport::Audio { .. }) => ("DAB", DAB),
        _ => ("data", Color32::GRAY),
    }
}

/// The service list. Data services are listed after the audio ones, dimmed
/// (they cannot be played).
pub fn list(
    ui: &mut Ui,
    ens: &Ensemble,
    selected: &mut Option<u32>,
    playing: Option<u32>,
    logos: &mut Logos,
) -> Option<Action> {
    let ctx = ui.ctx().clone();
    let mut action = None;
    let mut services = ens.services_by_name();
    services.sort_by_key(|s| !matches!(s.primary().map(|c| c.transport), Some(Transport::Audio { .. })));
    if services.is_empty() {
        ui.label(RichText::new("the FIC has not listed any services yet").weak());
        return None;
    }
    egui::ScrollArea::vertical()
        .id_salt("services")
        .max_height(ui.available_height().max(160.0))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Grid::new("service_rows")
                .num_columns(5)
                .spacing([10.0, 3.0])
                .striped(true)
                .show(ui, |ui| {
                    for s in services {
                        let (k, colour) = kind(s);
                        let audio = k != "data";
                        let is_playing = playing == Some(s.sid);
                        let mut name = RichText::new(s.name());
                        if is_playing {
                            name = name.color(PLAYING).strong();
                        } else if !audio {
                            name = name.weak();
                        }
                        // Its logo (from SPI), small.
                        match logos.get(&ctx, s.sid, 32) {
                            Some(t) => {
                                fit(ui, &t, 22.0, 16.0);
                            }
                            None => {
                                ui.label("");
                            }
                        }
                        let r = ui.selectable_label(*selected == Some(s.sid), name);
                        let sub = s.primary().and_then(|c| ens.subchannel_of(c));
                        let hover = format!(
                            "SId {:04X}{}{}",
                            s.sid,
                            sub.map_or(String::new(), |x| format!(
                                " · sub-channel {} · {} CU · {}",
                                x.id,
                                x.size,
                                x.protection_name()
                            )),
                            s.label
                                .as_ref()
                                .map_or(String::new(), |l| format!(" · short label \"{}\"", l.short))
                        );
                        let r = r.on_hover_text(hover);
                        if r.clicked() {
                            *selected = Some(s.sid);
                            if audio {
                                action = Some(Action::Play(s.sid));
                            }
                        }
                        ui.label(RichText::new(k).size(11.0).color(colour));
                        ui.label(
                            RichText::new(sub.map_or(String::new(), |x| format!("{}k", x.bitrate())))
                                .size(11.0),
                        );
                        ui.label(
                            RichText::new(s.pty.map_or("", pty_name))
                                .size(11.0)
                                .weak(),
                        );
                        ui.end_row();
                    }
                });
        });
    action
}

/// What the playing service shows.
pub struct NowPlaying<'a> {
    pub service: &'a Service,
    pub stats: Option<&'a ServiceStats>,
    pub dls: Option<&'a DynamicLabel>,
    pub slide: Option<&'a TextureHandle>,
    pub slide_name: Option<&'a str>,
    /// The station's logo (SPI), square-ish, and a bigger one shown in the
    /// slideshow's place until a slide comes.
    pub logo: Option<TextureHandle>,
    pub big_logo: Option<TextureHandle>,
    pub audio: Option<&'a AudioHandle>,
}

pub fn now_playing(ui: &mut Ui, np: &NowPlaying, volume: &mut f32) -> Option<Action> {
    let mut action = None;
    ui.horizontal(|ui| {
        if let Some(t) = &np.logo {
            fit(ui, t, 36.0, 36.0);
        }
        ui.label(
            RichText::new(np.service.name())
                .size(18.0)
                .strong()
                .color(PLAYING),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            if ui.button("■ Stop").clicked() {
                action = Some(Action::Stop);
            }
            let before = *volume;
            ui.add(egui::Slider::new(volume, 0.0..=1.0).show_value(false))
                .on_hover_text("volume");
            if *volume != before {
                decdab_audio::set_volume(*volume);
            }
        });
    });
    // The dynamic label, and DL Plus's objects.
    match np.dls {
        Some(l) => {
            ui.label(RichText::new(&l.text).size(13.0));
            let shown: Vec<String> = l
                .tags
                .iter()
                .filter(|(k, _)| !dl_plus_name(*k).is_empty())
                .map(|(k, v)| format!("{}: {v}", dl_plus_name(*k)))
                .collect();
            if !shown.is_empty() {
                ui.label(RichText::new(shown.join("  ·  ")).size(12.0).color(DABPLUS));
            }
        }
        None => {
            ui.label(RichText::new("no dynamic label yet").weak());
        }
    }
    // The slideshow.
    // A logo stands in for the slideshow only if it is big enough not to
    // be blown up into a blur (the cache may have only the 32 px one).
    let big = np.big_logo.as_ref().filter(|t| t.size()[0] >= 120);
    if let Some(tex) = np.slide.or(big) {
        let w = (ui.available_width() - 8.0).min(320.0);
        let [tw, th] = tex.size();
        let h = w * th as f32 / tw.max(1) as f32;
        ui.add(egui::Image::new((tex.id(), egui::vec2(w, h))))
            .on_hover_text(np.slide_name.unwrap_or("slide"));
    }
    // How it is coded and received.
    if let Some(s) = np.stats {
        let mut bits = Vec::new();
        if !s.codec.is_empty() {
            bits.push(s.codec.clone());
        }
        if s.superframes > 0 {
            bits.push(format!(
                "RS {} fixed · {} lost",
                s.rs_corrected, s.rs_failed
            ));
        }
        bits.push(format!("frames {} good · {} bad", s.au_ok, s.au_bad));
        if s.concealed > 0 {
            bits.push(format!("{} concealed", s.concealed));
        }
        ui.label(RichText::new(bits.join(" · ")).size(11.0).weak());
        if !s.synced && s.frames > 20 {
            ui.label(RichText::new("audio not in sync yet").color(Color32::from_rgb(255, 176, 40)));
        }
    }
    if let Some(a) = np.audio {
        let st = a.status();
        if let Some(e) = &st.error {
            ui.label(RichText::new(format!("sound: {e}")).color(Color32::from_rgb(255, 90, 80)));
        }
    }
    action
}
