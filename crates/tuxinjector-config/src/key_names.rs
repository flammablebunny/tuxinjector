// **THE FOLLOWING CODE WAS WRITTEN BY AN LLM, AGAIN**
//
// ═══════════════════════════════════════════════════════════════════════════
// Module: key_names — GLFW Keycode ↔ String Name Mapping
// ═══════════════════════════════════════════════════════════════════════════
//
// Provides bidirectional conversion between GLFW keycodes and their
// human-readable string representations, with serde integration for
// config deserialization/serialization.

use std::borrow::Cow;

// Keycodes at or above this value represent physical scancodes rather than
// GLFW virtual keycodes. The actual evdev scancode = keycode - SCANCODE_OFFSET.
// Offset is above Unicode max (U+10FFFF = 1,114,111) + CHAR_OFFSET (1000)
// so character keycodes and scancodes never collide.
pub const SCANCODE_OFFSET: u32 = 0x20_0000; // 2,097,152

// Single character keycodes are stored as CHAR_OFFSET + unicode codepoint.
// This supports the full Unicode range for chat/text rebinds.
pub const CHAR_OFFSET: u32 = 1000;

// Parse a "Ctrl+Shift+Z" style combo string → sorted Vec of GLFW keycodes.
pub fn parse_key_combo_str(combo: &str) -> Result<Vec<u32>, String> {
    let mut keys: Vec<u32> = Vec::new();

    for part in combo.split('+') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let code = parse_key_name(part)
            .ok_or_else(|| format!("unknown key name: '{part}'"))?;
        keys.push(code);
    }

    if keys.is_empty() {
        return Err("empty key combo".into());
    }

    keys.sort();
    keys.dedup();
    Ok(keys)
}

