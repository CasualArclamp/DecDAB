<!-- made by CasualArclamp -->
# DecDAB — working notes

Read [`docs/DESIGN.md`](docs/DESIGN.md) first: the agreed scope, the crate
layout and the milestone plan. Update it when a decision changes.
[`docs/STATUS.md`](docs/STATUS.md) logs what is done.

## Commands

```bash
cargo check --workspace --all-targets   # fast feedback
cargo test --workspace --release        # unit tests
cargo clippy --workspace --all-targets --release -- -D warnings
cargo run --release -p decdab-cli -- blocks                           # the Band III plan
cargo run --release -p decdab-cli -- scan --amp                       # HackRF: which blocks carry an ensemble
cargo run --release -p decdab-cli -- capture 12B --amp --seconds 10   # captures/12b_225.648MHz_4.096Msps_amp.cs8
cargo run --release -p decdab-cli -- airspy                           # Airspy R2: board, firmware, sample rates
cargo run --release -p decdab-cli -- info 12B --radio airspy --airspy-preset sensitivity:15   # the same live, from the Airspy (--airspy-lna/-mixer/-vga, --airspy-packed)
cargo run --release -p decdab-cli -- capture 12B --radio airspy --seconds 10   # captures/12b_225.648MHz_4.096Msps_airspy.cs16
cargo test --release -p decdab-io bench_converter -- --ignored --nocapture      # the Airspy's real-to-IQ chain: ns a sample, % of a core
cargo test --release -p decdab-io --features airspy live_airspy -- --ignored --nocapture   # Airspy attached: 3 s stream, retune
cargo run --release -p decdab-cli -- rtlsdr                           # RTL-SDR: the dongle and its tuner (R820T/R828D only)
cargo run --release -p decdab-cli -- info 11D --radio rtlsdr --rtl-gain 40.2   # live from an RTL-SDR (always 2.048 MS/s; --rtl-gain agc, --rtl-ppm N)
cargo test --release -p decdab-io --features rtlsdr live_rtlsdr -- --ignored --nocapture   # RTL-SDR attached: 3 s stream, retune
cargo run --release -p decdab-cli -- info captures/12b_225.648MHz_4.096Msps_amp.cs8   # ensemble, services, sub-channels
cargo run --release -p decdab-cli -- info 11D --amp                   # the same, live
cargo run --release -p decdab-cli -- decode captures/11d_222.064MHz_4.096Msps_amp.cs8 -s "Classic FM" -o out/x.wav   # a service to WAV, CPU per stage
cargo run --release -p decdab-cli -- decode captures/11d_222.064MHz_4.096Msps_amp.cs8 -s "Virgin Radio" -o out/x.wav --mp2-conceal   # MP2: check the header and scale-factor CRCs, conceal what fails (the GUI has a checkbox in the service panel)
cargo run --release -p decdab-gui                                     # the GUI
cargo run --release -p decdab-gui -- captures/11d_long_222.064MHz_4.096Msps_amp.cs8 --play "Classic FM" --screenshot out/x.png --after 10
cargo run --release -p decdab-gui -- --block 11D --play Absolute       # live (needs the HackRF free; --radio airspy for the Airspy)
cargo run --release -p decdab-gui -- CAPTURE --play SERVICE --fps 30 --record-video out/x.mp4 --record-from 5 --record-secs 12   # --bottom scopes|channel|guide|sfn|none (channel: the sounder; guide: SPI schedules; sfn: TII); the GUI records its own frames (ffmpeg on PATH); scripts/promo makes the promo from such clips
cargo run --release -p decdab-engine --example threshold -- captures/11a_long_216.928MHz_4.096Msps_amp.cs8 "absolute 80" 12 -12 -10 -8 -6   # MER where decoding breaks
cargo run --release -p decdab-ofdm --example sound_probe -- captures/11d_long_222.064MHz_4.096Msps_amp.cs8   # the sounder's rows: common phase step and power, row by row (frame-periodic faults show here)
cargo run --release -p decdab-ofdm --example tii_probe -- captures/11a_long_216.928MHz_4.096Msps_amp.cs8 20   # the SFN's transmitters (TII): ids, levels, delays, and the comb × group grid
cargo run --release -p decdab-cli -- decode captures/12b_spi_225.648MHz_2.048Msps_amp.cs8 -s "BBC Guide" --slides out/guide   # a packet-mode data service (SPI): logos, EPG XML and the MOT directory (330 s: the whole carousel)
cargo run -p decdab-gui --example make_icon                           # redraws apps/decdab-gui/assets/icon.ico and icon-256.png (preview in out/)
cargo run --release -p decdab-ofdm --example fic_probe -- captures/12a_223.936MHz_4.096Msps_amp.cs8   # per-frame lock figures and FIB CRCs
```

Captures live in `captures/`, named by their tuning
(`12b_225.648MHz_4.096Msps_amp.cs8`); the tools read the centre and rate
from the name. They are not committed. Band III is weak on this aerial:
record with `--amp --lna 40 --vga 20` (VGA 30 clips 12B).

**GitHub** (public: CasualArclamp/DecDAB; `gh` is at `C:\Program Files\GitHub CLI\gh.exe`).
`ci.yml` runs on every push to main and every PR: Linux and Windows build, tests,
clippy, `scripts/smoke-test.sh`. Run locally only what a change needs and let CI do
the rest. Commits are authored `Arclamp <45412977+CasualArclamp@users.noreply.github.com>`
(set in this repo's git config), never a personal address: the repo is public. Releases: push a
tag `vX.Y.Z` (the workspace version must match); `release.yml` builds the portable
Windows exes, checks and smoke-tests them and drafts the release; then
`gh release edit vX.Y.Z --title … --notes-file … --draft=false --latest`. Locally,
`powershell -ExecutionPolicy Bypass -File scripts\build-portable.ps1` writes the same
exes to `exe\` (git-ignored).

Rust is installed through rustup (stable MSVC). In Git Bash, `~/.cargo/bin`
may need adding to PATH. FDK-AAC builds from `third_party/fdk-aac` (a
submodule; `git submodule update --init`), which takes a minute the first
time.

## House rules

- **Licence:** GPL-3.0-or-later. Code borrowed from DecDVB-T and DecDRM (the
  user's own) and from other GPL/MPL sources is credited in a comment on the
  module or function.
- **The specs are the authority:** EN 300 401, TS 102 563, EN 301 234,
  TS 101 756, ISO/IEC 11172-3. Cite the clause or table next to constants,
  tables and bit layouts.
- Rory has a strong DSP background and is newer to Rust. Explain Rust idioms
  in comments, not DSP basics.
- **CPU efficiency is the main requirement** (DESIGN.md §0). Measure before
  and after on a capture (`decdab decode` prints per-stage CPU).
- **Never commit** IQ captures, decoded audio, built exes or reference clones.
- When Python edits source files, open them with `encoding='utf-8'`.
- The HackRF (and the Airspy and RTL-SDR) may be in use by the user's own DecDVB-T app; ask before
  taking it unless the user has handed it over.
