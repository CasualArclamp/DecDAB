// made by CasualArclamp
//! Draws the app's icon: `cargo run -p decdab-gui --example make_icon`
//! writes `assets/icon.ico` (16 to 256 px, PNG inside) and `assets/icon-256.png`
//! (the window icon, compiled into the app).
//!
//! A rounded near-black square holding a DQPSK scope: the four diagonal
//! clouds of a phase scope, green dots smeared along their radii into an X,
//! over faint axes. At 16 and 24 px, where a cloud of dots does not read,
//! four bold dots on a faint X. Each size is drawn from the shapes'
//! distance functions (not scaled down from the big one), so the edges are
//! as sharp at 16 px as at 256.

use std::io::Cursor;

use image::{ImageFormat, Rgba, RgbaImage};

const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

const GREEN: [f32; 3] = [90.0, 235.0, 120.0];
const TOP: [f32; 3] = [16.0, 22.0, 26.0];
const BOTTOM: [f32; 3] = [4.0, 7.0, 9.0];
const RIM: [f32; 3] = [52.0, 80.0, 64.0];
const AXIS: [f32; 3] = [60.0, 78.0, 70.0];

/// The scope's dots, in the unit square: four diagonal clouds, each point
/// spread more along its radius than across it (as a fading channel's
/// amplitude wanders more than its phase). A fixed generator, so the icon
/// is the same every time.
fn cloud() -> Vec<(f32, f32)> {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut uniform = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f32 / (1u64 << 53) as f32
    };
    let mut gauss = move || {
        let (a, b) = (uniform().max(1e-9), uniform());
        (-2.0 * a.ln()).sqrt() * (std::f32::consts::TAU * b).cos()
    };
    let mut pts = Vec::new();
    for q in 0..4 {
        let ang = std::f32::consts::FRAC_PI_4 + q as f32 * std::f32::consts::FRAC_PI_2;
        let (c, s) = (ang.cos(), ang.sin());
        for _ in 0..340 {
            let r = 0.27 + 0.055 * gauss();
            let t = 0.022 * gauss();
            pts.push((0.5 + r * c - t * s, 0.5 - r * s - t * c));
        }
    }
    pts
}

/// Coverage of a shape whose signed distance (in pixels, negative inside) is
/// `d`: one pixel of anti-aliasing across the edge.
fn cover(d: f32) -> f32 {
    (0.5 - d).clamp(0.0, 1.0)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn render(size: u32) -> RgbaImage {
    let s = size as f32;
    let mut img = RgbaImage::new(size, size);
    // The square: half-side and corner radius, as fractions of the size.
    let (half, radius) = (0.47, 0.21);
    let small = size < 48;
    let pts = if small { Vec::new() } else { cloud() };
    // Each dot's radius: a fraction of the icon, never under 0.6 px.
    let dot_r = (0.0085 * s).max(0.6);
    // The dots' light, accumulated per pixel (they crowd in the clouds' hearts).
    let mut light = vec![0f32; (size * size) as usize];
    for &(cx, cy) in &pts {
        let (px, py) = (cx * s, cy * s);
        let reach = dot_r + 1.0;
        let (x0, x1) = ((px - reach).floor().max(0.0) as u32, ((px + reach).ceil() as u32).min(size - 1));
        let (y0, y1) = ((py - reach).floor().max(0.0) as u32, ((py + reach).ceil() as u32).min(size - 1));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = (x as f32 + 0.5 - px).hypot(y as f32 + 0.5 - py) - dot_r;
                light[(y * size + x) as usize] += cover(d) * 0.55;
            }
        }
    }
    for (x, y, px) in img.enumerate_pixels_mut() {
        let (u, v) = ((x as f32 + 0.5) / s, (y as f32 + 0.5) / s);
        // Signed distance to the rounded square, in pixels.
        let (qx, qy) = (
            (u - 0.5).abs() - (half - radius),
            (v - 0.5).abs() - (half - radius),
        );
        let sd = ((qx.max(0.0)).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius) * s;
        let alpha = cover(sd);
        if alpha <= 0.0 {
            *px = Rgba([0, 0, 0, 0]);
            continue;
        }
        // Diagonal gradient, a thin lighter rim just inside the edge.
        let mut c = mix(TOP, BOTTOM, ((u + v) * 0.5).clamp(0.0, 1.0));
        let rim = (1.0 - (-sd / (0.018 * s).max(1.0)).clamp(0.0, 1.0)) * 0.55;
        c = mix(c, RIM, rim);
        // The scope's axes, inside the square only.
        let inside = (u - 0.5).abs().max((v - 0.5).abs()) < 0.40;
        if inside && !small {
            let w = (0.004 * s).max(0.5);
            let d = ((u - 0.5).abs() * s).min((v - 0.5).abs() * s) - w;
            c = mix(c, AXIS, cover(d) * 0.8);
        }
        if small {
            // A faint X and four bold dots on it.
            let d = ((u - v).abs().min((u + v - 1.0).abs()) * s * std::f32::consts::FRAC_1_SQRT_2) - 0.5;
            if inside {
                c = mix(c, GREEN, cover(d) * 0.35);
            }
            let r = 0.27 * std::f32::consts::FRAC_1_SQRT_2;
            for (cx, cy) in [(0.5 - r, 0.5 - r), (0.5 + r, 0.5 - r), (0.5 - r, 0.5 + r), (0.5 + r, 0.5 + r)] {
                let d = ((u - cx).hypot(v - cy) - (1.6 / s).max(0.075)) * s;
                c = mix(c, GREEN, cover(d));
            }
        } else {
            // The cloud: its light, with a glow around the clouds' hearts.
            let l = light[(y * size + x) as usize];
            c = mix(c, GREEN, (1.0 - (-l).exp()).min(1.0));
        }
        *px = Rgba([
            c[0].round() as u8,
            c[1].round() as u8,
            c[2].round() as u8,
            (alpha * 255.0).round() as u8,
        ]);
    }
    img
}

fn png(img: &RgbaImage) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).expect("PNG");
    out.into_inner()
}

/// An .ico holding each image as a PNG (Windows Vista and later).
fn ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = vec![0, 0, 1, 0];
    out.extend((images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, data) in images {
        // A width of 0 means 256.
        let b = if *size >= 256 { 0 } else { *size as u8 };
        out.extend([b, b, 0, 0, 1, 0, 32, 0]);
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in images {
        out.extend(data);
    }
    out
}

fn main() -> std::io::Result<()> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    std::fs::create_dir_all(&dir)?;
    let images: Vec<(u32, Vec<u8>)> = SIZES.iter().map(|&s| (s, png(&render(s)))).collect();
    std::fs::write(dir.join("icon.ico"), ico(&images))?;
    let big = &images.last().expect("sizes").1;
    std::fs::write(dir.join("icon-256.png"), big)?;
    // A contact sheet for a look: every size on a grey ground.
    let mut sheet = RgbaImage::from_pixel(680, 272, Rgba([58, 58, 62, 255]));
    let mut x = 8;
    for s in [256, 128, 64, 48, 32, 24, 16] {
        image::imageops::overlay(&mut sheet, &render(s), x as i64, 8);
        x += s as i32 + 12;
    }
    let out = dir.join("..").join("..").join("..").join("out");
    std::fs::create_dir_all(&out)?;
    sheet.save(out.join("icon-preview.png")).ok();
    println!("wrote {}", dir.display());
    Ok(())
}
