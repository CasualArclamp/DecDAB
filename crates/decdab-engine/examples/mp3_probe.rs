// made by CasualArclamp
//! Records one service of a capture to MP3 through the engine's command path
//! (what the GUI's MP3 button does) and checks the file.
//!
//! `cargo run --release -p decdab-engine --example mp3_probe -- CAPTURE.cs8 "Classic FM" out/x.mp3`

use std::time::{Duration, Instant};

use decdab_core::SampleFormat;
use decdab_engine::{Command, Rx, RxOptions};
use decdab_io::{IqFileReader, IqSource, format_from_path, name_hint};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (path, name, out) = (&args[0], &args[1], std::path::PathBuf::from(&args[2]));
    let hint = name_hint(std::path::Path::new(path));
    let (centre, rate) = (hint.center_hz.unwrap(), hint.sample_rate.unwrap());
    let fmt = format_from_path(std::path::Path::new(path)).unwrap_or(SampleFormat::Cs8);
    let src = IqFileReader::open(std::path::Path::new(path), fmt, rate, 1 << 15)
        .unwrap()
        .with_center_freq(centre);
    let src: Box<dyn IqSource> = Box::new(src);
    let rx = Rx::start(
        src,
        RxOptions {
            sound: false,
            ..RxOptions::default()
        },
        None,
    );
    rx.send(Command::Decode(Some(centre)));

    let t0 = Instant::now();
    let sid = loop {
        let u = rx.take_dab();
        if let Some(e) = u.ensemble
            && let Some((sid, _)) = e.services.iter().find(|(_, s)| s.name().contains(name))
        {
            break *sid;
        }
        assert!(t0.elapsed() < Duration::from_secs(30), "no ensemble");
        std::thread::sleep(Duration::from_millis(100));
    };
    rx.send(Command::Select(Some(sid)));
    while rx.take_dab().status.playing != Some(sid) {
        assert!(t0.elapsed() < Duration::from_secs(40), "service not playing");
        std::thread::sleep(Duration::from_millis(100));
    }
    rx.send(Command::RecordMp3(Some(out.clone())));
    std::thread::sleep(Duration::from_secs(6));
    println!("recording: {:?}", rx.stats().mp3_recording);
    rx.send(Command::RecordMp3(None));
    std::thread::sleep(Duration::from_millis(500));
    println!(
        "after stop: {:?}, error {:?}",
        rx.stats().mp3_recording,
        rx.stats().error
    );
    println!("{} bytes", std::fs::metadata(&out).map_or(0, |m| m.len()));
}
