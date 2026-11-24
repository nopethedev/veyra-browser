//! CSS colour parsing and blending.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Default for Color {
    fn default() -> Self {
        Color::BLACK
    }
}

impl Color {
    pub const TRANSPARENT: Color = Color { r: 0, g: 0, b: 0, a: 0 };
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0, a: 255 };
    pub const WHITE: Color = Color { r: 255, g: 255, b: 255, a: 255 };

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b, a: 255 }
    }
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color { r, g, b, a }
    }
    pub fn from_f32(r: f32, g: f32, b: f32, a: f32) -> Self {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Color { r: q(r), g: q(g), b: q(b), a: q(a) }
    }
    pub fn to_f32(self) -> [f32; 4] {
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            self.a as f32 / 255.0,
        ]
    }
    pub fn with_alpha(self, a: u8) -> Self {
        Color { a, ..self }
    }
    pub fn opaque(self) -> bool {
        self.a == 255
    }
    pub fn is_transparent(self) -> bool {
        self.a == 0
    }

    /// Source-over compositing of `self` on top of `dst`, both as straight
    /// (non-premultiplied) alpha.
    pub fn over(self, dst: Color) -> Color {
        if self.a == 0 {
            return dst;
        }
        if self.a == 255 || dst.a == 0 {
            return self;
        }
        let sa = self.a as f32 / 255.0;
        let da = dst.a as f32 / 255.0;
        let oa = sa + da * (1.0 - sa);
        if oa <= 0.0 {
            return Color::TRANSPARENT;
        }
        let mix = |s: u8, d: u8| {
            (((s as f32 * sa) + (d as f32 * da * (1.0 - sa))) / oa).round().clamp(0.0, 255.0) as u8
        };
        Color {
            r: mix(self.r, dst.r),
            g: mix(self.g, dst.g),
            b: mix(self.b, dst.b),
            a: (oa * 255.0).round() as u8,
        }
    }

    pub fn luminance(self) -> f32 {
        let f = |c: u8| {
            let c = c as f32 / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(self.r) + 0.7152 * f(self.g) + 0.0722 * f(self.b)
    }

    /// Perceptual contrast ratio per WCAG 2.1 (1.0 .. 21.0).
    pub fn contrast(self, other: Color) -> f32 {
        let a = self.luminance();
        let b = other.luminance();
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }

    pub fn mix(self, other: Color, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        let f = |a: u8, b: u8| ((a as f32) * (1.0 - t) + (b as f32) * t).round() as u8;
        Color { r: f(self.r, other.r), g: f(self.g, other.g), b: f(self.b, other.b), a: f(self.a, other.a) }
    }

    pub fn lighten(self, t: f32) -> Color {
        self.mix(Color::WHITE, t)
    }
    pub fn darken(self, t: f32) -> Color {
        self.mix(Color::BLACK, t)
    }

    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }

    /// Parses a CSS colour. Returns `None` for `currentcolor` and invalid
    /// values; callers resolve `currentcolor` against the inherited colour.
    pub fn parse(input: &str) -> Option<Color> {
        let s = input.trim().to_ascii_lowercase();
        if s.is_empty() {
            return None;
        }
        if let Some(hex) = s.strip_prefix('#') {
            return parse_hex(hex);
        }
        if let Some((name, args)) = split_function(&s) {
            return parse_function(name, args);
        }
        named_color(&s)
    }
}

fn split_function(s: &str) -> Option<(&str, &str)> {
    let open = s.find('(')?;
    if !s.ends_with(')') {
        return None;
    }
    Some((s[..open].trim(), &s[open + 1..s.len() - 1]))
}

fn parse_hex(hex: &str) -> Option<Color> {
    let digits: Vec<u8> = hex
        .bytes()
        .map(|b| (b as char).to_digit(16).map(|d| d as u8))
        .collect::<Option<Vec<u8>>>()?;
    let d = &digits;
    Some(match d.len() {
        3 => Color::rgb(d[0] * 17, d[1] * 17, d[2] * 17),
        4 => Color::rgba(d[0] * 17, d[1] * 17, d[2] * 17, d[3] * 17),
        6 => Color::rgb(d[0] * 16 + d[1], d[2] * 16 + d[3], d[4] * 16 + d[5]),
        8 => Color::rgba(d[0] * 16 + d[1], d[2] * 16 + d[3], d[4] * 16 + d[5], d[6] * 16 + d[7]),
        _ => return None,
    })
}

