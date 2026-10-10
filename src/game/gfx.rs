//! The 84x48 monochrome LCD framebuffer and 1-bit bitmaps.

use super::data::Bmp;

pub const W: i32 = 84;
pub const H: i32 = 48;

/// A parsed 1-bit bitmap.
#[derive(Clone)]
pub struct Sprite {
    pub w: i32,
    pub h: i32,
    px: Vec<u8>,
}

impl Sprite {
    pub fn parse(b: &Bmp) -> Sprite {
        let (w, h) = (b.w as i32, b.h as i32);
        let mut px = vec![0u8; (w * h) as usize];
        for (y, row) in b.rows.iter().enumerate() {
            for (x, c) in row.bytes().enumerate() {
                px[y * w as usize + x] = (c == b'#') as u8;
            }
        }
        Sprite { w, h, px }
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.w && y < self.h && self.px[(y * self.w + x) as usize] != 0
    }
}

pub struct Frame {
    pub px: [u8; (W * H) as usize],
}

impl Default for Frame {
    fn default() -> Self {
        Frame {
            px: [0; (W * H) as usize],
        }
    }
}

impl Frame {
    pub fn clear(&mut self) {
        self.px.fill(0);
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < W && y < H && self.px[(y * W + x) as usize] != 0
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, on: bool) {
        if x >= 0 && y >= 0 && x < W && y < H {
            self.px[(y * W + x) as usize] = on as u8;
        }
    }

    /// OR a sprite onto the frame, clipped to rows `y0..y1`.
    pub fn blit_clip(&mut self, s: &Sprite, x: i32, y: i32, y0: i32, y1: i32) {
        for sy in 0..s.h {
            let py = y + sy;
            if py < y0 || py >= y1 {
                continue;
            }
            for sx in 0..s.w {
                if s.get(sx, sy) {
                    self.set(x + sx, py, true);
                }
            }
        }
    }

    pub fn blit(&mut self, s: &Sprite, x: i32, y: i32) {
        self.blit_clip(s, x, y, 0, H);
    }

    /// Copy a sprite, both its on and off pixels.
    pub fn paste(&mut self, s: &Sprite, x: i32, y: i32) {
        for sy in 0..s.h {
            for sx in 0..s.w {
                self.set(x + sx, y + sy, s.get(sx, sy));
            }
        }
    }

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, on: bool) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.set(xx, yy, on);
            }
        }
    }

    pub fn invert(&mut self) {
        for p in self.px.iter_mut() {
            *p ^= 1;
        }
    }
}

impl Sprite {
    /// Parse rows of `#`/`.` of any width (scenery strips exceed `Bmp`'s u8 width).
    pub fn from_rows(rows: &[&str]) -> Sprite {
        let w = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let mut px = vec![0u8; w * rows.len()];
        for (y, row) in rows.iter().enumerate() {
            for (x, c) in row.bytes().enumerate() {
                px[y * w + x] = (c == b'#') as u8;
            }
        }
        Sprite {
            w: w as i32,
            h: rows.len() as i32,
            px,
        }
    }
}
