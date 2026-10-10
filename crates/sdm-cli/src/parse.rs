//! Parsers for command line values.

/// Parses a speed such as `500K`, `2M`, `1.5MB` or `750000` into bytes per
/// second. Units are powers of 1024.
pub fn rate(input: &str) -> Result<u64, String> {
    let text = input.trim();
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let value: f64 = number
        .parse()
        .map_err(|_| format!("`{input}` is not a speed (try 500K or 2M)"))?;

    let unit = unit.trim().to_ascii_lowercase();
    let unit = unit.strip_suffix("/s").unwrap_or(&unit);
    let unit = unit
        .strip_suffix("ib")
        .or_else(|| unit.strip_suffix('b'))
        .unwrap_or(unit);
    let multiplier: f64 = match unit {
        "" => 1.0,
        "k" => 1024.0,
        "m" => 1024.0 * 1024.0,
        "g" => 1024.0 * 1024.0 * 1024.0,
        _ => return Err(format!("unknown unit in `{input}` (use K, M or G)")),
    };

    let bytes = value * multiplier;
    if !bytes.is_finite() || bytes < 1.0 {
        return Err(format!("`{input}` is too small"));
    }
    Ok(bytes as u64)
}

/// Parses `Name: value`.
pub fn header(input: &str) -> Result<(String, String), String> {
    let (name, value) = input
        .split_once(':')
        .ok_or_else(|| format!("`{input}` is not a header (expected `Name: value`)"))?;
    let name = name.trim();
    if name.is_empty() || name.contains(char::is_whitespace) {
        return Err(format!("`{input}` has an invalid header name"));
    }
    Ok((name.to_owned(), value.trim().to_owned()))
}

/// Normalises a SHA-256 checksum written in hex.
pub fn sha256(input: &str) -> Result<String, String> {
    let hex = input.trim().to_ascii_lowercase();
    if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(hex)
    } else {
        Err(format!(
            "`{input}` is not a SHA-256 checksum (64 hex digits)"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rates() {
        assert_eq!(rate("750000"), Ok(750_000));
        assert_eq!(rate("500K"), Ok(500 * 1024));
        assert_eq!(rate("500kb"), Ok(500 * 1024));
        assert_eq!(rate("2M"), Ok(2 * 1024 * 1024));
        assert_eq!(rate("1.5MiB"), Ok(1536 * 1024));
        assert_eq!(rate("1G/s"), Ok(1024 * 1024 * 1024));
        assert_eq!(rate("10 B"), Ok(10));
        assert!(rate("fast").is_err());
        assert!(rate("5X").is_err());
        assert!(rate("0").is_err());
    }

    #[test]
    fn parses_headers() {
        assert_eq!(
            header("Authorization: Bearer a:b"),
            Ok(("Authorization".into(), "Bearer a:b".into()))
        );
        assert!(header("no colon").is_err());
        assert!(header("bad name: x").is_err());
        assert!(header(": x").is_err());
    }

    #[test]
    fn parses_checksums() {
        let hex = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD";
        assert_eq!(sha256(hex), Ok(hex.to_ascii_lowercase()));
        assert!(sha256("abc").is_err());
    }
}
