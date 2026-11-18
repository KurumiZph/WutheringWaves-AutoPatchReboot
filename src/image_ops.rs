//! Pixel helpers on BGRA frames (pure Rust, no Win32).

use std::io::BufWriter;
use std::path::Path;

/// Max luminance spread (0-255) across a sparse sample for a frame to count as blank.
const BLANK_FRAME_SPREAD: u32 = 4;

pub struct Frame {
    pub width: i32,
    pub height: i32,
    /// width * height * 4 bytes, B G R A (alpha forced to 255)
    pub bgra: Vec<u8>,
}

impl Frame {
    /// Box-filter downscale to `target_h` pixels of height (aspect ratio kept). Never upscales.
    pub fn resized_to_height(&self, target_h: i32) -> Frame {
        if target_h <= 0 || self.height <= target_h {
            return Frame { width: self.width, height: self.height, bgra: self.bgra.clone() };
        }
        let scale = target_h as f32 / self.height as f32;
        let dw = ((self.width as f32 * scale).round() as usize).max(1);
        let dh = target_h as usize;
        let (sw, sh) = (self.width as usize, self.height as usize);
        let mut out = vec![255u8; dw * dh * 4];
        for dy in 0..dh {
            let y0 = dy * sh / dh;
            let y1 = ((dy + 1) * sh / dh).max(y0 + 1).min(sh);
            for dx in 0..dw {
                let x0 = dx * sw / dw;
                let x1 = ((dx + 1) * sw / dw).max(x0 + 1).min(sw);
                let (mut b, mut g, mut r, mut n) = (0u32, 0u32, 0u32, 0u32);
                for y in y0..y1 {
                    let row = y * sw * 4;
                    for x in x0..x1 {
                        let i = row + x * 4;
                        b += self.bgra[i] as u32;
                        g += self.bgra[i + 1] as u32;
                        r += self.bgra[i + 2] as u32;
                        n += 1;
                    }
                }
                let o = (dy * dw + dx) * 4;
                out[o] = (b / n) as u8;
                out[o + 1] = (g / n) as u8;
                out[o + 2] = (r / n) as u8;
            }
        }
        Frame { width: dw as i32, height: dh as i32, bgra: out }
    }

    /// True if the frame is (almost) one flat colour: a failed/black capture, a minimised window or a
    /// fade-to-black. Such frames carry nothing to OCR and must not count as "nothing detected".
    pub fn looks_blank(&self) -> bool {
        let (w, h) = (self.width.max(0) as usize, self.height.max(0) as usize);
        if w == 0 || h == 0 || self.bgra.len() < w * h * 4 {
            return true;
        }
        let (gx, gy) = (64usize, 36usize);
        let (mut lo, mut hi) = (255u32, 0u32);
        for iy in 0..gy {
            let y = (2 * iy + 1) * h / (2 * gy);
            for ix in 0..gx {
                let x = (2 * ix + 1) * w / (2 * gx);
                let i = (y * w + x) * 4;
                let lum = (29 * self.bgra[i] as u32 + 150 * self.bgra[i + 1] as u32 + 77 * self.bgra[i + 2] as u32) >> 8;
                lo = lo.min(lum);
                hi = hi.max(lum);
            }
        }
        hi - lo <= BLANK_FRAME_SPREAD
    }

