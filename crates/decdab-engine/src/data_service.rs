// made by CasualArclamp
//! One data component, from its sub-channel's CIFs to MOT objects: the
//! sub-channel decoder, then (packet mode) the packets and data groups on the
//! component's address, then the data decoder for its user application
//! (SPI / EPG, slideshow, Journaline...). Stream-mode data components pass
//! their logical frames straight to the data decoder.

use decdab_data::dab_packet::{DabPacketDemux, DabPacketStats};
use decdab_data::{AppDomain, DataDecoder, DataEvent, DataServiceConfig, DataStats, UserApplication};
use decdab_fic::{Component, Ensemble, Transport};
use decdab_msc::SubChannelDecoder;

/// TS 101 756 Table 2: data service component type 60 is MOT.
const DSCTY_MOT: u8 = 60;

pub struct DataServiceDecoder {
    sub: SubChannelDecoder,
    /// Packet mode only.
    demux: Option<DabPacketDemux>,
    data: DataDecoder,
    app: UserApplication,
    groups: Vec<Vec<u8>>,
}

impl DataServiceDecoder {
    /// For a data component (`None` for an audio one, or while the FIC has
    /// not yet said where it is carried).
    pub fn new(comp: &Component, ens: &Ensemble) -> Option<Self> {
        let sc = *ens.subchannel_of(comp)?;
        let (demux, dscty) = match comp.transport {
            Transport::Packet { scid } => {
                let p = ens.packet.get(&scid)?;
                (Some(DabPacketDemux::new(p.address)), p.dscty)
            }
            Transport::StreamData { dscty, .. } => (None, dscty),
            Transport::Audio { .. } | Transport::Fidc { .. } => return None,
        };
        // The user application (FIG 0/13) decides the handler; until it has
        // been seen, a MOT component is treated as SPI, whose handler also
        // passes any other MOT object (logos, slides) through.
        let app = comp
            .apps
            .first()
            .map(|a| UserApplication::from_id(AppDomain::Dab, a.kind))
            .unwrap_or(if dscty == DSCTY_MOT {
                UserApplication::Epg
            } else {
                UserApplication::Other(0)
            });
        // The DAB packet layer is ours, so the decoder only ever sees whole
        // data groups (`push_data_unit`) or stream frames: it is set up in
        // stream mode, which gives it no DRM packet layer of its own.
        let data = DataDecoder::new(DataServiceConfig::stream(app));
        Some(DataServiceDecoder {
            sub: SubChannelDecoder::new(sc)?,
            demux,
            data,
            app,
            groups: Vec::new(),
        })
    }

    pub fn application(&self) -> UserApplication {
        self.app
    }

    pub fn subchannel(&self) -> &decdab_fic::SubChannel {
        self.sub.subchannel()
    }

    /// Logical frames out of the sub-channel decoder.
    pub fn frames(&self) -> u64 {
        self.sub.frames
    }

    pub fn packet_stats(&self) -> Option<&DabPacketStats> {
        self.demux.as_ref().map(|d| d.stats())
    }

    pub fn data_stats(&self) -> &DataStats {
        self.data.stats()
    }

    /// The MOT decoder, for its directory.
    pub fn mot(&self) -> Option<&decdab_data::mot::MotDecoder> {
        self.data.mot()
    }

    /// Take a CIF's soft bits; what the application produced goes to
    /// `events` (counters are left out: read them with the getters).
    pub fn push_cif(&mut self, cif: &[i8], events: &mut Vec<DataEvent>) {
        let Some(lf) = self.sub.push_cif(cif) else {
            return;
        };
        match &mut self.demux {
            Some(d) => {
                d.push_frame(lf, &mut self.groups);
                for g in self.groups.drain(..) {
                    events.extend(self.data.push_data_unit(&g));
                }
            }
            None => events.extend(
                self.data
                    .push_frame(lf)
                    .into_iter()
                    .filter(|e| !matches!(e, DataEvent::Stats(_))),
            ),
        }
    }
}
