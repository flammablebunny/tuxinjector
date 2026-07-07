// Formatting + color helpers for the Ninjabrain Bot overlay. Ported straight
// from toolscreen: NBGradientColor + cell formatting live in render.cpp, the
// markup stuff came from ninjabrain_information_messages.cpp.

use tuxinjector_core::color::Color;

use crate::nbb_data::NbbInfoMessage;

/// Two-segment lerp: low -> mid on the first half, mid -> high on the second.
/// Result is always opaque.
pub fn gradient_color(probability: f64, low: &Color, mid: &Color, high: &Color) -> [f32; 4] {
    let p = probability.clamp(0.0, 1.0) as f32;
    // Split at the midpoint and rescale t back into [0, 1] for the chosen leg.
    let (start, end, t) = if p < 0.5 {
        (low, mid, p * 2.0)
    } else {
        (mid, high, (p - 0.5) * 2.0)
    };
    [
        start.r + (end.r - start.r) * t,
        start.g + (end.g - start.g) * t,
        start.b + (end.b - start.b) * t,
        1.0,
    ]
}

/// Coordinate pair "(x, z)" colored the way toolscreen does it. If both
/// components share a sign the whole string gets one color; if they disagree
/// we split after "(x, " so each half can be colored on its own. Return is
/// (text, color, split_prefix, part1_color) - a non-empty split_prefix means
/// part1 is that prefix.
pub fn format_coords(
    x: i64,
    z: i64,
    positive: [f32; 4],
    negative: [f32; 4],
) -> (String, [f32; 4], String, [f32; 4]) {
    let text = format!("({x}, {z})");
    let x_neg = x < 0;
    let z_neg = z < 0;
    if x_neg == z_neg {
        // Signs agree - one color for the whole thing.
        let c = if x_neg { negative } else { positive };
        (text, c, String::new(), c)
    } else {
        let prefix = format!("({x}, ");
        let c1 = if x_neg { negative } else { positive };
        let c2 = if z_neg { negative } else { positive };
        (text, c2, prefix, c1)
    }
}

/// Angle cell. With direction it's "-42.51 (-> 3.2)", otherwise just the angle.
/// The second return value is the bare angle - the caller feeds that into the
/// gradient, and the suffix starts right after it.
pub fn format_angle_with_direction(angle: f64, needed_correction: f64) -> (String, String) {
    let base = format!("{angle:.2}");
    let arrow = if needed_correction > 0.0 { "-> " } else { "<- " };
    let text = format!("{base} ({arrow}{:.1})", needed_correction.abs());
    (text, base)
}

/// Subpixel row: "%.2f%+d", sign always forced on the increment. A zero
/// increment drops the suffix entirely.
pub fn format_subpixel(angle: f64, increments: i32) -> String {
    if increments == 0 {
        format!("{angle:.2}")
    } else {
        format!("{angle:.2}{increments:+}")
    }
}

// info-message markup handling (ninjabrain_information_messages.cpp)

#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub color: Option<[u8; 3]>,
}

const COMBINED_CERTAINTY_DEFAULT_COLOR: &str = "#00CE29";

fn parse_hex_color(s: &str) -> Option<[u8; 3]> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn normalize_hex_color(color: &str) -> String {
    let c = if color.starts_with('#') {
        color.to_string()
    } else {
        format!("#{color}")
    };
    if c.len() == 7 && c[1..].bytes().all(|b| b.is_ascii_hexdigit()) {
        c.to_uppercase()
    } else {
        COMBINED_CERTAINTY_DEFAULT_COLOR.to_string()
    }
}

fn extract_leading_integers(text: &str, count: usize) -> Option<Vec<i64>> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() && out.len() < count {
        let c = bytes[i] as char;
        let signed_start = (c == '+' || c == '-')
            && i + 1 < bytes.len()
            && (bytes[i + 1] as char).is_ascii_digit();
        if signed_start || c.is_ascii_digit() {
            let start = i;
            let mut j = if signed_start { i + 1 } else { i };
            while j < bytes.len() && (bytes[j] as char).is_ascii_digit() {
                j += 1;
            }
            if let Ok(v) = text[start..j].parse::<i64>() {
                out.push(v);
                i = j;
                continue;
            }
        }
        i += 1;
    }
    (out.len() >= count).then_some(out)
}

