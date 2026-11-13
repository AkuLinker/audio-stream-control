//! The icon, drawn in code: one line forks into two.
//! Upper branch = you, lower branch = whoever receives the copy.
//!   sharing: lower branch green
//!   off:     lower branch grey and dotted, its end an empty ring

use std::path::PathBuf;

use tiny_skia::{
    Color, FillRule, LineCap, Paint, PathBuilder, Pixmap, Stroke, StrokeDash, Transform,
};

const TRAY_SIZES: [u32; 6] = [16, 22, 24, 32, 48, 64];
const APP_SIZES: [u32; 7] = [16, 22, 24, 32, 48, 64, 128];

/// Tray icons in several sizes, so the panel can pick a sharp one.
pub fn tray_icons(sharing: bool) -> Vec<ksni::Icon> {
    TRAY_SIZES
        .iter()
        .map(|&s| {
            // premultiplied RGBA → straight ARGB, network byte order
            let mut data = Vec::with_capacity((s * s * 4) as usize);
            for px in draw(s, sharing).pixels() {
                let c = px.demultiply();
                data.extend_from_slice(&[c.alpha(), c.red(), c.green(), c.blue()]);
            }
            ksni::Icon {
                width: s as i32,
                height: s as i32,
                data,
            }
        })
        .collect()
}

/// Window icon, for platforms that let apps set one (X11).
/// On Wayland the desktop takes it from the .desktop file instead (see `install`).
pub fn window_icon() -> eframe::egui::IconData {
    let size = 64;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for px in draw(size, true).pixels() {
        let c = px.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    eframe::egui::IconData {
        rgba,
        width: size,
        height: size,
    }
}

/// Write PNG icons into ~/.local/share/icons/hicolor/<n>x<n>/apps/.
pub fn install(name: &str) -> anyhow::Result<Vec<PathBuf>> {
    let base = data_home().join("icons/hicolor");
    let mut written = Vec::new();
    for s in APP_SIZES {
        let dir = base.join(format!("{s}x{s}/apps"));
        std::fs::create_dir_all(&dir)?;
        let file = dir.join(format!("{name}.png"));
        draw(s, true).save_png(&file)?;
        written.push(file);
    }
    Ok(written)
}

pub fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        })
}

/// Draw the icon at `size`×`size` pixels.
pub fn draw(size: u32, sharing: bool) -> Pixmap {
    let s = size as f32;
    let mut pm = Pixmap::new(size, size).expect("icon size");

    // geometry in 0..1 units, scaled to the icon size
    let p = |x: f32, y: f32| (x * s, y * s);
    let (sx, sy) = p(0.10, 0.50); // start of the stem
    let (jx, jy) = p(0.42, 0.50); // fork point
    let (ux, uy) = p(0.74, 0.26); // upper branch end (you)
    let (lx, ly) = p(0.74, 0.74); // lower branch end (copy)
    let (udx, udy) = p(0.80, 0.22); // upper dot
    let (ldx, ldy) = p(0.80, 0.78); // lower dot
    let w = 0.11 * s; // line width
    let r = 0.10 * s; // dot radius
    let o = (0.05 * s).max(1.0); // dark outline, keeps it visible on light panels

    let white = rgba(0xe8, 0xea, 0xed, 255);
    let green = rgba(0x5a, 0xd1, 0x7a, 255);
    let grey = rgba(0x8a, 0x90, 0x99, 255);
    let outline = rgba(0x10, 0x12, 0x14, 150);

    // 1) dark outline under the parts that are solid
    line(&mut pm, (sx, sy), (jx, jy), outline, w + 2.0 * o, None);
    line(&mut pm, (jx, jy), (ux, uy), outline, w + 2.0 * o, None);
    dot(&mut pm, udx, udy, r + o, outline);
    if sharing {
        line(&mut pm, (jx, jy), (lx, ly), outline, w + 2.0 * o, None);
        dot(&mut pm, ldx, ldy, r + o, outline);
    }

    // 2) lower branch first, so the white stem stays on top at the fork:
    //    green when sharing, dotted grey when off
    if sharing {
        line(&mut pm, (jx, jy), (lx, ly), green, w, None);
        dot(&mut pm, ldx, ldy, r, green);
    } else {
        let dash = StrokeDash::new(vec![0.02 * s, 0.13 * s], 0.0);
        line(&mut pm, (jx, jy), (lx, ly), grey, w * 0.85, dash);
        ring(&mut pm, ldx, ldy, r * 0.8, grey, w * 0.45);
    }

    // 3) stem and upper branch: always white
    line(&mut pm, (sx, sy), (jx, jy), white, w, None);
    line(&mut pm, (jx, jy), (ux, uy), white, w, None);
    dot(&mut pm, udx, udy, r, white);

    pm
}

fn paint(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(c);
    p.anti_alias = true;
    p
}

fn line(
    pm: &mut Pixmap,
    a: (f32, f32),
    b: (f32, f32),
    c: Color,
    width: f32,
    dash: Option<StrokeDash>,
) {
    let mut pb = PathBuilder::new();
    pb.move_to(a.0, a.1);
    pb.line_to(b.0, b.1);
    let path = pb.finish().unwrap();
    let stroke = Stroke {
        width,
        line_cap: LineCap::Round,
        dash,
        ..Default::default()
    };
    pm.stroke_path(&path, &paint(c), &stroke, Transform::identity(), None);
}

fn dot(pm: &mut Pixmap, x: f32, y: f32, radius: f32, c: Color) {
    let path = PathBuilder::from_circle(x, y, radius).unwrap();
    pm.fill_path(
        &path,
        &paint(c),
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

fn ring(pm: &mut Pixmap, x: f32, y: f32, radius: f32, c: Color, width: f32) {
    let path = PathBuilder::from_circle(x, y, radius).unwrap();
    let stroke = Stroke {
        width,
        ..Default::default()
    };
    pm.stroke_path(&path, &paint(c), &stroke, Transform::identity(), None);
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}
