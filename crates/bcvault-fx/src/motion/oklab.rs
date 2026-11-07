use egui::Color32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OkLab {
    pub l: f32,
    pub a: f32,
    pub b: f32,
    pub alpha: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OkLch {
    pub l: f32,
    pub c: f32,
    pub h_deg: f32,
}

fn srgb_to_linear(c: u8) -> f32 {
    let v = f32::from(c) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(v: f32) -> u8 {
    let clamped = v.clamp(0.0, 1.0);
    let s = if clamped <= 0.003_130_8 {
        12.92 * clamped
    } else {
        1.055 * clamped.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round().clamp(0.0, 255.0) as u8
}

impl OkLab {
    pub fn from_color32(c: Color32) -> Self {
        let [r, g, b, a] = c.to_srgba_unmultiplied();
        Self::from_rgb([r, g, b], f32::from(a) / 255.0)
    }

    pub fn from_rgb([r, g, b]: [u8; 3], alpha: f32) -> Self {
        let lr = srgb_to_linear(r);
        let lg = srgb_to_linear(g);
        let lb = srgb_to_linear(b);

        let l = 0.412_221_46 * lr + 0.536_332_55 * lg + 0.051_445_995 * lb;
        let m = 0.211_903_5 * lr + 0.680_699_5 * lg + 0.107_396_96 * lb;
        let s = 0.088_302_46 * lr + 0.281_718_85 * lg + 0.629_978_7 * lb;

        let l_ = l.max(0.0).cbrt();
        let m_ = m.max(0.0).cbrt();
        let s_ = s.max(0.0).cbrt();

        Self {
            l: 0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_,
            a: 1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_,
            b: 0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_77 * s_,
            alpha,
        }
    }

    pub fn to_rgb(self) -> [u8; 3] {
        let l_ = self.l + 0.396_337_78 * self.a + 0.215_803_76 * self.b;
        let m_ = self.l - 0.105_561_346 * self.a - 0.063_854_17 * self.b;
        let s_ = self.l - 0.089_484_18 * self.a - 1.291_485_5 * self.b;

        let l = l_ * l_ * l_;
        let m = m_ * m_ * m_;
        let s = s_ * s_ * s_;

        let lr = 4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s;
        let lg = -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s;
        let lb = -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s;

        [linear_to_srgb(lr), linear_to_srgb(lg), linear_to_srgb(lb)]
    }

    pub fn to_color32(self) -> Color32 {
        let [r, g, b] = self.to_rgb();
        let a = (self.alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        Color32::from_rgba_unmultiplied(r, g, b, a)
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            l: self.l + (other.l - self.l) * t,
            a: self.a + (other.a - self.a) * t,
            b: self.b + (other.b - self.b) * t,
            alpha: self.alpha + (other.alpha - self.alpha) * t,
        }
    }

    pub fn to_lch(self) -> OkLch {
        let c = (self.a * self.a + self.b * self.b).sqrt();
        let mut h = self.b.atan2(self.a).to_degrees();
        if h < 0.0 {
            h += 360.0;
        }
        OkLch {
            l: self.l,
            c,
            h_deg: h,
        }
    }
}

pub fn lerp_color32(a: Color32, b: Color32, t: f32) -> Color32 {
    OkLab::from_color32(a)
        .lerp(OkLab::from_color32(b), t)
        .to_color32()
}

pub fn relative_luminance(c: Color32) -> f32 {
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    let lr = srgb_to_linear(r);
    let lg = srgb_to_linear(g);
    let lb = srgb_to_linear(b);
    0.2126 * lr + 0.7152 * lg + 0.0722 * lb
}

pub fn contrast_ratio(c1: Color32, c2: Color32) -> f32 {
    let l1 = relative_luminance(c1);
    let l2 = relative_luminance(c2);
    let (hi, lo) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

/// Adjusts `accent` lightness in OKLab so its contrast against `bg` is at least `min_ratio`.
pub fn clamp_accent_contrast(accent: Color32, bg: Color32, min_ratio: f32) -> Color32 {
    if contrast_ratio(accent, bg) >= min_ratio {
        return accent;
    }
    let bg_lum = relative_luminance(bg);
    let lighten = bg_lum < 0.35;
    let mut lab = OkLab::from_color32(accent);
    for _ in 0..32 {
        if lighten {
            lab.l = (lab.l + 0.025).min(0.96);
        } else {
            lab.l = (lab.l - 0.025).max(0.18);
        }
        let cand = lab.to_color32();
        if contrast_ratio(cand, bg) >= min_ratio {
            return cand;
        }
    }
    lab.to_color32()
}
