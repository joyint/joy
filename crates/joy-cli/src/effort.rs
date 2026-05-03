// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

use anyhow::{anyhow, Result};

/// Parse an effort value from a string.
///
/// Accepts:
/// - "none" -> Ok(None)
/// - "1".."7" -> Ok(Some(1..=7))
/// - T-shirt sizes (case-insensitive): xxs=1, xs=2, s=3, m=4, l=5, xl=6, xxl=7
pub fn parse_effort(s: &str) -> Result<Option<u8>> {
    let trimmed = s.trim();
    if trimmed.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    if let Ok(n) = trimmed.parse::<u8>() {
        if (1..=7).contains(&n) {
            return Ok(Some(n));
        }
        return Err(anyhow!("effort must be between 1 and 7"));
    }
    let n = match trimmed.to_ascii_lowercase().as_str() {
        "xxs" => 1,
        "xs" => 2,
        "s" => 3,
        "m" => 4,
        "l" => 5,
        "xl" => 6,
        "xxl" => 7,
        _ => {
            return Err(anyhow!(
                "effort must be 1-7, a t-shirt size (xxs, xs, s, m, l, xl, xxl), or 'none'"
            ));
        }
    };
    Ok(Some(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_none() {
        assert_eq!(parse_effort("none").unwrap(), None);
        assert_eq!(parse_effort("NONE").unwrap(), None);
        assert_eq!(parse_effort(" none ").unwrap(), None);
    }

    #[test]
    fn parses_numeric() {
        for n in 1..=7u8 {
            assert_eq!(parse_effort(&n.to_string()).unwrap(), Some(n));
        }
    }

    #[test]
    fn parses_tshirt_sizes() {
        assert_eq!(parse_effort("xxs").unwrap(), Some(1));
        assert_eq!(parse_effort("xs").unwrap(), Some(2));
        assert_eq!(parse_effort("s").unwrap(), Some(3));
        assert_eq!(parse_effort("m").unwrap(), Some(4));
        assert_eq!(parse_effort("l").unwrap(), Some(5));
        assert_eq!(parse_effort("xl").unwrap(), Some(6));
        assert_eq!(parse_effort("xxl").unwrap(), Some(7));
    }

    #[test]
    fn parses_tshirt_case_insensitive() {
        assert_eq!(parse_effort("XXS").unwrap(), Some(1));
        assert_eq!(parse_effort("Xl").unwrap(), Some(6));
        assert_eq!(parse_effort(" XXL ").unwrap(), Some(7));
    }

    #[test]
    fn rejects_out_of_range_numbers() {
        assert!(parse_effort("0").is_err());
        assert!(parse_effort("8").is_err());
        assert!(parse_effort("100").is_err());
    }

    #[test]
    fn rejects_unknown_strings() {
        assert!(parse_effort("huge").is_err());
        assert!(parse_effort("").is_err());
        assert!(parse_effort("xxxl").is_err());
    }
}
