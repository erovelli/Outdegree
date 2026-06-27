//! Pure search-query extraction from already-captured navigation URLs.
//!
//! This reads only URLs the extension already stored (no new capture, no network,
//! no extra permission). Because search terms are sensitive, the dashboard keeps
//! this surface **off by default** and only renders it when the user opts in.
//!
//! Kept pure (parsing + aggregation) so the engine table and ranking are
//! unit-tested under `cargo test`; the wasm UI only reads events and renders.

use url::Url;

/// A recognized search-engine result URL decomposed into the engine's display
/// name and the decoded query terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchQuery {
    pub engine: &'static str,
    pub terms: String,
}

/// An aggregated, ranked search term (how often a given query was issued).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchCount {
    pub engine: String,
    pub terms: String,
    pub count: u32,
}

/// Resolve a known search engine for `(host, path)`, returning its display name
/// and the query-string parameter that carries the terms. Matching is restricted
/// (engine host + result path) to avoid mistaking, say, a maps `?q=` for a web
/// search. `host` is lower-cased and `www.`-stripped by the caller.
fn engine_for(host: &str, path: &str) -> Option<(&'static str, &'static str)> {
    // Google web search: any ccTLD (google.com, google.co.uk, …), path /search.
    if (host == "google.com" || host.starts_with("google.")) && path == "/search" {
        return Some(("Google", "q"));
    }
    match (host, path) {
        ("bing.com", "/search") => Some(("Bing", "q")),
        ("duckduckgo.com", _) => Some(("DuckDuckGo", "q")),
        ("search.brave.com", "/search") => Some(("Brave", "q")),
        ("ecosia.org", "/search") => Some(("Ecosia", "q")),
        ("startpage.com", _) => Some(("Startpage", "query")),
        ("youtube.com" | "m.youtube.com", "/results") => Some(("YouTube", "search_query")),
        _ => None,
    }
}

/// Extract the engine + decoded terms from a single URL, or `None` when it isn't
/// a recognized `http(s)` search-result URL (or carries no non-empty terms).
pub fn extract_query(raw: &str) -> Option<SearchQuery> {
    let u = Url::parse(raw).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    let host = u.host_str()?.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let (engine, param) = engine_for(host, u.path())?;
    // query_pairs() percent-decodes and turns '+' into space.
    let terms = u
        .query_pairs()
        .find(|(k, _)| k == param)
        .map(|(_, v)| v.trim().to_string())
        .filter(|s| !s.is_empty())?;
    Some(SearchQuery { engine, terms })
}

/// Extract and aggregate search terms across `urls`, returning the `top` most
/// frequent (count desc, then terms asc, then engine asc — a total order, so the
/// result is deterministic). Terms are grouped case-insensitively after trimming;
/// the first-seen display casing is kept.
pub fn top_searches(urls: &[String], top: usize) -> Vec<SearchCount> {
    use std::collections::HashMap;
    // key = (engine, lowercased terms) → (display terms, count)
    let mut agg: HashMap<(&'static str, String), (String, u32)> = HashMap::new();
    for raw in urls {
        if let Some(q) = extract_query(raw) {
            let key = (q.engine, q.terms.to_lowercase());
            let e = agg.entry(key).or_insert((q.terms.clone(), 0));
            e.1 += 1;
        }
    }
    let mut out: Vec<SearchCount> = agg
        .into_iter()
        .map(|((engine, _), (terms, count))| SearchCount {
            engine: engine.to_string(),
            terms,
            count,
        })
        .collect();
    out.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.terms.cmp(&b.terms))
            .then_with(|| a.engine.cmp(&b.engine))
    });
    out.truncate(top);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_known_engines() {
        assert_eq!(
            extract_query("https://www.google.com/search?q=rust+lang&hl=en"),
            Some(SearchQuery {
                engine: "Google",
                terms: "rust lang".into()
            })
        );
        assert_eq!(
            extract_query("https://google.co.uk/search?q=tea"),
            Some(SearchQuery {
                engine: "Google",
                terms: "tea".into()
            })
        );
        assert_eq!(
            extract_query("https://duckduckgo.com/?q=privacy%20tools"),
            Some(SearchQuery {
                engine: "DuckDuckGo",
                terms: "privacy tools".into()
            })
        );
        assert_eq!(
            extract_query("https://www.youtube.com/results?search_query=lofi"),
            Some(SearchQuery {
                engine: "YouTube",
                terms: "lofi".into()
            })
        );
        assert_eq!(
            extract_query("https://www.bing.com/search?q=weather"),
            Some(SearchQuery {
                engine: "Bing",
                terms: "weather".into()
            })
        );
    }

    #[test]
    fn ignores_non_search_urls() {
        // Right host, wrong path (maps query is not a web search).
        assert_eq!(extract_query("https://www.google.com/maps?q=cafe"), None);
        // Search path but empty terms.
        assert_eq!(extract_query("https://www.google.com/search?q="), None);
        // Unrelated site that happens to use ?q=.
        assert_eq!(extract_query("https://example.com/?q=hello"), None);
        // Non-http scheme.
        assert_eq!(extract_query("chrome://settings/?q=x"), None);
        // Plain page, no query.
        assert_eq!(extract_query("https://news.ycombinator.com/"), None);
    }

    #[test]
    fn aggregates_and_ranks_deterministically() {
        let urls = vec![
            "https://www.google.com/search?q=rust".to_string(),
            "https://www.google.com/search?q=Rust".to_string(), // same, case-insensitive
            "https://duckduckgo.com/?q=rust".to_string(),       // different engine
            "https://www.google.com/search?q=apples".to_string(),
            "https://news.ycombinator.com/".to_string(), // ignored
        ];
        let top = top_searches(&urls, 10);
        assert_eq!(top.len(), 3);
        // "rust" on Google appears twice → ranks first.
        assert_eq!(top[0].engine, "Google");
        assert_eq!(top[0].terms, "rust");
        assert_eq!(top[0].count, 2);
        // The remaining two (count 1) are ordered by terms asc: "apples" < "rust".
        assert_eq!(top[1].terms, "apples");
        assert_eq!(top[2].terms, "rust");
        assert_eq!(top[2].engine, "DuckDuckGo");
    }

    #[test]
    fn top_limit_truncates() {
        let urls = vec![
            "https://www.google.com/search?q=a".to_string(),
            "https://www.google.com/search?q=b".to_string(),
            "https://www.google.com/search?q=c".to_string(),
        ];
        assert_eq!(top_searches(&urls, 2).len(), 2);
    }
}
