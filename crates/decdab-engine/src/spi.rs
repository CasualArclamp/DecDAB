// made by CasualArclamp
//! Service and Programme Information (SPI, TS 102 818 / TS 102 371): what
//! an ensemble's SPI data services have sent so far, kept for the GUI.
//!
//! - **Logos** are MOT images. The service information file (SI, ContentType
//!   7/0) lists each service's `multimedia` URLs; until it arrives, an image
//!   whose name holds a service's SId in hex (the BBC's `siC22401.png`) is
//!   taken as that service's.
//! - **Schedules** are programme information files (PI, 7/1). Each
//!   `programme` gives a start time, a duration, names and a description.
//!   The service it belongs to comes from its bearer, the schedule's scope or
//!   the object's MOT ScopeId and name, whichever names a known SId.

use std::collections::BTreeMap;
use std::sync::Arc;

use decdab_data::DataEvent;
use decdab_data::epg::{EpgElement, EpgValue};
use decdab_data::mot::{MotHeader, content_type};

/// A station logo.
#[derive(Debug, Clone)]
pub struct Logo {
    pub name: String,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub data: Arc<Vec<u8>>,
}

/// One programme of a schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Programme {
    /// Start, Unix seconds (UTC).
    pub start: i64,
    /// Seconds.
    pub duration: u32,
    pub name: String,
    pub description: String,
}

impl Programme {
    pub fn end(&self) -> i64 {
        self.start + i64::from(self.duration)
    }
}

/// Whatever names a service, gathered from one SPI object: identifiers as
/// sent (DAB contentIds are bytes; TS 102 818 ids are text) and names.
#[derive(Debug, Clone, Default)]
struct Mentions {
    bytes: Vec<Vec<u8>>,
    texts: Vec<String>,
}

impl Mentions {
    fn names(&self, sid: u32) -> bool {
        let be: Vec<u8> = if sid > 0xFFFF {
            sid.to_be_bytes().to_vec()
        } else {
            (sid as u16).to_be_bytes().to_vec()
        };
        let hex = if sid > 0xFFFF {
            format!("{sid:08x}")
        } else {
            format!("{sid:04x}")
        };
        self.bytes.iter().any(|b| b.windows(be.len()).any(|w| w == be))
            || self.texts.iter().any(|t| t.to_lowercase().contains(&hex))
    }

    fn add_value(&mut self, v: &EpgValue) {
        match v {
            EpgValue::Raw(b) | EpgValue::Genre(b) => self.bytes.push(b.clone()),
            EpgValue::Sid(b) => self.bytes.push(b.to_vec()),
            other => self.texts.push(other.to_xml_string()),
        }
    }
}

#[derive(Debug, Clone)]
struct Schedule {
    mentions: Mentions,
    programmes: Vec<Programme>,
}

/// One service element of the SI file: who it is, and its logos' URLs.
#[derive(Debug, Clone)]
struct SiService {
    mentions: Mentions,
    urls: Vec<String>,
}

/// Everything the SPI services have delivered.
#[derive(Debug, Clone, Default)]
pub struct SpiStore {
    /// By object name: a newer version replaces the old.
    logos: BTreeMap<String, Logo>,
    schedules: BTreeMap<String, Schedule>,
    si: Vec<SiService>,
    /// Bumped by every change, so a GUI can tell.
    pub generation: u64,
    /// Objects received.
    pub objects: u64,
    /// The SPI carousel as its MOT directory lists it: objects, and seconds
    /// to send them all once at the sub-channel's bit rate.
    pub listed: usize,
    pub carousel_s: f32,
}

fn basename(url: &str) -> &str {
    url.rsplit(['/', '\\']).next().unwrap_or(url)
}