fn extract_first_color_span(text: &str) -> Option<(String, String)> {
    let span_start = text.find("<span")?;
    let tag_end = span_start + text[span_start..].find('>')?;
    let close = tag_end + 1 + text[tag_end + 1..].find("</span>")?;
    let open_tag = &text[span_start..=tag_end];
    let hash = open_tag.find('#')?;
    let candidate = open_tag.get(hash..hash + 7)?;
    parse_hex_color(candidate)?;
    Some((
        normalize_hex_color(candidate),
        text[tag_end + 1..close].to_string(),
    ))
}

/// Canned strings for known message types (toolscreen lang/en.json).
pub fn translate_markup(msg: &NbbInfoMessage) -> String {
    match msg.msg_type.as_str() {
        "MISMEASURE" => "Detected unusually large errors, you probably mismeasured or your standard deviation is too low.".to_string(),
        "PORTAL_LINKING" => "You might not be able to nether travel into the stronghold due to portal linking.".to_string(),
        "MC_VERSION" => "Detected wrong Minecraft version, make sure the correct version is chosen in the settings.".to_string(),
        "NEXT_THROW_DIRECTION" => {
            if let Some(v) = extract_leading_integers(&msg.message, 2) {
                format!("Go left {} blocks, or right {} blocks, for ~95% certainty after next measurement.", v[0], v[1])
            } else {
                msg.message.clone()
            }
        }
        "COMBINED_CERTAINTY" => {
            if let (Some(v), Some((color, inner))) = (
                extract_leading_integers(&msg.message, 2),
                extract_first_color_span(&msg.message),
            ) {
                format!(
                    "Nether coords ({}, {}) have <span style=\"color:{color};\">{inner}</span> chance to hit the stronghold (it is between the top 2 offsets).",
                    v[0], v[1]
                )
            } else {
                msg.message.clone()
            }
        }
        _ => msg.message.clone(),
    }
}

/// Parse `<span style="color:#RRGGBB;">…</span>` markup into colored runs.
pub fn parse_markup_to_runs(markup: &str) -> Vec<TextRun> {
    let mut runs: Vec<TextRun> = Vec::new();
    let mut push = |text: &str, color: Option<[u8; 3]>| {
        if text.is_empty() {
            return;
        }
        if let Some(last) = runs.last_mut() {
            if last.color == color {
                last.text.push_str(text);
                return;
            }
        }
        runs.push(TextRun { text: text.to_string(), color });
    };

    let mut cursor = 0;
    while cursor < markup.len() {
        let Some(rel) = markup[cursor..].find("<span") else {
            push(&markup[cursor..], None);
            break;
        };
        let span_start = cursor + rel;
        if span_start > cursor {
            push(&markup[cursor..span_start], None);
        }
        let Some(rel_end) = markup[span_start..].find('>') else {
            push(&markup[span_start..], None);
            break;
        };
        let tag_end = span_start + rel_end;
        let Some(rel_close) = markup[tag_end + 1..].find("</span>") else {
            push(&markup[span_start..], None);
            break;
        };
        let close = tag_end + 1 + rel_close;

        let open_tag = &markup[span_start..=tag_end];
        let color = open_tag
            .find('#')
            .and_then(|h| open_tag.get(h..h + 7))
            .and_then(parse_hex_color);

        push(&markup[tag_end + 1..close], color);
        cursor = close + "</span>".len();
    }
    runs
}

pub fn format_info_message(msg: &NbbInfoMessage) -> Vec<TextRun> {
    parse_markup_to_runs(&translate_markup(msg))
}

