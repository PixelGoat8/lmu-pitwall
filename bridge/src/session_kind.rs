//! Which sessions count. Pitwall usually starts well before the race, so the
//! practice before it must not leave anything behind: no AI spending, no pit
//! loss measurements, no history entry. The live widgets still work in it.

/// Practice, test day and warm-up, as `session_type_str` in main.rs names them.
pub fn is_practice(session_type: &str) -> bool {
    matches!(session_type, "Practice" | "TestDay" | "Warmup")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn practice_like_sessions() {
        for s in ["Practice", "TestDay", "Warmup"] {
            assert!(is_practice(s), "{s}");
        }
        for s in ["Qualifying", "Race", "Unknown", ""] {
            assert!(!is_practice(s), "{s}");
        }
    }
}
