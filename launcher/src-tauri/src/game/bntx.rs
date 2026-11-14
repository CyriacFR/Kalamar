//! BNTX texture containers: parse the BRTI texture headers, undo the Tegra X1 block-linear swizzle
//! (`tegra_swizzle`) and decode the pixel formats these UIs use (`texture2ddecoder`) to RGBA8.
//!
//! The third-party calls run under [`guard::catch`] (texture2ddecoder 0.1.2 panics on some malformed ASTC blocks), and
//! a texture is decoded only up to [`MAX_DIM`] pixels a side and [`MAX_PIXELS`] in all, its buffers reserved with
//! [`guard::filled`].

use super::guard;
use tegra_swizzle::{BlockHeight, surface::BlockDim, surface::deswizzle_surface};

/// Largest texture side decoded. Every `UI/Icon` texture of 11.2.0 is at most 1530x830 (stage pictures: 640x360), the
/// creation-screen icons taken from layouts at most 172x256.
pub const MAX_DIM: u32 = 2048;
/// Largest texture area decoded (2 Mpx: 1920x1080 fits; 11.2.0's largest icon has 1.27 Mpx). Decoding one costs about
/// 30 bytes per pixel at most, scaling it another 16.
pub const MAX_PIXELS: u64 = 1 << 21;

#[derive(Debug, Clone)]
pub struct Texture {
    pub name: String,
    /// `(type << 8) | variant`, for example `0x2d06` = ASTC 4x4 sRGB.
    pub format: u32,
    pub width: u32,
    pub height: u32,
    pub mips: u32,
    pub layers: u32,
    pub block_height_log2: u32,
    /// Channel selectors, one byte each in R, G, B, A order: 0 = zero, 1 = one, 2..5 = source R, G, B, A.
    pub channels: [u8; 4],
    pub data_offset: usize,
    pub data_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    R8,
    Rg8,
    Rgba8,
    Bgra8,
    Bc1,
    Bc2,
    Bc3,
    Bc4,
    Bc5,
    Bc7,
    Astc(u32, u32),
}

pub fn format_name(fmt: u32) -> String {
    let srgb = fmt & 0xFF == 6;
    let base = match kind_of(fmt) {
        Some(Kind::R8) => "R8".to_string(),
        Some(Kind::Rg8) => "R8G8".into(),
        Some(Kind::Rgba8) => "RGBA8".into(),
        Some(Kind::Bgra8) => "BGRA8".into(),
        Some(Kind::Bc1) => "BC1".into(),
        Some(Kind::Bc2) => "BC2".into(),
        Some(Kind::Bc3) => "BC3".into(),
        Some(Kind::Bc4) => "BC4".into(),
        Some(Kind::Bc5) => "BC5".into(),
        Some(Kind::Bc7) => "BC7".into(),
        Some(Kind::Astc(w, h)) => format!("ASTC{w}x{h}"),
        None => format!("unknown {fmt:#06x}"),
    };
    if srgb { format!("{base} sRGB") } else { base }
}

fn kind_of(fmt: u32) -> Option<Kind> {
    Some(match fmt >> 8 {
        0x02 => Kind::R8,
        0x09 => Kind::Rg8,
        0x0b => Kind::Rgba8,
        0x0c => Kind::Bgra8,
        0x1a => Kind::Bc1,
        0x1b => Kind::Bc2,
        0x1c => Kind::Bc3,
        0x1d => Kind::Bc4,
        0x1e => Kind::Bc5,
        0x20 => Kind::Bc7,
        t @ 0x2d..=0x3a => {
            const DIMS: [(u32, u32); 14] = [(4, 4), (5, 4), (5, 5), (6, 5), (6, 6), (8, 5), (8, 6), (8, 8), (10, 5), (10, 6), (10, 8), (10, 10), (12, 10), (12, 12)];
            let (w, h) = DIMS[(t - 0x2d) as usize];
            Kind::Astc(w, h)
        }
        _ => return None,
    })
}

