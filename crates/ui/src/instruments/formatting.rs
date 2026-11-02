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
            if digits && !nonzero {
                i += 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
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
