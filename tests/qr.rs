use kurtz::qr;

/// Decodes the PNG on a white background with a quiet zone, as it appears on
/// the admin's white QR card.
fn decode_png(bytes: &[u8]) -> (u32, String) {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().expect("image fits in memory")];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    let (w, h) = (info.width as usize, info.height as usize);
    assert_eq!(w, h);
    let border = 40;
    let mut img = rqrr::PreparedImage::prepare_from_greyscale(w + 2 * border, h + 2 * border, |x, y| {
        if x < border || y < border || x >= w + border || y >= h + border {
            return 255;
        }
        let i = ((y - border) * w + (x - border)) * 4;
        if buf[i + 3] == 0 {
            255
        } else {
            buf[i]
        }
    });
    let grids = img.detect_grids();
    assert_eq!(grids.len(), 1, "expected one QR code");
    let (_, content) = grids[0].decode().unwrap();
    (w as u32, content)
}

#[test]
fn png_is_512_pixels_and_decodes() {
    let (size, content) = decode_png(&qr::png("https://a.test/abc12").unwrap());
    assert_eq!(size, 512);
    assert_eq!(content, "https://a.test/abc12");
}

#[test]
fn png_background_is_transparent() {
    let bytes = qr::png("https://a.test/abc12").unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(&bytes[..])).read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().expect("image fits in memory")];
    reader.next_frame(&mut buf).unwrap();
    assert!(buf.chunks(4).any(|p| p[3] == 0), "no transparent pixel");
    assert!(buf.chunks(4).any(|p| p == [0, 0, 0, 255]), "no black pixel");
}

#[test]
fn svg_has_black_modules_and_no_background_or_margin() {
    let svg = qr::svg("https://a.test/x").unwrap();
    assert!(svg.starts_with("<svg"));
    assert!(svg.contains("fill=\"#000\""));
    assert!(!svg.contains("<rect"));
    // version 2 code (25 modules), no quiet zone
    assert!(svg.contains("viewBox=\"0 0 25 25\""), "{svg}");
}
