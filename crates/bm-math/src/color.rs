//! BlueMap's mutable RGBA colour. Channels are `0..=1` floats, either straight or premultiplied by alpha; the
//! blend ops convert `self` to the representation they need first.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
    pub premultiplied: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid color format: '{0}'")]
pub struct ParseColorError(String);

impl Color {
    pub fn set(&mut self, r: f32, g: f32, b: f32, a: f32, premultiplied: bool) -> &mut Self {
        *self = Self { r, g, b, a, premultiplied };
        self
    }

    /// `set(int)`: ARGB, straight alpha.
    pub fn set_int(&mut self, color: i32) -> &mut Self {
        self.set_int_premultiplied(color, false)
    }

    pub fn set_int_premultiplied(&mut self, color: i32, premultiplied: bool) -> &mut Self {
        self.r = ((color >> 16) & 0xFF) as f32 / 255.0;
        self.g = ((color >> 8) & 0xFF) as f32 / 255.0;
        self.b = (color & 0xFF) as f32 / 255.0;
        self.a = ((color >> 24) & 0xFF) as f32 / 255.0;
        self.premultiplied = premultiplied;
        self
    }

    /// ARGB with each channel truncated (not rounded) from `channel * 255`, in whatever representation `self` is.
    pub fn get_int(&self) -> i32 {
        let r = (self.r * 255.0) as i32 & 0xFF;
        let g = (self.g * 255.0) as i32 & 0xFF;
        let b = (self.b * 255.0) as i32 & 0xFF;
        let a = (self.a * 255.0) as i32 & 0xFF;
        (a << 24) | (r << 16) | (g << 8) | b
    }

    pub fn add(&mut self, color: &Color) -> &mut Self {
        assert_premultiplied(color, "add");
        self.premultiplied();
        self.r += color.r;
        self.g += color.g;
        self.b += color.b;
        self.a += color.a;
        self
    }

    pub fn div(&mut self, divisor: i32) -> &mut Self {
        self.premultiplied();
        let p = 1.0 / divisor as f32;
        self.r *= p;
        self.g *= p;
        self.b *= p;
        self.a *= p;
        self
    }

    /// Channel-wise product, after converting `self` to `color`'s representation.
    pub fn multiply(&mut self, color: &Color) -> &mut Self {
        if color.premultiplied {
            self.premultiplied();
        } else {
            self.straight();
        }
        self.r *= color.r;
        self.g *= color.g;
        self.b *= color.b;
        self.a *= color.a;
        self
    }

    /// Draws `color` over `self`.
    pub fn overlay(&mut self, color: &Color) -> &mut Self {
        assert_premultiplied(color, "overlay");
        self.premultiplied();
        let p = 1.0 - color.a;
        self.a = p * self.a + color.a;
        self.r = p * self.r + color.r;
        self.g = p * self.g + color.g;
        self.b = p * self.b + color.b;
        self
    }

    /// Draws `color` under `self`.
    pub fn underlay(&mut self, color: &Color) -> &mut Self {
        assert_premultiplied(color, "underlay");
        self.premultiplied();
        let p = 1.0 - self.a;
        self.a += p * color.a;
        self.r += p * color.r;
        self.g += p * color.g;
        self.b += p * color.b;
        self
    }

    /// Forces alpha to 1, un-premultiplying first. Leaves the `premultiplied` flag as it was.
    pub fn flatten(&mut self) -> &mut Self {
        if self.a == 1.0 {
            return self;
        }
        if self.premultiplied && self.a > 0.0 {
            let m = 1.0 / self.a;
            self.r *= m;
            self.g *= m;
            self.b *= m;
        }
        self.a = 1.0;
        self
    }

    pub fn premultiplied(&mut self) -> &mut Self {
        if !self.premultiplied {
            self.r *= self.a;
            self.g *= self.a;
            self.b *= self.a;
            self.premultiplied = true;
        }
        self
    }

    pub fn straight(&mut self) -> &mut Self {
        if self.premultiplied {
            if self.a > 0.0 {
                let m = 1.0 / self.a;
                self.r *= m;
                self.g *= m;
                self.b *= m;
            }
            self.premultiplied = false;
        }
        self
    }

    /// `Color.parse`: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa` (CSS order, alpha last) or a decimal ARGB int whose
    /// alpha defaults to opaque when its top byte is zero. Always yields straight alpha.
    pub fn parse(&mut self, value: &str) -> Result<&mut Self, ParseColorError> {
        let err = || ParseColorError(value.to_owned());
        // Java indexes UTF-16 units and accepts Unicode digits; non-ASCII colours are rejected here instead
        if !value.is_ascii() {
            return Err(err());
        }
        if let Some(hex) = value.strip_prefix('#') {
            let b = hex.as_bytes();
            let full = match b.len() {
                3 => [b[0], b[0], b[1], b[1], b[2], b[2], b'f', b'f'],
                4 => [b[0], b[0], b[1], b[1], b[2], b[2], b[3], b[3]],
                6 => [b[0], b[1], b[2], b[3], b[4], b[5], b'f', b'f'],
                8 => [b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]],
                _ => return Err(err()),
            };
            let argb = [full[6], full[7], full[0], full[1], full[2], full[3], full[4], full[5]];
            let argb = std::str::from_utf8(&argb).map_err(|_| err())?;
            let color = u32::from_str_radix(argb, 16).map_err(|_| err())?;
            return Ok(self.set_int(color as i32));
        }
        let mut color: i32 = value.parse().map_err(|_| err())?;
        if color as u32 & 0xFF00_0000 == 0 {
            color |= 0xFF00_0000u32 as i32;
        }
        Ok(self.set_int(color))
    }
}

/// Java's `IllegalArgumentException` guard; written as `a < 1` so a NaN alpha passes, as it does in Java.
fn assert_premultiplied(color: &Color, op: &str) {
    if color.a < 1.0 && !color.premultiplied {
        panic!("can only {op} premultiplied colors with alpha");
    }
}
