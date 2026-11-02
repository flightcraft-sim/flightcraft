//! Formatting is shared so rounded zero never inherits a sensor's sign bit.
pub(crate) fn clean_zero(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if c == '-' || c == '+' {
            let mut j = i + 1;
            let mut digits = false;
            let mut nonzero = false;
            while let Some(&n) = chars.get(j) {
                if n.is_ascii_digit() {
                    digits = true;
                    nonzero |= n != '0';
                } else if n != '.' {
                    break;
                }
                j += 1;
            }
            if digits && !nonzero && !(c == '+' && out.ends_with("VS ")) {
                i += 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

pub(crate) fn integer(value: f64) -> String {
    let raw = format!("{:.0}", if value.abs() < 0.5 { 0.0 } else { value });
    let (sign, digits) = raw
        .strip_prefix('-')
        .map_or(("", raw.as_str()), |digits| ("-", digits));
    let mut out = String::from(sign);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
pub(crate) fn feet(metres: f64) -> String {
    integer(metres / 0.3048)
}
pub(crate) fn selected_feet(metres: f64) -> String {
    integer(libm::round(metres / 30.48) * 100.0)
}
pub(crate) fn vertical_speed(mps: f64) -> String {
    let rounded = libm::round(mps / 0.508) * 100.0;
    format!("{:+.0}", if rounded == 0.0 { 0.0 } else { rounded })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controller_units_have_explicit_precision() {
        assert_eq!(feet(1493.52), "4,900");
        assert_eq!(selected_feet(1500.0), "4,900");
        assert_eq!(vertical_speed(2.54), "+500");
        assert_eq!(vertical_speed(-2.54), "-500");
        assert_eq!(clean_zero(&format!("VS {}", vertical_speed(-0.01))), "VS +0");
    }
    #[test]
    fn rounded_zero_has_no_sign() {
        for (from, to) in [
            ("VS -0", "VS 0"),
            ("TURN -0.0°/s", "TURN 0.0°/s"),
            ("+0.0 amps", "0.0 amps"),
            ("-0.1", "-0.1"),
            ("-10", "-10"),
        ] {
            assert_eq!(clean_zero(from), to);
        }
    }
}
