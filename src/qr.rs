//! QR code rendering as SVG and PNG: black modules on a transparent
//! background, without a quiet zone, so the code fits any surface.

use qrcode::{Color, EcLevel, QrCode};

/// Edge length of the PNG in pixels; the SVG scales freely.
const PNG_SIZE: u32 = 512;

fn code(text: &str) -> Result<QrCode, String> {
    QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M).map_err(|e| e.to_string())
}

/// SVG in module units; it scales freely.
pub fn svg(text: &str) -> Result<String, String> {
    let code = code(text)?;
    let n = code.width() as u32;
    let colors = code.to_colors();
    let mut path = String::new();
    for y in 0..n {
        let mut x = 0;
        while x < n {
            if colors[(y * n + x) as usize] == Color::Dark {
                let start = x;
                while x < n && colors[(y * n + x) as usize] == Color::Dark {
                    x += 1;
                }
                path.push_str(&format!("M{start} {y}h{}v1h-{}z", x - start, x - start));
            } else {
                x += 1;
            }
        }
    }
    Ok(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {n} {n}" width="{PNG_SIZE}" height="{PNG_SIZE}" shape-rendering="crispEdges"><path d="{path}" fill="#000"/></svg>"##
    ))
}

/// Transparent PNG of exactly 512 pixels; modules are whole pixels and the
/// code is centred.
pub fn png(text: &str) -> Result<Vec<u8>, String> {
    let code = code(text)?;
    let n = code.width() as u32;
    let size = PNG_SIZE.max(n);
    let scale = size / n;
    let offset = (size - scale * n) / 2;
    let colors = code.to_colors();

    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let inside = px >= offset && py >= offset && (px - offset) / scale < n && (py - offset) / scale < n;
            let dark = inside && colors[(((py - offset) / scale) * n + (px - offset) / scale) as usize] == Color::Dark;
            pixels.extend_from_slice(if dark { &[0, 0, 0, 255] } else { &[0, 0, 0, 0] });
        }
    }

    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, size, size);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&pixels).map_err(|e| e.to_string())?;
    }
    Ok(out)
}
