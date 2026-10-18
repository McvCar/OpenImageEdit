//! Picked-color readout for the 拾色器 (color picker) panel.
//!
//! A picked pixel is published to QML as one JSON object carrying the same
//! color in every notation an artist needs at hand — HEX, `rgb()`/`rgba()`,
//! `hsl()` and a naive `cmyk()` — plus the canvas coordinate it came from.
//! Each string is finished here (not in QML) so the formatting is unit-tested
//! and the panel only has to render + copy it.
//!
//! The conversions are deliberately simple and document-free:
//!
//! * HEX / RGB are exact byte values.
//! * HSL uses the standard sRGB cylinder (hue in degrees, saturation and
//!   lightness in whole percent).
//! * CMYK is the naive `k = 1 - max(r,g,b)` conversion (no ICC profile), which
//!   is what every picker without a print profile shows.

/// One sampled pixel of the composition plus its canvas coordinate.
///
/// `x`/`y` are image-pixel coordinates (top-left origin, `0..width`,
/// `0..height`); the Y-flip used by the ruler is applied by the UI, never
/// here — the picked color must not depend on the display preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickedColor {
    pub x: i32,
    pub y: i32,
    pub rgba: [u8; 4],
}

impl PickedColor {
    pub fn new(x: i32, y: i32, rgba: [u8; 4]) -> Self {
        Self { x, y, rgba }
    }

    /// `#rrggbb` (lowercase, alpha dropped — see [`Self::rgb`] for alpha).
    pub fn hex(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.rgba[0], self.rgba[1], self.rgba[2])
    }

    /// `rgb(r, g, b)`, or `rgba(r, g, b, 0.42)` when the pixel is translucent
    /// (a partly transparent canvas / layer must not silently read as opaque).
    pub fn rgb(&self) -> String {
        let [r, g, b, a] = self.rgba;
        if a == 255 {
            format!("rgb({r}, {g}, {b})")
        } else {
            format!("rgba({r}, {g}, {b}, {:.2})", a as f64 / 255.0)
        }
    }

    /// `hsl(210, 63%, 40%)` — hue in integer degrees, S/L in whole percent.
    pub fn hsl(&self) -> String {
        let (h, s, l) = hsl_of(self.rgba[0], self.rgba[1], self.rgba[2]);
        format!("hsl({}, {}%, {}%)", h.round() as i32, (s * 100.0).round() as i32, (l * 100.0).round() as i32)
    }

    /// `cmyk(70%, 30%, 0%, 25%)` — naive conversion (see module docs).
    pub fn cmyk(&self) -> String {
        let (c, m, y, k) = cmyk_of(self.rgba[0], self.rgba[1], self.rgba[2]);
        format!(
            "cmyk({}%, {}%, {}%, {}%)",
            (c * 100.0).round() as i32,
            (m * 100.0).round() as i32,
            (y * 100.0).round() as i32,
            (k * 100.0).round() as i32
        )
    }

    /// One JSON object for QML: coordinates, raw bytes and every notation.
    pub fn to_json(&self) -> String {
        let [r, g, b, a] = self.rgba;
        format!(
            "{{\"x\":{},\"y\":{},\"r\":{r},\"g\":{g},\"b\":{b},\"a\":{a},\"hex\":\"{}\",\"rgb\":\"{}\",\"hsl\":\"{}\",\"cmyk\":\"{}\"}}",
            self.x,
            self.y,
            self.hex(),
            self.rgb(),
            self.hsl(),
            self.cmyk()
        )
    }
}