    /// Paints black rectangles. Boxes are (left, top, width, height) in the coordinates
    /// of the OCR'd image, which may be smaller than this frame.
    pub fn blackout(&mut self, boxes: &[(f32, f32, f32, f32)], ocr_width: i32) {
        let scale = self.width as f32 / ocr_width.max(1) as f32;
        let pad = 4.0;
        let (w, h) = (self.width as i64, self.height as i64);
        for &(l, t, bw, bh) in boxes {
            let x0 = (((l - pad) * scale) as i64).clamp(0, w);
            let y0 = (((t - pad) * scale) as i64).clamp(0, h);
            let x1 = (((l + bw + pad) * scale) as i64).clamp(0, w);
            let y1 = (((t + bh + pad) * scale) as i64).clamp(0, h);
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = ((y * w + x) * 4) as usize;
                    self.bgra[i] = 0;
                    self.bgra[i + 1] = 0;
                    self.bgra[i + 2] = 0;
                }
            }
        }
    }

    pub fn save_png(&self, path: &Path) -> Result<(), String> {
        let mut rgb = Vec::with_capacity(self.bgra.len() / 4 * 3);
        for px in self.bgra.chunks_exact(4) {
            rgb.push(px[2]);
            rgb.push(px[1]);
            rgb.push(px[0]);
        }
        let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
        let mut encoder = png::Encoder::new(BufWriter::new(file), self.width as u32, self.height as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgb).map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Where a close-up crop of the frame sits, so boxes found in the crop can be mapped back to the frame.
#[derive(Clone, Copy, Debug)]
pub struct CropMap {
    /// crop origin in frame pixels
    pub x0: f32,
    pub y0: f32,
    /// the crop was enlarged by this factor
    pub factor: f32,
    /// pixels of edge padding added around the enlarged crop
    pub pad: f32,
}

impl CropMap {
    /// A box in the enlarged crop's pixels -> the same box in frame pixels (left, top, width, height).
    pub fn to_frame(&self, l: f32, t: f32, w: f32, h: f32) -> (f32, f32, f32, f32) {
        (self.x0 + (l - self.pad) / self.factor, self.y0 + (t - self.pad) / self.factor, w / self.factor, h / self.factor)
    }
}

/// Part of the screen where the game shows the user id (fractions of the window).
pub const UID_CORNER: (f32, f32, f32, f32) = (0.62, 0.90, 1.0, 1.0);
/// The enlarged corner should end up about this tall (px): small text reads far better at that size.
const UID_CORNER_TARGET_H: f32 = 300.0;
/// Edge padding (px) around the enlarged crop, so text touching the screen edge is not clipped by the OCR.
const UID_CORNER_PAD: i32 = 24;

impl Frame {
    /// A full-resolution crop of the bottom-right corner (where the user id is), enlarged and padded for OCR.
    pub fn uid_corner(&self) -> (Frame, CropMap) {
        let (fx0, fy0, fx1, fy1) = UID_CORNER;
        let x0 = ((self.width as f32 * fx0) as i32).clamp(0, self.width.max(1) - 1);
        let y0 = ((self.height as f32 * fy0) as i32).clamp(0, self.height.max(1) - 1);
        let x1 = ((self.width as f32 * fx1).round() as i32).clamp(x0 + 1, self.width.max(1));
        let y1 = ((self.height as f32 * fy1).round() as i32).clamp(y0 + 1, self.height.max(1));
        let (cw, ch) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let factor = (UID_CORNER_TARGET_H / ch as f32).round().clamp(1.0, 4.0) as usize;
        let pad = UID_CORNER_PAD as usize;
        let (ow, oh) = (cw * factor + pad * 2, ch * factor + pad * 2);
        let sw = self.width as usize;
        let mut out = vec![255u8; ow * oh * 4];
        // bilinear sample of the crop; positions outside the enlarged crop clamp to its nearest edge pixel
        for oy in 0..oh {
            let fy = ((oy as f32 - pad as f32 + 0.5) / factor as f32 - 0.5).clamp(0.0, (ch - 1) as f32);
            let (iy0, ty) = (fy.floor() as usize, fy - fy.floor());
            let iy1 = (iy0 + 1).min(ch - 1);
            for ox in 0..ow {
                let fx = ((ox as f32 - pad as f32 + 0.5) / factor as f32 - 0.5).clamp(0.0, (cw - 1) as f32);
                let (ix0, tx) = (fx.floor() as usize, fx - fx.floor());
                let ix1 = (ix0 + 1).min(cw - 1);
                let px = |ix: usize, iy: usize, c: usize| -> f32 {
                    self.bgra[((y0 as usize + iy) * sw + x0 as usize + ix) * 4 + c] as f32
                };
                let o = (oy * ow + ox) * 4;
                for c in 0..3 {
                    let top = px(ix0, iy0, c) * (1.0 - tx) + px(ix1, iy0, c) * tx;
                    let bot = px(ix0, iy1, c) * (1.0 - tx) + px(ix1, iy1, c) * tx;
                    out[o + c] = (top * (1.0 - ty) + bot * ty).round() as u8;
                }
            }
        }
        (
            Frame { width: ow as i32, height: oh as i32, bgra: out },
            CropMap { x0: x0 as f32, y0: y0 as f32, factor: factor as f32, pad: pad as f32 },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: i32, h: i32, v: u8) -> Frame {
        Frame { width: w, height: h, bgra: vec![v; (w * h * 4) as usize] }
    }

    #[test]
    fn resize_to_720p_keeps_aspect_and_never_upscales() {
        let r = solid(4000, 2000, 10).resized_to_height(720);
        assert_eq!((r.width, r.height), (1440, 720));
        let r = solid(2560, 1440, 10).resized_to_height(720);
        assert_eq!((r.width, r.height), (1280, 720));
        let small = solid(1000, 600, 10).resized_to_height(720);
        assert_eq!((small.width, small.height), (1000, 600));
    }

    #[test]
    fn flat_frames_are_blank_but_real_content_is_not() {
        assert!(solid(640, 360, 0).looks_blank());
        assert!(solid(640, 360, 120).looks_blank());
        let mut f = solid(640, 360, 0);
        for y in 0..360usize {
            for x in 0..320usize {
                let i = (y * 640 + x) * 4;
                f.bgra[i] = 200;
                f.bgra[i + 1] = 200;
                f.bgra[i + 2] = 200;
            }
        }
        assert!(!f.looks_blank());
    }
}