fn rd<const N: usize>(b: &[u8], o: usize) -> Result<[u8; N], String> {
    o.checked_add(N).and_then(|e| b.get(o..e)).map(|s| s.try_into().unwrap()).ok_or_else(|| "BNTX: truncated".to_string())
}
fn u16at(b: &[u8], o: usize) -> Result<u32, String> {
    Ok(u16::from_le_bytes(rd(b, o)?) as u32)
}
fn u32at(b: &[u8], o: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(rd(b, o)?))
}
/// An offset into the file: past its end is an error, so adding a field offset to it never overflows.
fn u64at(b: &[u8], o: usize) -> Result<usize, String> {
    usize::try_from(u64::from_le_bytes(rd(b, o)?)).ok().filter(|v| *v <= b.len()).ok_or_else(|| "BNTX: offset out of range".to_string())
}

/// Lists the textures of a BNTX container.
pub fn parse(b: &[u8]) -> Result<Vec<Texture>, String> {
    if b.get(..4) != Some(b"BNTX") {
        return Err("BNTX: bad magic".into());
    }
    let count = u32at(b, 0x24)? as usize;
    let table = u64at(b, 0x28)?;
    if count > 4096 {
        return Err("BNTX: too many textures".into());
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let p = u64at(b, table + 8 * i)?;
        if b.get(p..p + 4) != Some(b"BRTI") {
            return Err("BNTX: BRTI missing".into());
        }
        let mips = u16at(b, p + 0x16)?;
        let format = u32at(b, p + 0x1C)?;
        let (width, height) = (u32at(b, p + 0x24)?, u32at(b, p + 0x28)?);
        let layers = u32at(b, p + 0x30)?.max(1);
        let layout = u32at(b, p + 0x34)?;
        let size = u32at(b, p + 0x50)? as usize;
        let chan = u32at(b, p + 0x58)?.to_le_bytes();
        let name_at = u64at(b, p + 0x60)?;
        let ptrs = u64at(b, p + 0x70)?;
        let data_offset = u64at(b, ptrs)?;
        let nl = u16at(b, name_at)? as usize;
        let name = String::from_utf8_lossy(b.get(name_at + 2..name_at + 2 + nl).ok_or("BNTX: bad name")?).into_owned();
        if data_offset.checked_add(size).is_none_or(|e| e > b.len()) {
            return Err(format!("BNTX: texture {name} data out of range"));
        }
        // The channel word stores the selectors in R, G, B, A order, one byte each (identity = 02 03 04 05).
        out.push(Texture { name, format, width, height, mips, layers, block_height_log2: layout & 7, channels: chan, data_offset, data_len: size });
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Decodes mip 0 of the first layer to RGBA8 (sRGB-encoded values are left as stored).
pub fn decode(b: &[u8], t: &Texture) -> Result<Rgba, String> {
    let kind = kind_of(t.format).ok_or_else(|| format!("unsupported texture format {:#06x}", t.format))?;
    let (w, h) = (t.width as usize, t.height as usize);
    if w == 0 || h == 0 || t.width > MAX_DIM || t.height > MAX_DIM || t.width as u64 * t.height as u64 > MAX_PIXELS {
        return Err(format!("texture size {w}x{h} out of the supported range"));
    }
    let (block, bpp) = match kind {
        Kind::R8 => ((1, 1), 1),
        Kind::Rg8 => ((1, 1), 2),
        Kind::Rgba8 | Kind::Bgra8 => ((1, 1), 4),
        Kind::Bc1 | Kind::Bc4 => ((4, 4), 8),
        Kind::Bc2 | Kind::Bc3 | Kind::Bc5 | Kind::Bc7 => ((4, 4), 16),
        Kind::Astc(bw, bh) => ((bw, bh), 16),
    };
    let dim = |w, h| {
        use std::num::NonZeroU32;
        BlockDim { width: NonZeroU32::new(w).unwrap(), height: NonZeroU32::new(h).unwrap(), depth: NonZeroU32::new(1).unwrap() }
    };
    let block_height = BlockHeight::new(1 << (t.block_height_log2 & 7)).ok_or("unsupported block height")?;
    let raw = t.data_offset.checked_add(t.data_len).and_then(|end| b.get(t.data_offset..end)).ok_or("texture data out of range")?;
    let linear = guard::catch("deswizzle", || {
        deswizzle_surface(t.width, t.height, 1, raw, dim(block.0, block.1), Some(block_height), bpp, 1, 1).map_err(|e| format!("deswizzle: {e:?}"))
    })?;

    // Decode to BGRA words as texture2ddecoder does, then to RGBA bytes.
    let mut rgba = guard::filled(w * h * 4, 0u8)?;
    match kind {
        Kind::R8 | Kind::Rg8 | Kind::Rgba8 | Kind::Bgra8 => {
            let step = bpp as usize;
            if linear.len() < w * h * step {
                return Err("short surface".into());
            }
            for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
                let s = &linear[i * step..i * step + step];
                px.copy_from_slice(&match kind {
                    Kind::R8 => [s[0], 0, 0, 255],
                    Kind::Rg8 => [s[0], s[1], 0, 255],
                    Kind::Rgba8 => [s[0], s[1], s[2], s[3]],
                    _ => [s[2], s[1], s[0], s[3]],
                });
            }
        }
        _ => {
            let mut words = guard::filled(w * h, 0u32)?;
            guard::catch("decode", || {
                match kind {
                    Kind::Bc1 => texture2ddecoder::decode_bc1(&linear, w, h, &mut words),
                    Kind::Bc2 => texture2ddecoder::decode_bc2(&linear, w, h, &mut words),
                    Kind::Bc3 => texture2ddecoder::decode_bc3(&linear, w, h, &mut words),
                    Kind::Bc4 => texture2ddecoder::decode_bc4(&linear, w, h, &mut words),
                    Kind::Bc5 => texture2ddecoder::decode_bc5(&linear, w, h, &mut words),
                    Kind::Bc7 => texture2ddecoder::decode_bc7(&linear, w, h, &mut words),
                    Kind::Astc(bw, bh) => texture2ddecoder::decode_astc(&linear, w, h, bw as usize, bh as usize, &mut words),
                    _ => Err("not a block format"),
                }
                .map_err(|e| format!("decode: {e}"))
            })?;
            for (px, wd) in rgba.chunks_exact_mut(4).zip(words) {
                let [bl, g, r, a] = wd.to_le_bytes();
                // One- and two-channel block formats behave like the GPU: missing channels read 0, alpha reads 1.
                let a = if matches!(kind, Kind::Bc4 | Kind::Bc5) { 255 } else { a };
                px.copy_from_slice(&[r, g, bl, a]);
            }
        }
    }
    apply_channel_select(&mut rgba, t.channels);
    Ok(Rgba { width: t.width, height: t.height, pixels: rgba })
}

fn apply_channel_select(px: &mut [u8], sel: [u8; 4]) {
    if sel == [2, 3, 4, 5] {
        return;
    }
    for p in px.chunks_exact_mut(4) {
        let src = [p[0], p[1], p[2], p[3]];
        for (i, s) in sel.iter().enumerate() {
            p[i] = match s {
                0 => 0,
                1 => 255,
                2..=5 => src[(*s - 2) as usize],
                _ => src[i],
            };
        }
    }
}

/// sRGB byte -> linear light.
fn srgb_to_linear(v: u8) -> f32 {
    let c = v as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Linear light -> sRGB byte.
fn linear_to_srgb(v: f32) -> u8 {
    let c = v.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

/// One axis of an area (box) resample: `n` lines of `src_len` premultiplied pixels to `dst_len` (`dst_len < src_len`),
/// each output pixel the average of the source pixels it covers, partial ones weighted by their coverage.
fn shrink_axis(src: &[[f32; 4]], src_len: usize, dst_len: usize, lines: usize, stride_in_line: usize, line_stride: usize) -> Vec<[f32; 4]> {
    let scale = src_len as f64 / dst_len as f64;
    let mut out = vec![[0f32; 4]; dst_len * lines];
    for line in 0..lines {
        for d in 0..dst_len {
            let (a, b) = (d as f64 * scale, (d + 1) as f64 * scale);
            let mut acc = [0f64; 4];
            let mut i = a.floor() as usize;
            while (i as f64) < b && i < src_len {
                let w = (b.min((i + 1) as f64) - a.max(i as f64)).max(0.0);
                let p = src[line * line_stride + i * stride_in_line];
                for k in 0..4 {
                    acc[k] += p[k] as f64 * w;
                }
                i += 1;
            }
            let o = &mut out[line * dst_len + d];
            for k in 0..4 {
                o[k] = (acc[k] / scale) as f32;
            }
        }
    }
    out
}

/// Scales an image down to `width` (the height keeps the proportions), averaging in linear light with premultiplied
/// alpha. An image already that narrow (or narrower) is returned as it is: pictures are never enlarged.
pub fn scale_to_width(img: &Rgba, width: u32) -> Rgba {
    if width == 0 || width >= img.width || img.height == 0 {
        return img.clone();
    }
    let (sw, sh) = (img.width as usize, img.height as usize);
    let dw = width as usize;
    let dh = ((sh as u64 * dw as u64 + sw as u64 / 2) / sw as u64).max(1) as usize;
    let lut: Vec<f32> = (0..=255u8).map(srgb_to_linear).collect();
    let px: Vec<[f32; 4]> = img
        .pixels
        .chunks_exact(4)
        .map(|p| {
            let a = p[3] as f32 / 255.0;
            [lut[p[0] as usize] * a, lut[p[1] as usize] * a, lut[p[2] as usize] * a, a]
        })
        .collect();
    // Rows first (sh lines of sw pixels), then columns (dw lines of sh pixels, read down the intermediate image).
    let rows = shrink_axis(&px, sw, dw, sh, 1, sw);
    let cols = shrink_axis(&rows, sh, dh, dw, dw, 1);
    let mut pixels = vec![0u8; dw * dh * 4];
    for x in 0..dw {
        for y in 0..dh {
            let p = cols[x * dh + y];
            let o = &mut pixels[(y * dw + x) * 4..][..4];
            let a = p[3];
            if a > 0.0 {
                for k in 0..3 {
                    o[k] = linear_to_srgb(p[k] / a);
                }
            }
            o[3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    Rgba { width: dw as u32, height: dh as u32, pixels }
}

pub fn encode_png(img: &Rgba) -> Result<Vec<u8>, String> {
    guard::catch("png", || {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, img.width, img.height);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::Fast);
            let mut w = enc.write_header().map_err(|e| e.to_string())?;
            w.write_image_data(&img.pixels).map_err(|e| e.to_string())?;
        }
        Ok(out)
    })
}

#[cfg(test)]
pub mod build {
    /// Builds a one-texture BNTX around already swizzled data.
    pub fn container(name: &str, fmt: u32, w: u32, h: u32, bh_log2: u32, chan: [u8; 4], data: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; 0x1000];
        b[..4].copy_from_slice(b"BNTX");
        b[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
        b[0x28..0x30].copy_from_slice(&0x100u64.to_le_bytes());
        b[0x100..0x108].copy_from_slice(&0x200u64.to_le_bytes());
        let p = 0x200;
        b[p..p + 4].copy_from_slice(b"BRTI");
        b[p + 0x16..p + 0x18].copy_from_slice(&1u16.to_le_bytes());
        b[p + 0x1C..p + 0x20].copy_from_slice(&fmt.to_le_bytes());
        b[p + 0x24..p + 0x28].copy_from_slice(&w.to_le_bytes());
        b[p + 0x28..p + 0x2C].copy_from_slice(&h.to_le_bytes());
        b[p + 0x30..p + 0x34].copy_from_slice(&1u32.to_le_bytes());
        b[p + 0x34..p + 0x38].copy_from_slice(&bh_log2.to_le_bytes());
        b[p + 0x50..p + 0x54].copy_from_slice(&(data.len() as u32).to_le_bytes());
        b[p + 0x58..p + 0x5C].copy_from_slice(&chan);
        b[p + 0x60..p + 0x68].copy_from_slice(&0x300u64.to_le_bytes());
        b[p + 0x70..p + 0x78].copy_from_slice(&0x400u64.to_le_bytes());
        b[0x300..0x302].copy_from_slice(&(name.len() as u16).to_le_bytes());
        b[0x302..0x302 + name.len()].copy_from_slice(name.as_bytes());
        b[0x400..0x408].copy_from_slice(&0x1000u64.to_le_bytes());
        b.extend_from_slice(data);
        b
    }

    /// A one-texture RGBA8 BNTX of a linear picture (swizzled as the console stores it).
    pub fn rgba8(name: &str, w: u32, h: u32, linear: &[u8]) -> Vec<u8> {
        use tegra_swizzle::{BlockHeight, surface::BlockDim, surface::swizzle_surface};
        let swz = swizzle_surface(w, h, 1, linear, BlockDim::uncompressed(), Some(BlockHeight::One), 4, 1, 1).unwrap();
        container(name, 0x0b01, w, h, 0, [2, 3, 4, 5], &swz)
    }
}

#[cfg(test)]
mod tests {
    use super::build::container;
    use super::*;
    use tegra_swizzle::surface::swizzle_surface;

    #[test]
    fn rgba8_roundtrip_through_the_swizzle() {
        let (w, h) = (20u32, 17u32);
        let linear: Vec<u8> = (0..w * h * 4).map(|i| (i * 7 % 251) as u8).collect();
        let swz = swizzle_surface(w, h, 1, &linear, BlockDim::uncompressed(), Some(BlockHeight::Two), 4, 1, 1).unwrap();
        let bntx = container("t", 0x0b01, w, h, 1, [2, 3, 4, 5], &swz);
        let tex = parse(&bntx).unwrap();
        assert_eq!(tex.len(), 1);
        assert_eq!(tex[0].name, "t");
        assert_eq!(format_name(tex[0].format), "RGBA8");
        let img = decode(&bntx, &tex[0]).unwrap();
        assert_eq!((img.width, img.height), (w, h));
        assert_eq!(img.pixels, linear);
        let png = encode_png(&img).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn bc4_single_channel_with_channel_select() {
        // One 4x4 BC4 block: both endpoints 200, all indices 0 -> every texel is 200.
        let block = [200u8, 200, 0, 0, 0, 0, 0, 0];
        let (w, h) = (4u32, 4u32);
        let swz = swizzle_surface(w, h, 1, &block, BlockDim::block_4x4(), Some(BlockHeight::One), 8, 1, 1).unwrap();
        let bntx = container("m", 0x1d01, w, h, 0, [2, 2, 2, 2], &swz);
        let t = parse(&bntx).unwrap();
        let img = decode(&bntx, &t[0]).unwrap();
        assert!(img.pixels.chunks(4).all(|p| p == [200, 200, 200, 200]));
        let bntx = container("m", 0x1d01, w, h, 0, [2, 3, 4, 5], &swz);
        let t = parse(&bntx).unwrap();
        let img = decode(&bntx, &t[0]).unwrap();
        assert!(img.pixels.chunks(4).all(|p| p == [200, 0, 0, 255]));
    }

    #[test]
    fn astc_4x4_constant_block() {
        // ASTC void-extent block, LDR: constant color (R,G,B,A) = 0xFFFF,0,0,0xFFFF.
        let mut blk = [0u8; 16];
        blk[0] = 0xFC; // void extent LDR marker
        blk[1] = 0xFD;
        blk[2..8].copy_from_slice(&[0xFF; 6]); // extent: all ones = no extent
        blk[8..10].copy_from_slice(&0xFFFFu16.to_le_bytes()); // R
        blk[10..12].copy_from_slice(&0u16.to_le_bytes()); // G
        blk[12..14].copy_from_slice(&0u16.to_le_bytes()); // B
        blk[14..16].copy_from_slice(&0xFFFFu16.to_le_bytes()); // A
        let dim = BlockDim { width: 4.try_into().unwrap(), height: 4.try_into().unwrap(), depth: 1.try_into().unwrap() };
        let swz = swizzle_surface(4, 4, 1, &blk, dim, Some(BlockHeight::One), 16, 1, 1).unwrap();
        let bntx = container("a", 0x2d06, 4, 4, 0, [2, 3, 4, 5], &swz);
        let t = parse(&bntx).unwrap();
        assert_eq!(format_name(t[0].format), "ASTC4x4 sRGB");
        let img = decode(&bntx, &t[0]).unwrap();
        assert!(img.pixels.chunks(4).all(|p| p == [255, 0, 0, 255]), "{:?}", &img.pixels[..8]);
    }

    #[test]
    fn scaling_down_averages_in_linear_light_and_keeps_the_proportions() {
        // 4x2, left half black, right half white -> 2x1: one black and one white pixel.
        let pixels: Vec<u8> = (0..8).flat_map(|i| if i % 4 < 2 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }).collect();
        let s = scale_to_width(&Rgba { width: 4, height: 2, pixels }, 2);
        assert_eq!((s.width, s.height), (2, 1));
        assert_eq!(s.pixels, vec![0, 0, 0, 255, 255, 255, 255, 255]);
        // A black and white checker becomes half the light (sRGB 188), not the byte average 128.
        let checker: Vec<u8> = (0..4).flat_map(|i| if i % 3 == 0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }).collect();
        let s = scale_to_width(&Rgba { width: 2, height: 2, pixels: checker }, 1);
        assert_eq!((s.width, s.height, &s.pixels[..]), (1, 1, &[188u8, 188, 188, 255][..]));
        // Never enlarged; a 16:9 picture stays 16:9.
        assert_eq!(scale_to_width(&s, 480).width, 1);
        let flat = Rgba { width: 640, height: 360, pixels: [90u8, 140, 200, 255].repeat(640 * 360) };
        let s = scale_to_width(&flat, 480);
        assert_eq!((s.width, s.height), (480, 270));
        assert!(s.pixels.chunks(4).all(|p| p == [90, 140, 200, 255]), "{:?}", &s.pixels[..4]);
        assert_eq!((scale_to_width(&flat, 160).width, scale_to_width(&flat, 160).height), (160, 90));
    }

    #[test]
    fn rejects_bad_containers() {
        assert!(parse(b"").is_err());
        assert!(parse(b"BNTX").is_err());
        let mut b = container("t", 0x0b01, 4, 4, 0, [2, 3, 4, 5], &[0; 64]);
        b[0x200 + 0x50..0x200 + 0x54].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
        assert!(parse(&b).is_err());
        let b = container("t", 0x7f01, 4, 4, 0, [2, 3, 4, 5], &[0; 64]);
        let t = parse(&b).unwrap();
        assert!(decode(&b, &t[0]).unwrap_err().contains("unsupported"));
        // An offset near the top of the address space is out of range, not an overflow.
        for at in [0x28, 0x100, 0x200 + 0x60, 0x200 + 0x70, 0x400] {
            let mut b = container("t", 0x0b01, 4, 4, 0, [2, 3, 4, 5], &[0; 64]);
            b[at..at + 8].copy_from_slice(&(u64::MAX - 2).to_le_bytes());
            assert!(parse(&b).unwrap_err().contains("out of range"), "{at:#x}");
        }
    }

    /// A one-texture ASTC 4x4 sRGB container holding one 16-byte block.
    fn astc_block(block: &[u8; 16]) -> Vec<u8> {
        let dim = BlockDim { width: 4.try_into().unwrap(), height: 4.try_into().unwrap(), depth: 1.try_into().unwrap() };
        let swz = swizzle_surface(4, 4, 1, block, dim, Some(BlockHeight::One), 16, 1, 1).unwrap();
        container("a", 0x2d06, 4, 4, 0, [2, 3, 4, 5], &swz)
    }

    fn hex16(s: &str) -> [u8; 16] {
        let v: Vec<u8> = (0..16).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect();
        v.try_into().unwrap()
    }

    #[test]
    fn malformed_astc_blocks_are_errors_not_panics() {
        // Blocks on which texture2ddecoder 0.1.2 panics (bitreader.rs:5, astc.rs:1493 and 1494).
        for blk in ["7cfdffffffffffff0000000000000000", "e585a0e462c03b1d516e89b52735ae96", "a40d068ce746a24f9781dcc1ec01ef95"] {
            let b = astc_block(&hex16(blk));
            let t = parse(&b).unwrap();
            let e = decode(&b, &t[0]).unwrap_err();
            assert!(e.starts_with("decode: invalid data ("), "{blk}: {e}");
        }
        // The thread goes on decoding: a sound block right after.
        let mut ok = [0u8; 16];
        ok[..2].copy_from_slice(&[0xFC, 0xFD]);
        ok[2..8].copy_from_slice(&[0xFF; 6]);
        let b = astc_block(&ok);
        assert!(decode(&b, &parse(&b).unwrap()[0]).is_ok());
    }

    #[test]
    fn textures_past_the_size_limits_are_refused_before_decoding() {
        for (w, h) in [(MAX_DIM + 1, 4), (4, MAX_DIM + 1), (MAX_DIM, (MAX_PIXELS / MAX_DIM as u64) as u32 + 1), (8192, 8192)] {
            let b = container("t", 0x1a01, w, h, 0, [2, 3, 4, 5], &[0; 64]);
            let t = parse(&b).unwrap();
            assert!(decode(&b, &t[0]).unwrap_err().contains("out of the supported range"), "{w}x{h}");
        }
        assert_eq!(MAX_PIXELS, 2048 * 1024);
    }
}