// Case-insensitive key name → GLFW keycode.
pub fn parse_key_name(name: &str) -> Option<u32> {
    let lower = name.to_lowercase();
    match lower.as_str() {
        // ── Alphabetic Keys (A–Z) ──
        "a" => Some(65),  "b" => Some(66),  "c" => Some(67),  "d" => Some(68),
        "e" => Some(69),  "f" => Some(70),  "g" => Some(71),  "h" => Some(72),
        "i" => Some(73),  "j" => Some(74),  "k" => Some(75),  "l" => Some(76),
        "m" => Some(77),  "n" => Some(78),  "o" => Some(79),  "p" => Some(80),
        "q" => Some(81),  "r" => Some(82),  "s" => Some(83),  "t" => Some(84),
        "u" => Some(85),  "v" => Some(86),  "w" => Some(87),  "x" => Some(88),
        "y" => Some(89),  "z" => Some(90),

        // ── Numeric Keys (0–9) + shifted symbols ──
        "0" | ")" => Some(48), "1" | "!" => Some(49), "2" | "@" => Some(50), "3" | "#" => Some(51),
        "4" | "$" => Some(52), "5" | "%" => Some(53), "6" | "^" => Some(54), "7" | "&" => Some(55),
        "8" | "*" => Some(56), "9" | "(" => Some(57),

        // ── Function Keys (F1–F24) ──
        "f1"  => Some(290), "f2"  => Some(291), "f3"  => Some(292), "f4"  => Some(293),
        "f5"  => Some(294), "f6"  => Some(295), "f7"  => Some(296), "f8"  => Some(297),
        "f9"  => Some(298), "f10" => Some(299), "f11" => Some(300), "f12" => Some(301),
        "f13" => Some(302), "f14" => Some(303), "f15" => Some(304), "f16" => Some(305),
        "f17" => Some(306), "f18" => Some(307), "f19" => Some(308), "f20" => Some(309),
        "f21" => Some(310), "f22" => Some(311), "f23" => Some(312), "f24" => Some(313),

        // ── Navigation & Miscellaneous ──
        "escape" | "esc"              => Some(256),
        "enter"  | "return"           => Some(257),
        "tab"                         => Some(258),
        "backspace"                   => Some(259),
        "insert"                      => Some(260),
        "delete" | "del"              => Some(261),
        "right"                       => Some(262),
        "left"                        => Some(263),
        "down"                        => Some(264),
        "up"                          => Some(265),
        "pageup"   | "page_up"        => Some(266),
        "pagedown" | "page_down"      => Some(267),
        "home"                        => Some(268),
        "end"                         => Some(269),
        "capslock" | "caps_lock"      => Some(280),
        "scrolllock" | "scroll_lock"  => Some(281),
        "numlock"  | "num_lock"       => Some(282),
        "printscreen" | "print_screen"=> Some(283),
        "pause"                       => Some(284),
        "space"                       => Some(32),

        // ── Modifier Keys ──
        "shift"  | "lshift" | "left_shift"    => Some(340),
        "ctrl"   | "control" | "lctrl" | "left_control" => Some(341),
        "alt"    | "lalt"   | "left_alt"      => Some(342),
        "super"  | "lsuper" | "left_super" | "meta" | "win" => Some(343),
        "rshift" | "right_shift"              => Some(344),
        "rctrl"  | "right_control"            => Some(345),
        "ralt"   | "right_alt"               => Some(346),
        "rsuper" | "right_super"             => Some(347),

        // ── Punctuation & Symbols ──
        "minus"  | "-" | "_"  => Some(45),
        "equal"  | "equals" | "=" | "+" => Some(61),
        "leftbracket"  | "left_bracket"  | "[" | "{" => Some(91),
        "rightbracket" | "right_bracket" | "]" | "}" => Some(93),
        "backslash" | "\\" | "|" => Some(92),
        "semicolon" | ";" | ":"  => Some(59),
        "apostrophe" | "'" | "\"" => Some(39),
        "grave" | "grave_accent" | "`" | "~" => Some(96),
        "comma"  | "," | "<"  => Some(44),
        "period" | "." | ">"  => Some(46),
        "slash"  | "/" | "?"  => Some(47),

        // ── Mouse Buttons ──
        "mouse1" | "mouse_left"   | "lmb" => Some(400),
        "mouse2" | "mouse_right"  | "rmb" => Some(401),
        "mouse3" | "mouse_middle" | "mmb" => Some(402),
        "mouse4" => Some(403),
        "mouse5" => Some(404),
        "mouse6" => Some(405),
        "mouse7" => Some(406),
        "mouse8" => Some(407),

        // ── Numpad ──
        "kp0" | "kp_0" | "numpad0" => Some(320),
        "kp1" | "kp_1" | "numpad1" => Some(321),
        "kp2" | "kp_2" | "numpad2" => Some(322),
        "kp3" | "kp_3" | "numpad3" => Some(323),
        "kp4" | "kp_4" | "numpad4" => Some(324),
        "kp5" | "kp_5" | "numpad5" => Some(325),
        "kp6" | "kp_6" | "numpad6" => Some(326),
        "kp7" | "kp_7" | "numpad7" => Some(327),
        "kp8" | "kp_8" | "numpad8" => Some(328),
        "kp9" | "kp_9" | "numpad9" => Some(329),
        "kp_decimal"  | "numpad_decimal"  => Some(330),
        "kp_divide"   | "numpad_divide"   => Some(331),
        "kp_multiply" | "numpad_multiply" => Some(332),
        "kp_subtract" | "numpad_subtract" => Some(333),
        "kp_add"      | "numpad_add"      => Some(334),
        "kp_enter"    | "numpad_enter"    => Some(335),
        "kp_equal"    | "numpad_equal"    => Some(336),

        _ => {
            // physical scancode: "scan:30" maps to SCANCODE_OFFSET + 30
            if let Some(sc) = lower.strip_prefix("scan:").or_else(|| lower.strip_prefix("sc:")) {
                if let Ok(code) = sc.parse::<u32>() {
                    return Some(SCANCODE_OFFSET + code);
                }
            }
            // single character -> CHAR_OFFSET + unicode codepoint
            // works for ASCII, Cyrillic, CJK, etc.
            let mut chars = name.chars();
            if let Some(ch) = chars.next() {
                if chars.next().is_none() && ch as u32 >= 0x20 {
                    return Some(CHAR_OFFSET + ch as u32);
                }
            }
            None
        }
    }
}

// ASCII 32..126 as single-char strings, indexed by (charcode − 32).
const CHAR_NAMES: [&str; 95] = [
    " ", "!", "\"", "#", "$", "%", "&", "'", "(", ")", "*", "+", ",", "-", ".", "/",
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", ":", ";", "<", "=", ">", "?",
    "@", "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O",
    "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z", "[", "\\", "]", "^", "_",
    "`", "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o",
    "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z", "{", "|", "}", "~",
];

