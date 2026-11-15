//! "The AI must never invent numbers": the check that enforces it.
//!
//! Every number in a comment must be one of the numbers in the snapshot the
//! model was given, possibly rounded to fewer decimals. A number that is not
//! — a sum, a difference, an estimate, a hallucination — makes the comment
//! unverified, and by default it is withheld from the dashboard.
//!
//! Numbers are compared as text tokens, so "1:32.4" in a comment matches the
//! "1:32.400" lap time it was copied from (both give 1 and 32.4).

/// Numbers in a text: integers and decimals, with "." or "," as the decimal
/// mark. Signs are dropped (a gap is written "+0.4" or "0.4" alike).
pub fn numbers(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            // A decimal part: mark followed by a digit.
            if i + 1 < chars.len() && (chars[i] == '.' || chars[i] == ',') && chars[i + 1].is_ascii_digit() {
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            out.push(chars[start..i].iter().collect::<String>().replace(',', "."));
        } else {
            i += 1;
        }
    }
    out
}

fn decimals(token: &str) -> usize {
    token.split_once('.').map(|(_, d)| d.len()).unwrap_or(0)
}

/// Whether `claimed` is `source` written with the same or fewer decimals.
fn matches(claimed: &str, source: &str) -> bool {
    let (Ok(c), Ok(s)) = (claimed.parse::<f64>(), source.parse::<f64>()) else { return false };
    let d = decimals(claimed);
    if d > decimals(source) {
        return false;
    }
    let scale = 10f64.powi(d as i32);
    // Rounded or truncated: both are honest ways to shorten a number.
    (s * scale).round() / scale == c || (s * scale).trunc() / scale == c
}

/// Small counts ("one stop", "two laps") are allowed as words or digits:
/// they carry no measured value of their own.
const FREE_INTEGERS: f64 = 3.0;

/// The numbers in `comment` that do not come from `source`.
pub fn unverified(comment: &str, source: &str) -> Vec<String> {
    let allowed = numbers(source);
    let mut out: Vec<String> = Vec::new();
    for n in numbers(comment) {
        let free = decimals(&n) == 0 && n.parse::<f64>().map(|v| v <= FREE_INTEGERS).unwrap_or(false);
        if free || allowed.iter().any(|a| matches(&n, a)) {
            continue;
        }
        if !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_tokenised_with_either_decimal_mark() {
        assert_eq!(numbers("P3, gap +1,25 s, lap 1:32.400, 4.5%/lap"), vec!["3", "1.25", "1", "32.400", "4.5"]);
        assert_eq!(numbers("no numbers here."), Vec::<String>::new());
        assert_eq!(numbers("ends with a dot 12."), vec!["12"]);
    }

    #[test]
    fn copied_and_rounded_numbers_pass() {
        let src = r#"{"fuel_per_lap":2.913,"laps_to_go":12.5,"last_lap":"1:32.400","class_position":"P4/8"}"#;
        assert!(unverified("Consumo 2.91 L/giro, 12.5 giri alla fine.", src).is_empty());
        assert!(unverified("Consumo 2,9 L/giro; ultimo giro 1:32.4; P4.", src).is_empty());
        // Truncation is accepted too.
        assert!(unverified("circa 2.9", r#"{"x":2.96}"#).is_empty());
        // Small counts are free.
        assert!(unverified("Due soste: 2.", src).is_empty());
    }

    #[test]
    fn invented_numbers_are_caught() {
        let src = r#"{"fuel_per_lap":2.913,"laps_to_go":12.5}"#;
        // A computed total (2.913 × 12.5 = 36.4) and a made-up figure.
        assert_eq!(unverified("Servono 36.4 L, guadagno di 0.8 s.", src), vec!["36.4", "0.8"]);
        // More decimals than the source has: not a rounding of it.
        assert_eq!(unverified("2.9134 L", src), vec!["2.9134"]);
        // A larger integer that is not in the data.
        assert_eq!(unverified("tra 7 giri", src), vec!["7"]);
        // Each number reported once.
        assert_eq!(unverified("7 e ancora 7", src), vec!["7"]);
    }
}
