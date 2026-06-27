//! Pure CSV serialization helpers (RFC 4180 quoting), shared by the dashboard's
//! "Export tables (CSV)" action. Kept pure so the quoting rules are unit-tested.

/// Quote a single CSV field per RFC 4180: wrap in double quotes (doubling any
/// interior quote) when the value contains a comma, quote, CR, or LF; otherwise
/// emit it verbatim.
pub fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Join `fields` into one CSV record (no trailing newline).
pub fn csv_line(fields: &[&str]) -> String {
    fields
        .iter()
        .map(|f| csv_field(f))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_field_quotes_only_when_needed() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("line\nbreak"), "\"line\nbreak\"");
        assert_eq!(csv_field(""), "");
    }

    #[test]
    fn csv_line_joins_and_escapes() {
        assert_eq!(csv_line(&["host", "a,b.com", "12"]), "host,\"a,b.com\",12");
        assert_eq!(csv_line(&[]), "");
    }
}