// GLFW keycode → human-readable display name.
pub fn keycode_to_name(code: u32) -> Cow<'static, str> {
    // Scancode-based keycodes
    if code >= SCANCODE_OFFSET {
        return Cow::Owned(format!("scan:{}", code - SCANCODE_OFFSET));
    }
    // character keycodes: CHAR_OFFSET + unicode codepoint
    if code >= CHAR_OFFSET && code < SCANCODE_OFFSET {
        let cp = code - CHAR_OFFSET;
        // fast path for printable ASCII
        if cp >= 32 && cp <= 126 {
            return Cow::Borrowed(CHAR_NAMES[(cp - 32) as usize]);
        }
        // unicode path
        if let Some(ch) = char::from_u32(cp) {
            return Cow::Owned(ch.to_string());
        }
    }
    Cow::Borrowed(match code {
        65  => "A",  66  => "B",  67  => "C",  68  => "D",
        69  => "E",  70  => "F",  71  => "G",  72  => "H",
        73  => "I",  74  => "J",  75  => "K",  76  => "L",
        77  => "M",  78  => "N",  79  => "O",  80  => "P",
        81  => "Q",  82  => "R",  83  => "S",  84  => "T",
        85  => "U",  86  => "V",  87  => "W",  88  => "X",
        89  => "Y",  90  => "Z",

        48  => "0",  49  => "1",  50  => "2",  51  => "3",
        52  => "4",  53  => "5",  54  => "6",  55  => "7",
        56  => "8",  57  => "9",

        290 => "F1",  291 => "F2",  292 => "F3",  293 => "F4",
        294 => "F5",  295 => "F6",  296 => "F7",  297 => "F8",
        298 => "F9",  299 => "F10", 300 => "F11", 301 => "F12",
        302 => "F13", 303 => "F14", 304 => "F15", 305 => "F16",
        306 => "F17", 307 => "F18", 308 => "F19", 309 => "F20",
        310 => "F21", 311 => "F22", 312 => "F23", 313 => "F24",

        256 => "Escape",
        257 => "Enter",
        258 => "Tab",
        259 => "Backspace",
        260 => "Insert",
        261 => "Delete",
        262 => "Right",
        263 => "Left",
        264 => "Down",
        265 => "Up",
        266 => "PageUp",
        267 => "PageDown",
        268 => "Home",
        269 => "End",
        280 => "CapsLock",
        281 => "ScrollLock",
        282 => "NumLock",
        283 => "PrintScreen",
        284 => "Pause",
        32  => "Space",

        340 => "LShift",
        341 => "LCtrl",
        342 => "LAlt",
        343 => "LSuper",
        344 => "RShift",
        345 => "RCtrl",
        346 => "RAlt",
        347 => "RSuper",

        45  => "-",
        61  => "=",
        91  => "[",
        93  => "]",
        92  => "\\",
        59  => ";",
        39  => "'",
        96  => "`",
        44  => ",",
        46  => ".",
        47  => "/",

        320 => "KP0", 321 => "KP1", 322 => "KP2", 323 => "KP3",
        324 => "KP4", 325 => "KP5", 326 => "KP6", 327 => "KP7",
        328 => "KP8", 329 => "KP9",

        // ── Mouse ──
        400 => "Mouse1",
        401 => "Mouse2",
        402 => "Mouse3",
        403 => "Mouse4",
        404 => "Mouse5",
        405 => "Mouse6",
        406 => "Mouse7",
        407 => "Mouse8",

        330 => "KP.",
        331 => "KP/",
        332 => "KP*",
        333 => "KP-",
        334 => "KP+",
        335 => "KPEnter",
        336 => "KP=",

        _ => "unknown",
    })
}

// Format a slice of GLFW keycodes → "Ctrl+Shift+Z" style combo string.
pub fn keys_to_combo_string(keys: &[u32]) -> String {
    if keys.is_empty() {
        return String::new();
    }

    // Modifiers precede regular keys in output.
    let mut sorted: Vec<u32> = keys.to_vec();
    sorted.sort_by_key(|&k| {
        if (340..=347).contains(&k) {
            (0u32, k)
        } else {
            (1u32, k)
        }
    });

    sorted
        .iter()
        .map(|&k| keycode_to_name(k))
        .collect::<Vec<_>>()
        .join("+")
}

// ── Serde Integration ───────────────────────────────────────────────────
//
// Serializes keycodes as human-readable names in configuration files.
// Deserialization accepts either numeric codes or string names.

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum KeycodeOrName {
    Code(u32),
    Name(String),
}

impl KeycodeOrName {
    fn to_keycode<E: serde::de::Error>(self) -> Result<u32, E> {
        match self {
            Self::Code(c) => Ok(c),
            Self::Name(s) => parse_key_name(&s)
                .ok_or_else(|| E::custom(format!("unknown key name: '{s}'"))),
        }
    }
}

// Deserialize a vec of keycodes that can be numbers or string names.
pub fn deserialize_keycode_vec<'de, D>(deserializer: D) -> Result<Vec<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let items: Vec<KeycodeOrName> = serde::Deserialize::deserialize(deserializer)?;
    items
        .into_iter()
        .map(|item| item.to_keycode::<D::Error>())
        .collect()
}

// Deserialize a single keycode from number or string.
pub fn deserialize_keycode<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let item: KeycodeOrName = serde::Deserialize::deserialize(deserializer)?;
    item.to_keycode::<D::Error>()
}

// Serialize keycodes as their string names when possible.
pub fn serialize_keycode_vec<S>(keys: &Vec<u32>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(keys.len()))?;
    for &code in keys {
        let name = keycode_to_name(code);
        if *name == *"unknown" {
            seq.serialize_element(&code)?;
        } else {
            seq.serialize_element(&*name)?;
        }
    }
    seq.end()
}

// Serialize a single keycode as string name (falls back to numeric).
pub fn serialize_keycode<S>(code: &u32, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let name = keycode_to_name(*code);
    if *name == *"unknown" || *code == 0 {
        serializer.serialize_u32(*code)
    } else {
        serializer.serialize_str(&name)
    }
}