/// Blind evaluation -> (gradient bucket, display words).
pub fn blind_evaluation(eval: &str) -> (f64, &'static str) {
    match eval {
        "EXCELLENT" => (1.0, "excellent"),
        "HIGHROLL_GOOD" => (1.0, "good for highroll"),
        "HIGHROLL_OKAY" => (0.75, "okay for highroll"),
        "BAD_BUT_IN_RING" => (0.5, "bad, but in ring"),
        "BAD" => (0.25, "bad"),
        "NOT_IN_RING" => (0.0, "not in any ring"),
        _ => (0.0, "unknown"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(r: f32, g: f32, b: f32) -> Color {
        Color { r, g, b, a: 1.0 }
    }

    #[test]
    fn gradient_endpoints_and_midpoints() {
        let low = c(1.0, 0.0, 0.0);
        let mid = c(1.0, 1.0, 0.0);
        let high = c(0.0, 0.8078, 0.1608);
        assert_eq!(gradient_color(0.0, &low, &mid, &high), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(gradient_color(0.5, &low, &mid, &high), [1.0, 1.0, 0.0, 1.0]);
        let g1 = gradient_color(1.0, &low, &mid, &high);
        assert!((g1[0] - 0.0).abs() < 1e-6 && (g1[1] - 0.8078).abs() < 1e-6);
        let q = gradient_color(0.25, &low, &mid, &high);
        assert!((q[1] - 0.5).abs() < 1e-6, "quarter point lerps green channel");
        // out of range clamps
        assert_eq!(gradient_color(-1.0, &low, &mid, &high), [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn coords_sign_split() {
        let pos = [1.0, 1.0, 1.0, 1.0];
        let neg = [1.0, 0.45, 0.45, 1.0];
        let (t, col, split, _) = format_coords(5, 7, pos, neg);
        assert_eq!(t, "(5, 7)");
        assert_eq!(col, pos);
        assert!(split.is_empty());
        let (_, col, split, _) = format_coords(-5, -7, pos, neg);
        assert_eq!(col, neg);
        assert!(split.is_empty());
        let (t, col, split, p1) = format_coords(-5, 7, pos, neg);
        assert_eq!(t, "(-5, 7)");
        assert_eq!(split, "(-5, ");
        assert_eq!(p1, neg, "x part negative");
        assert_eq!(col, pos, "z part positive");
        let (_, col, split, p1) = format_coords(5, -7, pos, neg);
        assert_eq!(split, "(5, ");
        assert_eq!(p1, pos);
        assert_eq!(col, neg);
    }

    #[test]
    fn angle_and_subpixel_formats() {
        let (t, base) = format_angle_with_direction(-42.514, 3.21);
        assert_eq!(t, "-42.51 (-> 3.2)");
        assert_eq!(base, "-42.51");
        let (t, _) = format_angle_with_direction(10.0, -0.5);
        assert_eq!(t, "10.00 (<- 0.5)");
        assert_eq!(format_subpixel(-42.5, 3), "-42.50+3");
        assert_eq!(format_subpixel(-42.5, -2), "-42.50-2");
        assert_eq!(format_subpixel(-42.5, 0), "-42.50");
    }

    #[test]
    fn markup_runs() {
        let runs = parse_markup_to_runs("hello <span style=\"color:#FF0000;\">red</span> world");
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0], TextRun { text: "hello ".into(), color: None });
        assert_eq!(runs[1], TextRun { text: "red".into(), color: Some([255, 0, 0]) });
        assert_eq!(runs[2], TextRun { text: " world".into(), color: None });
        // unclosed span falls through as plain text
        let runs = parse_markup_to_runs("a <span style=\"color:#00FF00;\">b");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "a <span style=\"color:#00FF00;\">b");
        // adjacent same-color runs merge
        let runs = parse_markup_to_runs("ab<span>cd</span>");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "abcd");
    }

    #[test]
    fn translate_known_types() {
        let m = |t: &str, msg: &str| NbbInfoMessage {
            severity: "INFO".into(),
            msg_type: t.into(),
            message: msg.into(),
        };
        assert!(translate_markup(&m("MISMEASURE", "x")).starts_with("Detected unusually large"));
        let t = translate_markup(&m("NEXT_THROW_DIRECTION", "12 and -34 blah"));
        assert_eq!(t, "Go left 12 blocks, or right -34 blocks, for ~95% certainty after next measurement.");
        let t = translate_markup(&m(
            "COMBINED_CERTAINTY",
            "120 -340 <span style=\"color:#00ce29;\">88%</span>",
        ));
        assert!(t.contains("Nether coords (120, -340)"));
        assert!(t.contains("<span style=\"color:#00CE29;\">88%</span>"));
        // unknown passes through
        assert_eq!(translate_markup(&m("OTHER", "raw")), "raw");
    }

    #[test]
    fn blind_buckets() {
        assert_eq!(blind_evaluation("EXCELLENT"), (1.0, "excellent"));
        assert_eq!(blind_evaluation("NOT_IN_RING").1, "not in any ring");
        assert_eq!(blind_evaluation("???").1, "unknown");
    }
}
