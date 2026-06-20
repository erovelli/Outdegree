//! Pure interpretation helpers (§7.2): transition classification, host
//! extraction, registrable-domain (eTLD+1) resolution, and node keying.
//!
//! Uses only the `url` and `psl` crates. `psl` embeds the Public Suffix List at
//! compile time (no runtime fetch — §12.1).

use crate::model::{Granularity, Provenance};
use url::Url;

/// Map a Chrome `webNavigation` `transitionType` to a [`Provenance`] (§7.2).
///
/// Subframe transitions never reach here: the service worker only records
/// `frameId 0` navigations.
pub fn classify(transition_type: &str) -> Provenance {
    match transition_type {
        "link" => Provenance::Link,
        "form_submit" => Provenance::Form,
        "typed" => Provenance::TypedUrl,
        "generated" | "keyword" | "keyword_generated" => Provenance::SearchOrigin,
        "auto_bookmark" => Provenance::Bookmark,
        "start_page" => Provenance::Start,
        "reload" => Provenance::Reload,
        _ => Provenance::Other,
    }
}

/// Extract the host of an `http(s)` URL (decision #10). Returns `None` for any
/// other scheme (`chrome://`, `file://`, `about:`, …) so they are dropped.
pub fn host(raw: &str) -> Option<String> {
    let u = Url::parse(raw).ok()?;
    match u.scheme() {
        "http" | "https" => u.host_str().map(|h| h.to_string()),
        _ => None,
    }
}

/// Registrable domain (eTLD+1) using the **ICANN** section of the Public Suffix
/// List only.
///
/// This deliberately ignores PSL *private* entries so that, per the §11
/// fixtures, `erovelli.github.io -> github.io` and `gist.github.com ->
/// github.com`. `psl`'s longest match would otherwise treat `github.io`
/// (a private suffix) as the eTLD. IPs / `localhost` / unknown TLDs fall back to
/// the literal host (decision #10).
pub fn registrable(host: &str) -> String {
    let host = host.trim_end_matches('.');
    let labels: Vec<&str> = host.split('.').collect();
    for i in 0..labels.len() {
        let candidate = labels[i..].join(".");
        if let Some(s) = psl::suffix(candidate.as_bytes()) {
            let is_icann = matches!(s.typ(), Some(psl::Type::Icann));
            if is_icann && s.as_bytes() == candidate.as_bytes() {
                // `candidate` is exactly an ICANN public suffix; one label up is
                // the registrable domain.
                if i == 0 {
                    // The whole host is a public suffix; nothing registrable.
                    return host.to_string();
                }
                return labels[i - 1..].join(".");
            }
        }
    }
    // psl-None (IPs, localhost, unknown TLDs): literal-host fallback.
    host.to_string()
}

/// Compute the graph node key for a URL at a given granularity (§3.3, §3.10).
/// `None` when the URL is not `http(s)` (those navigations are not graphed).
pub fn node_key(url: &str, g: Granularity) -> Option<String> {
    let h = host(url)?;
    Some(match g {
        Granularity::Hostname => h,
        Granularity::Registrable => registrable(&h),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_every_transition_type() {
        assert_eq!(classify("link"), Provenance::Link);
        assert_eq!(classify("form_submit"), Provenance::Form);
        assert_eq!(classify("typed"), Provenance::TypedUrl);
        assert_eq!(classify("generated"), Provenance::SearchOrigin);
        assert_eq!(classify("keyword"), Provenance::SearchOrigin);
        assert_eq!(classify("keyword_generated"), Provenance::SearchOrigin);
        assert_eq!(classify("auto_bookmark"), Provenance::Bookmark);
        assert_eq!(classify("start_page"), Provenance::Start);
        assert_eq!(classify("reload"), Provenance::Reload);
        assert_eq!(classify("auto_subframe"), Provenance::Other);
        assert_eq!(classify("whatever"), Provenance::Other);
    }

    #[test]
    fn host_only_http_https() {
        assert_eq!(
            host("https://example.com/path?q=1"),
            Some("example.com".into())
        );
        assert_eq!(host("http://localhost:3000/x"), Some("localhost".into()));
        assert_eq!(host("chrome://extensions"), None);
        assert_eq!(host("file:///home/user/x.html"), None);
        assert_eq!(host("about:blank"), None);
        assert_eq!(host("not a url"), None);
    }

    #[test]
    fn registrable_icann_only() {
        // Private PSL entries (github.io) are ignored: ICANN eTLD is `io`.
        assert_eq!(registrable("erovelli.github.io"), "github.io");
        assert_eq!(registrable("gist.github.com"), "github.com");
        assert_eq!(registrable("www.example.com"), "example.com");
        assert_eq!(registrable("example.com"), "example.com");
        assert_eq!(registrable("a.b.example.co.uk"), "example.co.uk");
    }

    #[test]
    fn registrable_literal_fallback() {
        assert_eq!(registrable("localhost"), "localhost");
        assert_eq!(registrable("127.0.0.1"), "127.0.0.1");
        assert_eq!(registrable("192.168.0.1"), "192.168.0.1");
    }

    #[test]
    fn node_key_granularity_and_drops() {
        assert_eq!(
            node_key("https://gist.github.com/x", Granularity::Hostname),
            Some("gist.github.com".into())
        );
        assert_eq!(
            node_key("https://gist.github.com/x", Granularity::Registrable),
            Some("github.com".into())
        );
        assert_eq!(node_key("chrome://newtab", Granularity::Hostname), None);
        assert_eq!(node_key("file:///x", Granularity::Registrable), None);
    }
}