/// sRGB → HSL. Returns `(hue degrees 0..360, saturation 0..1, lightness 0..1)`.
pub fn hsl_of(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let (r, g, b) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let delta = max - min;
    if delta.abs() < f64::EPSILON {
        // Grey: hue is undefined (report 0), saturation 0.
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 {
        delta / (2.0 - max - min)
    } else {
        delta / (max + min)
    };
    let h = if (max - r).abs() < f64::EPSILON {
        ((g - b) / delta).rem_euclid(6.0)
    } else if (max - g).abs() < f64::EPSILON {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    } * 60.0;
    (h, s, l)
}

/// sRGB → naive CMYK. Each component runs `0..1`; `k = 1 - max(r, g, b)`.
pub fn cmyk_of(r: u8, g: u8, b: u8) -> (f64, f64, f64, f64) {
    let (r, g, b) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let k = 1.0 - r.max(g).max(b);
    if k >= 1.0 {
        // Pure black: only the K plate carries ink.
        return (0.0, 0.0, 0.0, 1.0);
    }
    let denom = 1.0 - k;
    ((1.0 - r - k) / denom, (1.0 - g - k) / denom, (1.0 - b - k) / denom, k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_lowercase_and_drops_alpha() {
        let c = PickedColor::new(0, 0, [255, 0, 171, 128]);
        assert_eq!(c.hex(), "#ff00ab");
    }

    #[test]
    fn opaque_pixel_uses_rgb_translucent_uses_rgba() {
        assert_eq!(PickedColor::new(0, 0, [12, 34, 56, 255]).rgb(), "rgb(12, 34, 56)");
        assert_eq!(
            PickedColor::new(0, 0, [255, 0, 0, 128]).rgb(),
            "rgba(255, 0, 0, 0.50)"
        );
        assert_eq!(
            PickedColor::new(0, 0, [255, 0, 0, 0]).rgb(),
            "rgba(255, 0, 0, 0.00)"
        );
    }

    #[test]
    fn hsl_matches_known_values() {
        assert_eq!(PickedColor::new(0, 0, [255, 255, 255, 255]).hsl(), "hsl(0, 0%, 100%)");
        assert_eq!(PickedColor::new(0, 0, [0, 0, 0, 255]).hsl(), "hsl(0, 0%, 0%)");
        assert_eq!(PickedColor::new(0, 0, [255, 0, 0, 255]).hsl(), "hsl(0, 100%, 50%)");
        assert_eq!(PickedColor::new(0, 0, [0, 255, 0, 255]).hsl(), "hsl(120, 100%, 50%)");
        assert_eq!(PickedColor::new(0, 0, [0, 0, 255, 255]).hsl(), "hsl(240, 100%, 50%)");
        // (0, 128, 255): hue 210°, fully saturated, mid lightness.
        assert_eq!(PickedColor::new(0, 0, [0, 128, 255, 255]).hsl(), "hsl(210, 100%, 50%)");
    }

    #[test]
    fn hsl_hue_is_never_negative() {
        // Magenta-ish: the `(g - b)` branch can go negative, rem_euclid keeps
        // the hue in 0..360 instead of reporting -30.
        let (h, _, _) = hsl_of(255, 0, 128);
        assert!((0.0..360.0).contains(&h), "hue {h} out of range");
    }

    #[test]
    fn cmyk_matches_known_values() {
        assert_eq!(PickedColor::new(0, 0, [255, 255, 255, 255]).cmyk(), "cmyk(0%, 0%, 0%, 0%)");
        assert_eq!(PickedColor::new(0, 0, [0, 0, 0, 255]).cmyk(), "cmyk(0%, 0%, 0%, 100%)");
        assert_eq!(PickedColor::new(0, 0, [255, 0, 0, 255]).cmyk(), "cmyk(0%, 100%, 100%, 0%)");
        assert_eq!(
            PickedColor::new(0, 0, [0, 128, 255, 255]).cmyk(),
            "cmyk(100%, 50%, 0%, 0%)"
        );
        // 25% black ink stays a pure K value.
        assert_eq!(PickedColor::new(0, 0, [191, 191, 191, 255]).cmyk(), "cmyk(0%, 0%, 0%, 25%)");
    }

    #[test]
    fn json_carries_every_notation_and_the_coordinate() {
        let json = PickedColor::new(7, 9, [0, 128, 255, 255]).to_json();
        for needle in [
            "\"x\":7",
            "\"y\":9",
            "\"r\":0",
            "\"g\":128",
            "\"b\":255",
            "\"a\":255",
            "\"hex\":\"#0080ff\"",
            "\"rgb\":\"rgb(0, 128, 255)\"",
            "\"hsl\":\"hsl(210, 100%, 50%)\"",
            "\"cmyk\":\"cmyk(100%, 50%, 0%, 0%)\"",
        ] {
            assert!(json.contains(needle), "missing {needle} in {json}");
        }
    }
}
