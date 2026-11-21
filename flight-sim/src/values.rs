//! Value scaling between KnobDeck's dials and SimConnect's event data: steps, headings,
//! radio frequencies (MHz / kHz parts, channel spacing), transponder codes (BCD16), the
//! altimeter setting (millibars × 16) and trim / flaps / spoiler positions. No I/O.

/// Whole dial steps in a `delta` (at least one when it isn't zero).
pub fn steps(delta: f64) -> i64 {
    if delta == 0.0 || !delta.is_finite() {
        return 0;
    }
    let n = delta.round() as i64;
    if n == 0 {
        delta.signum() as i64
    } else {
        n
    }
}

/// `cur` moved by `steps` × `step`, landing on the `step` grid: from 3450 one step of 100 up
/// is 3500, not 3550.
pub fn snap_step(cur: f64, steps: i64, step: f64) -> f64 {
    let at = cur / step;
    let on_grid = (at - at.round()).abs() < 1e-6;
    let base = if on_grid {
        at.round()
    } else if steps > 0 {
        at.ceil() - 1.0
    } else if steps < 0 {
        at.floor() + 1.0
    } else {
        at.round()
    };
    (base + steps as f64) * step
}

/// The new target for a slider: the dial's absolute `value`, else `cur` moved by `delta` steps.
/// `None` when the invoke carried neither.
pub fn target(cur: f64, value: Option<f64>, delta: Option<f64>, step: f64) -> Option<f64> {
    match (value, delta) {
        (Some(v), _) if v.is_finite() => Some(v),
        (_, Some(d)) if steps(d) != 0 => Some(snap_step(cur, steps(d), step)),
        _ => None,
    }
}

/// A heading or course in whole degrees, 0–359.
pub fn wrap_deg(v: f64) -> u32 {
    (v.round() as i64).rem_euclid(360) as u32
}

/// A heading as pilots say it: three digits, 360 for north.
pub fn fmt_deg(deg: u32) -> String {
    format!("{:03}°", if deg % 360 == 0 { 360 } else { deg % 360 })
}

/// A signed value as SimConnect's DWORD event data (two's complement, as the sim reads it).
pub fn dword(v: i64) -> u32 {
    v as i32 as u32
}

/// Channel spacing of a radio's kHz knob.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Spacing {
    /// COM, 25 kHz: .000 .025 … .975
    Com25,
    /// COM, 8.33 kHz channel names: every 5 kHz except the x20 / x45 / x70 / x95 ends.
    Com833,
    /// NAV, 50 kHz: .00 .05 … .95
    Nav50,
}

impl Spacing {
    /// Channels in one MHz.
    fn channels(self) -> u32 {
        match self {
            Spacing::Com25 => 40,
            Spacing::Com833 => 160,
            Spacing::Nav50 => 20,
        }
    }
    /// The channel a kHz part is on (the one below it when it is between channels).
    fn index(self, khz: u32) -> u32 {
        let khz = khz.min(999);
        match self {
            Spacing::Com25 => khz / 25,
            Spacing::Com833 => (khz / 25) * 4 + ((khz % 25) / 5).min(3),
            Spacing::Nav50 => khz / 50,
        }
    }
    fn khz(self, index: u32) -> u32 {
        match self {
            Spacing::Com25 => index * 25,
            Spacing::Com833 => (index / 4) * 25 + (index % 4) * 5,
            Spacing::Nav50 => index * 50,
        }
    }
    /// MHz range of the band.
    pub fn mhz_range(self) -> (u32, u32) {
        match self {
            Spacing::Com25 | Spacing::Com833 => (118, 136),
            Spacing::Nav50 => (108, 117),
        }
    }
}

/// A frequency in whole kHz from the sim's MHz reading.
pub fn khz_of_mhz(mhz: f64) -> u32 {
    (mhz * 1000.0).round().max(0.0) as u32
}

/// The event data of `COM_STBY_RADIO_SET_HZ` / `NAV1_STBY_SET_HZ`: plain Hz.
pub fn hz(khz: u32) -> u32 {
    khz * 1000
}

/// `freq` (kHz) with its MHz part set to `mhz`, clamped to the band.
pub fn with_mhz(freq: u32, mhz: f64, spacing: Spacing) -> u32 {
    let (lo, hi) = spacing.mhz_range();
    let m = (mhz.round().max(0.0) as u32).clamp(lo, hi);
    m * 1000 + freq % 1000
}