fn parse_function(name: &str, args: &str) -> Option<Color> {
    // Accept both the comma syntax and the modern space/slash syntax.
    let normalized = args.replace(',', " ");
    let mut parts: Vec<&str> = normalized.split([' ', '/']).filter(|p| !p.is_empty()).collect();
    let alpha = if let Some(pos) = normalized.find('/') {
        let a = parse_alpha(normalized[pos + 1..].trim())?;
        // Remove alpha token if it was space separated after "/".
        if parts.len() > 3 {
            parts.truncate(3);
        }
        a
    } else if parts.len() >= 4 {
        parse_alpha(parts.pop().unwrap())?
    } else {
        1.0
    };
    match name {
        "rgb" | "rgba" => {
            if parts.len() != 3 {
                return None;
            }
            let c = |t: &str| -> Option<u8> {
                if let Some(p) = t.strip_suffix('%') {
                    let v: f32 = p.trim().parse().ok()?;
                    Some((v.clamp(0.0, 100.0) / 100.0 * 255.0).round() as u8)
                } else {
                    let v: f32 = t.trim().parse().ok()?;
                    Some(v.clamp(0.0, 255.0).round() as u8)
                }
            };
            Some(Color::rgba(
                c(parts[0])?,
                c(parts[1])?,
                c(parts[2])?,
                (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
            ))
        }
        "hsl" | "hsla" => {
            if parts.len() != 3 {
                return None;
            }
            let h = parts[0].trim().trim_end_matches("deg").trim().parse::<f32>().ok()?;
            let sat = parts[1].trim().trim_end_matches('%').trim().parse::<f32>().ok()? / 100.0;
            let light = parts[2].trim().trim_end_matches('%').trim().parse::<f32>().ok()? / 100.0;
            let (r, g, b) = hsl_to_rgb(h, sat.clamp(0.0, 1.0), light.clamp(0.0, 1.0));
            Some(Color::from_f32(r, g, b, alpha))
        }
        _ => None,
    }
}

fn parse_alpha(t: &str) -> Option<f32> {
    if let Some(p) = t.trim().strip_suffix('%') {
        p.trim().parse::<f32>().ok().map(|v| v / 100.0)
    } else {
        t.trim().parse::<f32>().ok()
    }
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let h = (h.rem_euclid(360.0)) / 360.0;
    if s == 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let hue = |mut t: f32| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0))
}

