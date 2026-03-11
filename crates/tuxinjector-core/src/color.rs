use serde::de::{self, SeqAccess, Visitor};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

// RGBA as normalized floats internally. Serde accepts both 0-255 ints
// and 0.0-1.0 floats because people kept getting confused about which format to use
// in their config files. The heuristic is simple: if any channel > 1.0, treat as bytes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    pub const BLACK: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const TRANSPARENT: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

    pub fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: a as f32 / 255.0,
        }
    }

    // Approximate sRGB -> linear via gamma 2.2. Not physically accurate
    // but good enough for blending operations in the overlay.
    pub fn to_linear(self) -> Self {
        Self {
            r: self.r.powf(2.2),
            g: self.g.powf(2.2),
            b: self.b.powf(2.2),
            a: self.a, // alpha is always linear
        }
    }

    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::BLACK
    }
}

// -- serde --
//
// Serializes as 0-255 ints (omits alpha when fully opaque to keep configs clean).
// Deserializes from either int or float arrays.

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        let r = (self.r * 255.0).round() as u8;
        let g = (self.g * 255.0).round() as u8;
        let b = (self.b * 255.0).round() as u8;
        let a = (self.a * 255.0).round() as u8;

        if a == 255 {
            // no alpha needed - saves clutter in config files
            let mut seq = ser.serialize_seq(Some(3))?;
            seq.serialize_element(&r)?;
            seq.serialize_element(&g)?;
            seq.serialize_element(&b)?;
            seq.end()
        } else {
            let mut seq = ser.serialize_seq(Some(4))?;
            seq.serialize_element(&r)?;
            seq.serialize_element(&g)?;
            seq.serialize_element(&b)?;
            seq.serialize_element(&a)?;
            seq.end()
        }
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_seq(ColorVisitor)
    }
}

struct ColorVisitor;

impl<'de> Visitor<'de> for ColorVisitor {
    type Value = Color;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("an array of 3 or 4 numbers (integer 0-255 or float 0.0-1.0)")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Color, A::Error> {
        // f64 so both ints and floats land in the same type
        let r: f64 = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(0, &self))?;
        let g: f64 = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(1, &self))?;
        let b: f64 = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(2, &self))?;
        let a: Option<f64> = seq.next_element()?;

        // if anything is > 1.0, assume the whole thing is 0-255 range
        let peak = r.max(g).max(b).max(a.unwrap_or(0.0));
        let is_byte = peak > 1.0;

        let conv = |v: f64| -> f32 {
            if is_byte { (v / 255.0) as f32 } else { v as f32 }
        };

        Ok(Color {
            r: conv(r),
            g: conv(g),
            b: conv(b),
            a: a.map(conv).unwrap_or(1.0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_black() {
        assert_eq!(Color::default(), Color::BLACK);
    }

    #[test]
    fn to_linear_identity_for_zero_and_one() {
        let black = Color::BLACK.to_linear();
        assert!((black.r).abs() < 1e-6);
        let white = Color::WHITE.to_linear();
        assert!((white.r - 1.0).abs() < 1e-6);
    }

    #[test]
    fn serde_roundtrip_opaque() {
        let c = Color::from_rgba8(255, 128, 0, 255);
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, "[255,128,0]"); // no alpha in output
        let back: Color = serde_json::from_str(&json).unwrap();
        assert!((back.r - c.r).abs() < 0.01);
        assert!((back.g - c.g).abs() < 0.01);
        assert!((back.b - c.b).abs() < 0.01);
    }

    #[test]
    fn serde_roundtrip_transparent() {
        let c = Color::from_rgba8(255, 128, 0, 128);
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, "[255,128,0,128]");
        let back: Color = serde_json::from_str(&json).unwrap();
        assert!((back.a - c.a).abs() < 0.01);
    }

    #[test]
    fn deserialize_float_array() {
        let c: Color = serde_json::from_str("[1.0, 0.5, 0.0]").unwrap();
        assert!((c.r - 1.0).abs() < 1e-6);
        assert!((c.g - 0.5).abs() < 1e-6);
        assert!((c.b - 0.0).abs() < 1e-6);
        assert!((c.a - 1.0).abs() < 1e-6);
    }
}