/// `freq` (kHz) with its MHz part moved by `steps`, wrapping within the band like the outer knob.
pub fn step_mhz(freq: u32, steps: i64, spacing: Spacing) -> u32 {
    let (lo, hi) = spacing.mhz_range();
    let span = (hi - lo + 1) as i64;
    let m = ((freq / 1000).clamp(lo, hi) - lo) as i64;
    let m = (m + steps).rem_euclid(span) as u32 + lo;
    m * 1000 + freq % 1000
}

/// `freq` (kHz) with its kHz part set to the channel nearest at or below `khz`.
pub fn with_khz(freq: u32, khz: f64, spacing: Spacing) -> u32 {
    let k = khz.round().clamp(0.0, 999.0) as u32;
    let i = spacing.index(k).min(spacing.channels() - 1);
    (freq / 1000) * 1000 + spacing.khz(i)
}

/// `freq` (kHz) with its kHz part moved by `steps` channels, wrapping within the MHz like the
/// inner knob (no carry into the MHz).
pub fn step_khz(freq: u32, steps: i64, spacing: Spacing) -> u32 {
    let n = spacing.channels() as i64;
    let i = spacing.index(freq % 1000) as i64;
    let i = (i + steps).rem_euclid(n) as u32;
    (freq / 1000) * 1000 + spacing.khz(i)
}

/// "118.025" (COM) or "110.50" (NAV).
pub fn fmt_freq(khz: u32, spacing: Spacing) -> String {
    match spacing {
        Spacing::Nav50 => format!("{}.{:02}", khz / 1000, (khz % 1000) / 10),
        _ => format!("{}.{:03}", khz / 1000, khz % 1000),
    }
}

/// A 4-digit transponder code (each digit 0–7) from text: "7000", "1200".
pub fn parse_code(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.len() != 4 || !s.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
        return None;
    }
    s.parse().ok()
}

/// A code's digit at `place` (0 = the first, thousands digit … 3 = the last).
pub fn code_digit(code: u32, place: u32) -> u32 {
    (code / 10u32.pow(3 - place.min(3))) % 10
}

/// `code` with the digit at `place` set to `digit` (0–7).
pub fn with_digit(code: u32, place: u32, digit: u32) -> u32 {
    let p = 10u32.pow(3 - place.min(3));
    code - code_digit(code, place) * p + digit.min(7) * p
}

/// `code` with the digit at `place` moved by `steps`, wrapping 7 → 0 like the knob.
pub fn step_digit(code: u32, place: u32, steps: i64) -> u32 {
    let d = (code_digit(code, place) as i64 + steps).rem_euclid(8) as u32;
    with_digit(code, place, d)
}

/// The event data of `XPNDR_SET`: the code's digits as BCD16 nibbles (7700 → 0x7700).
pub fn bcd16(code: u32) -> u32 {
    (0..4).fold(0, |acc, place| (acc << 4) | code_digit(code, place))
}

/// A code from the sim's BCD16 reading (0x1200 → 1200); `None` for a nibble that isn't 0–7.
pub fn from_bcd16(raw: f64) -> Option<u32> {
    if !raw.is_finite() || raw < 0.0 || raw > 65535.0 {
        return None;
    }
    let raw = raw.round() as u32;
    let mut code = 0;
    for shift in [12, 8, 4, 0] {
        let n = (raw >> shift) & 0xF;
        if n > 7 {
            return None;
        }
        code = code * 10 + n;
    }
    Some(code)
}

/// hPa per inHg.
pub const HPA_PER_INHG: f64 = 33.8639;
/// Standard pressure.
pub const STD_HPA: f64 = 1013.25;

/// The event data of `KOHLSMAN_SET`: millibars × 16.
pub fn kohlsman(hpa: f64) -> u32 {
    (hpa * 16.0).round().max(0.0) as u32
}

pub fn inhg(hpa: f64) -> f64 {
    hpa / HPA_PER_INHG
}

pub fn hpa(inhg: f64) -> f64 {
    inhg * HPA_PER_INHG
}

/// Round to `decimals` places.
pub fn round_to(v: f64, decimals: i32) -> f64 {
    let p = 10f64.powi(decimals);
    (v * p).round() / p
}