fn named_color(s: &str) -> Option<Color> {
    let c = match s {
        "transparent" => Color::TRANSPARENT,
        "black" => Color::rgb(0, 0, 0),
        "white" => Color::rgb(255, 255, 255),
        "silver" => Color::rgb(192, 192, 192),
        "gray" | "grey" => Color::rgb(128, 128, 128),
        "maroon" => Color::rgb(128, 0, 0),
        "red" => Color::rgb(255, 0, 0),
        "purple" => Color::rgb(128, 0, 128),
        "fuchsia" | "magenta" => Color::rgb(255, 0, 255),
        "green" => Color::rgb(0, 128, 0),
        "lime" => Color::rgb(0, 255, 0),
        "olive" => Color::rgb(128, 128, 0),
        "yellow" => Color::rgb(255, 255, 0),
        "navy" => Color::rgb(0, 0, 128),
        "blue" => Color::rgb(0, 0, 255),
        "teal" => Color::rgb(0, 128, 128),
        "aqua" | "cyan" => Color::rgb(0, 255, 255),
        "orange" => Color::rgb(255, 165, 0),
        "aliceblue" => Color::rgb(240, 248, 255),
        "antiquewhite" => Color::rgb(250, 235, 215),
        "aquamarine" => Color::rgb(127, 255, 212),
        "azure" => Color::rgb(240, 255, 255),
        "beige" => Color::rgb(245, 245, 220),
        "bisque" => Color::rgb(255, 228, 196),
        "blanchedalmond" => Color::rgb(255, 235, 205),
        "blueviolet" => Color::rgb(138, 43, 226),
        "brown" => Color::rgb(165, 42, 42),
        "burlywood" => Color::rgb(222, 184, 135),
        "cadetblue" => Color::rgb(95, 158, 160),
        "chartreuse" => Color::rgb(127, 255, 0),
        "chocolate" => Color::rgb(210, 105, 30),
        "coral" => Color::rgb(255, 127, 80),
        "cornflowerblue" => Color::rgb(100, 149, 237),
        "cornsilk" => Color::rgb(255, 248, 220),
        "crimson" => Color::rgb(220, 20, 60),
        "darkblue" => Color::rgb(0, 0, 139),
        "darkcyan" => Color::rgb(0, 139, 139),
        "darkgoldenrod" => Color::rgb(184, 134, 11),
        "darkgray" | "darkgrey" => Color::rgb(169, 169, 169),
        "darkgreen" => Color::rgb(0, 100, 0),
        "darkkhaki" => Color::rgb(189, 183, 107),
        "darkmagenta" => Color::rgb(139, 0, 139),
        "darkolivegreen" => Color::rgb(85, 107, 47),
        "darkorange" => Color::rgb(255, 140, 0),
        "darkorchid" => Color::rgb(153, 50, 204),
        "darkred" => Color::rgb(139, 0, 0),
        "darksalmon" => Color::rgb(233, 150, 122),
        "darkseagreen" => Color::rgb(143, 188, 143),
        "darkslateblue" => Color::rgb(72, 61, 139),
        "darkslategray" | "darkslategrey" => Color::rgb(47, 79, 79),
        "darkturquoise" => Color::rgb(0, 206, 209),
        "darkviolet" => Color::rgb(148, 0, 211),
        "deeppink" => Color::rgb(255, 20, 147),
        "deepskyblue" => Color::rgb(0, 191, 255),
        "dimgray" | "dimgrey" => Color::rgb(105, 105, 105),
        "dodgerblue" => Color::rgb(30, 144, 255),
        "firebrick" => Color::rgb(178, 34, 34),
        "floralwhite" => Color::rgb(255, 250, 240),
        "forestgreen" => Color::rgb(34, 139, 34),
        "gainsboro" => Color::rgb(220, 220, 220),
        "ghostwhite" => Color::rgb(248, 248, 255),
        "gold" => Color::rgb(255, 215, 0),
        "goldenrod" => Color::rgb(218, 165, 32),
        "greenyellow" => Color::rgb(173, 255, 47),
        "honeydew" => Color::rgb(240, 255, 240),
        "hotpink" => Color::rgb(255, 105, 180),
        "indianred" => Color::rgb(205, 92, 92),
        "indigo" => Color::rgb(75, 0, 130),
        "ivory" => Color::rgb(255, 255, 240),
        "khaki" => Color::rgb(240, 230, 140),
        "lavender" => Color::rgb(230, 230, 250),
        "lavenderblush" => Color::rgb(255, 240, 245),
        "lawngreen" => Color::rgb(124, 252, 0),
        "lemonchiffon" => Color::rgb(255, 250, 205),
        "lightblue" => Color::rgb(173, 216, 230),
        "lightcoral" => Color::rgb(240, 128, 128),
        "lightcyan" => Color::rgb(224, 255, 255),
        "lightgoldenrodyellow" => Color::rgb(250, 250, 210),
        "lightgray" | "lightgrey" => Color::rgb(211, 211, 211),
        "lightgreen" => Color::rgb(144, 238, 144),
        "lightpink" => Color::rgb(255, 182, 193),
        "lightsalmon" => Color::rgb(255, 160, 122),
        "lightseagreen" => Color::rgb(32, 178, 170),
        "lightskyblue" => Color::rgb(135, 206, 250),
        "lightslategray" | "lightslategrey" => Color::rgb(119, 136, 153),
        "lightsteelblue" => Color::rgb(176, 196, 222),
        "lightyellow" => Color::rgb(255, 255, 224),
        "limegreen" => Color::rgb(50, 205, 50),
        "linen" => Color::rgb(250, 240, 230),
        "mediumaquamarine" => Color::rgb(102, 205, 170),
        "mediumblue" => Color::rgb(0, 0, 205),
        "mediumorchid" => Color::rgb(186, 85, 211),
        "mediumpurple" => Color::rgb(147, 112, 219),
        "mediumseagreen" => Color::rgb(60, 179, 113),
        "mediumslateblue" => Color::rgb(123, 104, 238),
        "mediumspringgreen" => Color::rgb(0, 250, 154),
        "mediumturquoise" => Color::rgb(72, 209, 204),
        "mediumvioletred" => Color::rgb(199, 21, 133),
        "midnightblue" => Color::rgb(25, 25, 112),
        "mintcream" => Color::rgb(245, 255, 250),
        "mistyrose" => Color::rgb(255, 228, 225),
        "moccasin" => Color::rgb(255, 228, 181),
        "navajowhite" => Color::rgb(255, 222, 173),
        "oldlace" => Color::rgb(253, 245, 230),
        "olivedrab" => Color::rgb(107, 142, 35),
        "orangered" => Color::rgb(255, 69, 0),
        "orchid" => Color::rgb(218, 112, 214),
        "palegoldenrod" => Color::rgb(238, 232, 170),
        "palegreen" => Color::rgb(152, 251, 152),
        "paleturquoise" => Color::rgb(175, 238, 238),
        "palevioletred" => Color::rgb(219, 112, 147),
        "papayawhip" => Color::rgb(255, 239, 213),
        "peachpuff" => Color::rgb(255, 218, 185),
        "peru" => Color::rgb(205, 133, 63),
        "pink" => Color::rgb(255, 192, 203),
        "plum" => Color::rgb(221, 160, 221),
        "powderblue" => Color::rgb(176, 224, 230),
        "rosybrown" => Color::rgb(188, 143, 143),
        "royalblue" => Color::rgb(65, 105, 225),
        "saddlebrown" => Color::rgb(139, 69, 19),
        "salmon" => Color::rgb(250, 128, 114),
        "sandybrown" => Color::rgb(244, 164, 96),
        "seagreen" => Color::rgb(46, 139, 87),
        "seashell" => Color::rgb(255, 245, 238),
        "sienna" => Color::rgb(160, 82, 45),
        "skyblue" => Color::rgb(135, 206, 235),
        "slateblue" => Color::rgb(106, 90, 205),
        "slategray" | "slategrey" => Color::rgb(112, 128, 144),
        "snow" => Color::rgb(255, 250, 250),
        "springgreen" => Color::rgb(0, 255, 127),
        "steelblue" => Color::rgb(70, 130, 180),
        "tan" => Color::rgb(210, 180, 140),
        "thistle" => Color::rgb(216, 191, 216),
        "tomato" => Color::rgb(255, 99, 71),
        "turquoise" => Color::rgb(64, 224, 208),
        "violet" => Color::rgb(238, 130, 238),
        "wheat" => Color::rgb(245, 222, 179),
        "whitesmoke" => Color::rgb(245, 245, 245),
        "yellowgreen" => Color::rgb(154, 205, 50),
        "rebeccapurple" => Color::rgb(102, 51, 153),
        _ => return None,
    };
    Some(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_colours() {
        assert_eq!(Color::parse("#fff"), Some(Color::WHITE));
        assert_eq!(Color::parse("#00000080"), Some(Color::rgba(0, 0, 0, 128)));
        assert_eq!(Color::parse("rgb(1, 2, 3)"), Some(Color::rgb(1, 2, 3)));
        assert_eq!(Color::parse("rgba(1,2,3,0.5)"), Some(Color::rgba(1, 2, 3, 128)));
        assert_eq!(Color::parse("rgb(100% 0% 0%)"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(Color::parse("hsl(0, 100%, 50%)"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(Color::parse("rebeccapurple"), Some(Color::rgb(102, 51, 153)));
        assert!(Color::parse("currentcolor").is_none());
    }

    #[test]
    fn composites() {
        let out = Color::rgba(255, 0, 0, 128).over(Color::WHITE);
        assert!(out.r == 255 && out.g > 120 && out.g < 140);
    }
}