impl SpiStore {
    /// Take one event of an SPI data service.
    pub fn push(&mut self, ev: DataEvent) {
        match ev {
            DataEvent::MotObject {
                header, body, ..
            } if header.content_type == content_type::IMAGE => self.image(&header, body),
            DataEvent::SlideShowImage {
                name, mime, data, ..
            } => {
                if let Some((w, h)) = image_size(&data) {
                    self.add_logo(Logo {
                        name,
                        mime,
                        width: w,
                        height: h,
                        data: Arc::new(data),
                    });
                }
            }
            DataEvent::Epg {
                name, root, header, ..
            } => {
                self.objects += 1;
                self.generation += 1;
                match header.content_subtype {
                    0 => self.service_information(&root),
                    1 => {
                        let mut m = Mentions::default();
                        m.texts.push(name.clone());
                        if let Some(id) = header.epg_scope_id() {
                            m.bytes.push(id.to_be_bytes().to_vec());
                        }
                        let mut programmes = Vec::new();
                        walk(&root, &mut |e| match e.name.as_str() {
                            "programme" => {
                                if let Some(p) = programme(e) {
                                    programmes.push(p);
                                }
                            }
                            // The schedule's scope, and each programme's
                            // location, name the service.
                            "serviceScope" | "bearer" | "serviceID" => {
                                if let Some(v) = e.attribute("id") {
                                    m.add_value(v);
                                }
                            }
                            _ => {}
                        });
                        programmes.sort_by_key(|p| p.start);
                        self.schedules.insert(
                            name,
                            Schedule {
                                mentions: m,
                                programmes,
                            },
                        );
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn image(&mut self, header: &MotHeader, body: Vec<u8>) {
        let name = header.content_name().unwrap_or_default();
        let mime = if body.starts_with(b"\x89PNG") {
            "image/png"
        } else {
            "image/jpeg"
        };
        if let Some((w, h)) = image_size(&body) {
            self.add_logo(Logo {
                name,
                mime: mime.into(),
                width: w,
                height: h,
                data: Arc::new(body),
            });
        }
    }

    fn add_logo(&mut self, logo: Logo) {
        self.objects += 1;
        self.generation += 1;
        self.logos.insert(logo.name.clone(), logo);
    }

    fn service_information(&mut self, root: &EpgElement) {
        let mut si = Vec::new();
        walk(root, &mut |e| {
            if e.name != "service" {
                return;
            }
            let mut s = SiService {
                mentions: Mentions::default(),
                urls: Vec::new(),
            };
            walk(e, &mut |c| match c.name.as_str() {
                "serviceID" | "bearer" => {
                    if let Some(v) = c.attribute("id") {
                        s.mentions.add_value(v);
                    }
                }
                "multimedia" => {
                    if let Some(u) = c.attribute("url") {
                        s.urls.push(basename(&u.to_xml_string()).to_string());
                    }
                }
                _ => {}
            });
            si.push(s);
        });
        if !si.is_empty() {
            self.si = si;
        }
    }

    /// Logos received.
    pub fn logo_count(&self) -> usize {
        self.logos.len()
    }

    /// A service's logos, any size.
    pub fn logos(&self, sid: u32) -> Vec<&Logo> {
        let from_si: Vec<&str> = self
            .si
            .iter()
            .filter(|s| s.mentions.names(sid))
            .flat_map(|s| s.urls.iter().map(String::as_str))
            .collect();
        self.logos
            .values()
            .filter(|l| {
                if from_si.is_empty() {
                    Mentions {
                        bytes: Vec::new(),
                        texts: vec![l.name.clone()],
                    }
                    .names(sid)
                } else {
                    from_si.contains(&basename(&l.name))
                }
            })
            .collect()
    }

    /// The service's logo nearest `width` pixels wide, at least that wide if
    /// it can be, and a little wider than tall rather than a banner.
    pub fn logo(&self, sid: u32, width: u32) -> Option<&Logo> {
        let all = self.logos(sid);
        // Square or 4:3 logos first: the 112 × 32 banner is for one-line
        // displays.
        let shaped = |l: &&Logo| l.width <= l.height * 2;
        let pool: Vec<&Logo> = if all.iter().any(shaped) {
            all.into_iter().filter(shaped).collect()
        } else {
            all
        };
        pool.iter()
            .filter(|l| l.width >= width)
            .min_by_key(|l| l.width)
            .or_else(|| pool.iter().max_by_key(|l| l.width))
            .copied()
    }

    /// A service's programmes, in start order (several schedule files merged,
    /// a programme sent twice listed once).
    pub fn schedule(&self, sid: u32) -> Vec<Programme> {
        let mut v: Vec<Programme> = self
            .schedules
            .values()
            .filter(|s| s.mentions.names(sid))
            .flat_map(|s| s.programmes.iter().cloned())
            .collect();
        v.sort_by_key(|p| p.start);
        v.dedup_by(|a, b| a.start == b.start);
        v
    }

    /// Schedules received (files).
    pub fn schedule_count(&self) -> usize {
        self.schedules.len()
    }
}

/// Visit `e` and every element below it.
fn walk(e: &EpgElement, f: &mut impl FnMut(&EpgElement)) {
    f(e);
    for c in e.elements() {
        walk(c, f);
    }
}

fn programme(e: &EpgElement) -> Option<Programme> {
    let text = |name: &str| e.find(name).map(|n| n.text_content().trim().to_string());
    let name = text("longName")
        .filter(|s| !s.is_empty())
        .or_else(|| text("mediumName"))
        .or_else(|| text("shortName"))
        .unwrap_or_default();
    // TS 102 818 §6.3: location > time (time, duration; the actual ones win).
    let mut start = None;
    let mut duration = 0u32;
    walk(e, &mut |c| {
        if c.name != "time" || start.is_some() {
            return;
        }
        let t = c.attribute("actualTime").or(c.attribute("time"));
        if let Some(EpgValue::Time(t)) = t {
            start = Some(t.to_unix());
            if let Some(EpgValue::Duration(d)) = c.attribute("actualDuration").or(c.attribute("duration")) {
                duration = u32::from(*d);
            }
        }
    });
    let mut description = String::new();
    walk(e, &mut |c| {
        if description.is_empty() && (c.name == "shortDescription" || c.name == "longDescription") {
            description = c.text_content().trim().to_string();
        }
    });
    Some(Programme {
        start: start?,
        duration,
        name,
        description,
    })
}

/// Width and height of a PNG (IHDR) or a JPEG (its first SOF marker).
pub fn image_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") && b.len() >= 24 {
        let w = u32::from_be_bytes(b[16..20].try_into().ok()?);
        let h = u32::from_be_bytes(b[20..24].try_into().ok()?);
        return Some((w, h));
    }
    if b.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2;
        while i + 9 < b.len() {
            if b[i] != 0xFF {
                return None;
            }
            let m = b[i + 1];
            let len = usize::from(u16::from_be_bytes([b[i + 2], b[i + 3]]));
            // SOF0..SOF15, except DHT (C4), JPG (C8) and DAC (CC).
            if (0xC0..=0xCF).contains(&m) && ![0xC4, 0xC8, 0xCC].contains(&m) {
                let h = u32::from(u16::from_be_bytes([b[i + 5], b[i + 6]]));
                let w = u32::from(u16::from_be_bytes([b[i + 7], b[i + 8]]));
                return Some((w, h));
            }
            i += 2 + len;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use decdab_data::time::MotTime;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        b.extend(w.to_be_bytes());
        b.extend(h.to_be_bytes());
        b
    }

    #[test]
    fn logos_by_name_then_by_si() {
        let mut s = SpiStore::default();
        for (name, w, h) in [("siC22401.png", 128, 128), ("siC22403.png", 112, 32), ("siC22400.png", 320, 240)] {
            let mut header = MotHeader {
                content_type: content_type::IMAGE,
                ..Default::default()
            };
            header.set_content_name(name);
            s.push(DataEvent::MotObject {
                transport_id: 1,
                header,
                body: png(w, h),
            });
        }
        assert_eq!(s.logos(0xC224).len(), 3);
        assert_eq!(s.logo(0xC224, 100).unwrap().width, 128);
        assert_eq!(s.logo(0xC224, 600).unwrap().width, 320);
        assert!(s.logo(0xC221, 32).is_none());
    }

    #[test]
    fn schedule_from_a_programme_file() {
        let t = MotTime::from_unix(1_790_000_000);
        let prog = EpgElement::new("programme")
            .child(EpgElement::new("mediumName").text("Today"))
            .child(
                EpgElement::new("location")
                    .child(
                        EpgElement::new("time")
                            .attr("time", EpgValue::Time(t))
                            .attr("duration", EpgValue::Duration(3600)),
                    )
                    .child(EpgElement::new("bearer").attr("id", EpgValue::Raw(vec![0x40, 0xC2, 0x24]))),
            );
        let root = EpgElement::new("epg").child(EpgElement::new("schedule").child(prog));
        let header = MotHeader {
            content_type: content_type::EPG,
            content_subtype: 1,
            ..Default::default()
        };
        let mut s = SpiStore::default();
        s.push(DataEvent::Epg {
            name: "w20261025d.EHB".into(),
            xml: String::new(),
            root,
            header,
        });
        let p = s.schedule(0xC224);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "Today");
        assert_eq!(p[0].end() - p[0].start, 3600);
        assert!(s.schedule(0xC221).is_empty());
    }
}