/// A 0–100 % position as the 0–16383 data of `SPOILERS_SET` / `FLAPS_SET`.
pub fn pos16k(pct: f64) -> u32 {
    (pct.clamp(0.0, 100.0) / 100.0 * 16383.0).round() as u32
}

/// The flap notch (0..=`positions`) nearest a 0–100 % handle position.
pub fn flap_notch(pct: f64, positions: u32) -> u32 {
    ((pct.clamp(0.0, 100.0) / 100.0) * positions as f64).round() as u32
}

/// `FLAPS_SET` data for notch `n` of `positions`.
pub fn flaps_data(n: u32, positions: u32) -> u32 {
    if positions == 0 {
        return 0;
    }
    pos16k(n.min(positions) as f64 * 100.0 / positions as f64)
}

/// Trim axes and how their SET event scales a −100…100 % position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrimScale {
    /// `ELEVATOR_TRIM_SET`: −16383…16383.
    Full,
    /// `AILERON_TRIM_SET` / `RUDDER_TRIM_SET`: −100…100.
    Percent,
}

pub fn trim_data(pct: f64, scale: TrimScale) -> u32 {
    let pct = pct.clamp(-100.0, 100.0);
    match scale {
        TrimScale::Full => dword((pct / 100.0 * 16383.0).round() as i64),
        TrimScale::Percent => dword(pct.round() as i64),
    }
}

