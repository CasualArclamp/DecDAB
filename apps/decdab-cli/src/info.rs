// made by CasualArclamp
//! `decdab info`: what an ensemble's FIC says.

use std::time::Instant;

use anyhow::{Result, bail};
use decdab_fic::{FicDecoder, Transport, pty_name};
use decdab_ofdm::params::FIC_BITS;
use decdab_ofdm::{Demod, Frontend};

use crate::Gains;

pub fn run(what: &str, seconds: f64, gains: Gains) -> Result<()> {
    let mut src = crate::source::open(what, gains)?;
    let rate = src.sample_rate();
    let mut fe = Frontend::new(rate).map_err(anyhow::Error::msg)?;
    let mut demod = Demod::new();
    let mut fic = FicDecoder::new();
    let (mut buf, mut dec) = (Vec::new(), Vec::new());
    let mut read = 0usize;
    let mut last = None;
    let t0 = Instant::now();
    while (read as f64) < seconds * rate {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        read += n;
        fe.process(&buf, &mut dec);
        demod.push(&dec, |f| {
            fic.push_frame(&f.soft[..FIC_BITS]);
            last = Some(f.stats);
        });
    }
    drop(src); // stop a radio before printing
    let Some(st) = last else {
        bail!("no DAB signal found in {seconds} s");
    };
    let e = &fic.ensemble;
    println!(
        "{}  EId {}  ECC {}  {} services, {} sub-channels",
        e.name(),
        e.eid.map_or("?".into(), |v| format!("{v:04X}")),
        e.ecc.map_or("?".into(), |v| format!("{v:02X}")),
        e.services.len(),
        e.subchannels.len()
    );
    if let Some(t) = e.time {
        let (y, m, d) = t.date();
        println!(
            "time {y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
            t.hours, t.minutes
        );
    }
    println!(
        "signal: MER {:.1} dB, CFO {:+.1} Hz, FIBs {}/{} good; {:.1} s in {:.2} s",
        st.mer_db,
        st.cfo_hz,
        fic.stats.fibs_ok,
        fic.stats.fibs,
        read as f64 / rate,
        t0.elapsed().as_secs_f64()
    );
    let c = &fic.counts;
    let list = |name: &str, v: &[u64]| -> String {
        v.iter()
            .enumerate()
            .filter(|(_, n)| **n > 0)
            .map(|(e, n)| format!("{name}/{e}×{n}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    println!(
        "FIGs: {} {} {} (skipped {}, malformed {})",
        list("0", &c.fig0),
        list("1", &c.fig1),
        list("2", &c.fig2),
        c.skipped,
        c.malformed
    );
    println!();
    println!(
        "{:<18} {:<9} {:<6} {:>5} {:>9} {:>4} {:>8}  {:<17} apps",
        "service", "short", "SId", "SubCh", "CUs", "kbps", "prot", "type"
    );
    for s in e.services_by_name() {
        for (i, c) in s.components.iter().enumerate() {
            let kind = match c.transport {
                Transport::Audio { ascty: 63, .. } => "DAB+".to_string(),
                Transport::Audio { ascty: 0, .. } => "DAB (MP2)".to_string(),
                Transport::Audio { ascty, .. } => format!("audio {ascty}"),
                Transport::StreamData { dscty, .. } => format!("stream data {dscty}"),
                Transport::Packet { scid } => format!("packet SCId {scid}"),
                Transport::Fidc { .. } => "FIDC".to_string(),
            };
            let sub = e.subchannel_of(c);
            let apps: Vec<String> = c.apps.iter().map(|a| a.name()).collect();
            let (name, short, sid) = if i == 0 {
                (
                    s.name(),
                    s.label.as_ref().map_or(String::new(), |l| l.short.clone()),
                    format!("{:04X}", s.sid),
                )
            } else {
                (String::new(), String::new(), String::new())
            };
            println!(
                "{:<18} {:<9} {:<6} {:>5} {:>9} {:>4} {:>8}  {:<17} {}{}",
                name,
                short,
                sid,
                sub.map_or("-".into(), |x| x.id.to_string()),
                sub.map_or("-".into(), |x| format!("{}-{}", x.start, x.start + x.size - 1)),
                sub.map_or("-".into(), |x| x.bitrate().to_string()),
                sub.map_or("-".into(), |x| x.protection_name()),
                kind,
                apps.join(", "),
                s.pty
                    .filter(|_| i == 0)
                    .map_or(String::new(), |p| format!("  [{}]", pty_name(p)))
            );
        }
    }
    Ok(())
}
