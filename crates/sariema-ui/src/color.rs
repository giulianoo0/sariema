// SPDX-License-Identifier: Apache-2.0

use serde::Deserialize;

/// sRGB with straight alpha, each channel in [0, 1], as CSS colours are.
/// The draw pass blends premultiplied, in the target's own encoding.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(try_from = "String")]
pub struct Color(pub [f32; 4]);

impl Color {
    pub const WHITE: Color = Color([1.0, 1.0, 1.0, 1.0]);
    pub const BLACK: Color = Color([0.0, 0.0, 0.0, 1.0]);
    pub const TRANSPARENT: Color = Color([0.0; 4]);

    /// `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.
    pub fn parse(s: &str) -> Result<Color, String> {
        let bad = || format!("{s:?} is not a colour (#rgb, #rgba, #rrggbb or #rrggbbaa)");
        let hex = s.strip_prefix('#').ok_or_else(bad)?;
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(bad());
        }
        let digit = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).map_err(|_| bad());
        let pair = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| bad());
        let bytes: [u8; 4] = match hex.len() {
            3 | 4 => {
                let mut b = [255u8; 4];
                for (i, v) in b.iter_mut().enumerate().take(hex.len()) {
                    *v = digit(i)? * 17;
                }
                b
            }
            6 | 8 => {
                let mut b = [255u8; 4];
                for (i, v) in b.iter_mut().enumerate().take(hex.len() / 2) {
                    *v = pair(2 * i)?;
                }
                b
            }
            _ => return Err(bad()),
        };
        Ok(Color(bytes.map(|b| f32::from(b) / 255.0)))
    }

    pub fn with_alpha(self, a: f32) -> Color {
        let [r, g, b, _] = self.0;
        Color([r, g, b, a])
    }
}

impl TryFrom<String> for Color {
    type Error = String;

    fn try_from(s: String) -> Result<Color, String> {
        Color::parse(&s)
    }
}

#[cfg(test)]
mod tests {
    use super::Color;

    #[test]
    fn parses_css_hex_forms() {
        assert_eq!(Color::parse("#fff").unwrap(), Color::WHITE);
        assert_eq!(Color::parse("#000000").unwrap(), Color::BLACK);
        let c = Color::parse("#33261C61").unwrap().0;
        assert!((c[0] - 0x33 as f32 / 255.0).abs() < 1e-6);
        assert!((c[3] - 0x61 as f32 / 255.0).abs() < 1e-6);
        assert_eq!(Color::parse("#0008").unwrap().0[3], 0x88 as f32 / 255.0);
        for bad in ["fff", "#ff", "#ggg", "#12345"] {
            assert!(Color::parse(bad).is_err(), "{bad}");
        }
    }
}