/// Sim rate as shown: "1×", "0.5×", "16×".
pub fn fmt_rate(rate: f64) -> String {
    let r = round_to(rate, 2);
    if r.fract() == 0.0 {
        format!("{}×", r as i64)
    } else {
        format!("{r}×")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_from_delta() {
        assert_eq!(steps(0.0), 0);
        assert_eq!(steps(0.2), 1);
        assert_eq!(steps(-0.2), -1);
        assert_eq!(steps(3.0), 3);
        assert_eq!(steps(-2.6), -3);
        assert_eq!(steps(f64::NAN), 0);
    }

    #[test]
    fn snapping_steps() {
        assert_eq!(snap_step(3450.0, 1, 100.0), 3500.0);
        assert_eq!(snap_step(3450.0, -1, 100.0), 3400.0);
        assert_eq!(snap_step(3500.0, 1, 100.0), 3600.0);
        assert_eq!(snap_step(3500.0, -2, 100.0), 3300.0);
        assert_eq!(snap_step(-650.0, -1, 100.0), -700.0);
        assert!((snap_step(29.92, 1, 0.01) - 29.93).abs() < 1e-9);
    }

    #[test]
    fn slider_target() {
        assert_eq!(target(100.0, Some(250.0), None, 1.0), Some(250.0));
        assert_eq!(target(100.0, None, Some(2.0), 10.0), Some(120.0));
        assert_eq!(target(100.0, None, Some(0.0), 10.0), None);
        assert_eq!(target(100.0, None, None, 10.0), None);
    }

    #[test]
    fn headings_wrap() {
        assert_eq!(wrap_deg(360.0), 0);
        assert_eq!(wrap_deg(361.0), 1);
        assert_eq!(wrap_deg(-1.0), 359);
        assert_eq!(wrap_deg(359.6), 0);
        assert_eq!(fmt_deg(0), "360°");
        assert_eq!(fmt_deg(5), "005°");
        assert_eq!(fmt_deg(270), "270°");
    }

    #[test]
    fn negative_dwords() {
        assert_eq!(dword(-500), 0xFFFF_FE0C);
        assert_eq!(dword(700), 700);
    }

    #[test]
    fn com_25_khz() {
        let f = 118_000;
        assert_eq!(step_khz(f, 1, Spacing::Com25), 118_025);
        assert_eq!(step_khz(f, -1, Spacing::Com25), 118_975);
        assert_eq!(step_khz(118_975, 1, Spacing::Com25), 118_000, "no carry into the MHz");
        assert_eq!(with_khz(f, 990.0, Spacing::Com25), 118_975);
        assert_eq!(with_khz(f, 510.0, Spacing::Com25), 118_500);
        // An 8.33 channel left over from before snaps to the 25 kHz channel below it.
        assert_eq!(step_khz(118_005, 1, Spacing::Com25), 118_025);
    }

    #[test]
    fn com_833_channels() {
        let mut f = 118_000;
        let mut seen = vec![];
        for _ in 0..9 {
            f = step_khz(f, 1, Spacing::Com833);
            seen.push(f % 1000);
        }
        assert_eq!(seen, vec![5, 10, 15, 25, 30, 35, 40, 50, 55]);
        assert_eq!(step_khz(118_000, -1, Spacing::Com833), 118_990);
        assert_eq!(with_khz(f, 20.0, Spacing::Com833), 118_015, "x20 is no channel name");
        assert_eq!(with_khz(f, 990.0, Spacing::Com833), 118_990);
    }

    #[test]
    fn mhz_parts() {
        assert_eq!(step_mhz(136_975, 1, Spacing::Com25), 118_975);
        assert_eq!(step_mhz(118_500, -1, Spacing::Com25), 136_500);
        assert_eq!(with_mhz(121_500, 124.0, Spacing::Com25), 124_500);
        assert_eq!(with_mhz(121_500, 140.0, Spacing::Com25), 136_500);
        assert_eq!(step_mhz(117_950, 1, Spacing::Nav50), 108_950);
        assert_eq!(step_khz(110_950, 1, Spacing::Nav50), 110_000);
        assert_eq!(with_khz(110_000, 330.0, Spacing::Nav50), 110_300);
    }

    #[test]
    fn frequency_scaling() {
        assert_eq!(khz_of_mhz(118.025), 118_025);
        assert_eq!(khz_of_mhz(121.49999), 121_500);
        assert_eq!(hz(118_025), 118_025_000);
        assert_eq!(fmt_freq(118_025, Spacing::Com25), "118.025");
        assert_eq!(fmt_freq(118_005, Spacing::Com833), "118.005");
        assert_eq!(fmt_freq(110_500, Spacing::Nav50), "110.50");
    }

    #[test]
    fn transponder_codes() {
        assert_eq!(parse_code("7000"), Some(7000));
        assert_eq!(parse_code(" 1200 "), Some(1200));
        assert_eq!(parse_code("7800"), None, "8 is no transponder digit");
        assert_eq!(parse_code("700"), None);
        assert_eq!(bcd16(7700), 0x7700);
        assert_eq!(bcd16(1200), 0x1200);
        assert_eq!(bcd16(0), 0);
        assert_eq!(from_bcd16(0x7700 as f64), Some(7700));
        assert_eq!(from_bcd16(4608.0), Some(1200));
        assert_eq!(from_bcd16(0x1280 as f64), None);
        for code in [0, 1200, 2000, 7777, 4321, 7500] {
            assert_eq!(from_bcd16(bcd16(code) as f64), Some(code));
        }
    }

    #[test]
    fn transponder_digits() {
        assert_eq!(code_digit(1234, 0), 1);
        assert_eq!(code_digit(1234, 3), 4);
        assert_eq!(with_digit(1200, 1, 5), 1500);
        assert_eq!(step_digit(1270, 2, 1), 1200, "7 wraps to 0");
        assert_eq!(step_digit(1200, 3, -1), 1207);
        assert_eq!(step_digit(7000, 0, 3), 2000);
    }

    #[test]
    fn altimeter() {
        assert_eq!(kohlsman(STD_HPA), 16212);
        assert_eq!(kohlsman(hpa(29.92)), 16211);
        assert!((inhg(1013.25) - 29.92).abs() < 0.005);
    }

    #[test]
    fn positions() {
        assert_eq!(pos16k(0.0), 0);
        assert_eq!(pos16k(100.0), 16383);
        assert_eq!(pos16k(150.0), 16383);
        assert_eq!(flap_notch(50.0, 4), 2);
        assert_eq!(flap_notch(60.0, 3), 2);
        assert_eq!(flaps_data(2, 4), 8192);
        assert_eq!(flaps_data(5, 4), 16383);
        assert_eq!(flaps_data(1, 0), 0);
    }

    #[test]
    fn trim() {
        assert_eq!(trim_data(100.0, TrimScale::Full), 16383);
        assert_eq!(trim_data(-50.0, TrimScale::Full), dword(-8192));
        assert_eq!(trim_data(-12.4, TrimScale::Percent), dword(-12));
        assert_eq!(trim_data(250.0, TrimScale::Percent), 100);
    }

    #[test]
    fn rates() {
        assert_eq!(fmt_rate(1.0), "1×");
        assert_eq!(fmt_rate(0.25), "0.25×");
        assert_eq!(fmt_rate(16.0), "16×");
    }
}
