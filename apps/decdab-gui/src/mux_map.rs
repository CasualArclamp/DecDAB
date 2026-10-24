// made by CasualArclamp
//! Where a service lives in the multiplex: the CIF's 864 capacity units,
//! each sub-channel a segment in its service's colour and the playing one
//! lit, for the strip the band view lays over the waterfall (EN 300 401
//! §6.2: a sub-channel is a run of capacity units, start and size from
//! FIG 0/1).

use std::collections::BTreeMap;

use decdab_fic::{Ensemble, Transport};
use eframe::egui::Color32;

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

/// One sub-channel of the multiplex, as the band view draws it.
#[derive(Debug, Clone)]
pub struct Segment {
    /// First capacity unit and how many.
    pub start: u16,
    pub size: u16,
    pub colour: Color32,
    /// It carries the service playing.
    pub playing: bool,
    /// Its service, for the pointer, and what to play on a click.
    pub label: String,
    pub sid: Option<u32>,
}

/// The CIF's 864 capacity units as segments, one per sub-channel, in its
/// service's colour; the playing one lit.
pub fn segments(e: &Ensemble, playing: Option<u32>) -> Vec<Segment> {
    let names = owners(e);
    let hot = playing.and_then(|sid| {
        let s = e.services.get(&sid)?;
        e.subchannel_of(s.primary()?).map(|sc| sc.id)
    });
    e.subchannels
        .values()
        .map(|sc| {
            let (name, sid, colour) = names
                .get(&sc.id)
                .map_or(("?".to_string(), None, DATA), |o| (o.0.clone(), Some(o.1), o.2));
            let playing = Some(sc.id) == hot;
            Segment {
                start: sc.start,
                size: sc.size,
                colour: if playing { PLAYING } else { colour },
                playing,
                label: format!(
                    "{name}
sub-channel {} · CU {}–{} · {} · {} kbit/s",
                    sc.id,
                    sc.start,
                    sc.start + sc.size - 1,
                    sc.protection_name(),
                    sc.bitrate()
                ),
                sid,
            }
        })
        .collect()
}

/// Capacity units a CIF holds.
pub const CIF_CUS: usize = CUS;

/// The background of capacity units no sub-channel uses.
pub const UNUSED: Color32 = EMPTY;
