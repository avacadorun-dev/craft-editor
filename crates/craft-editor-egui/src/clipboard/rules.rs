//! Enabled source paste-rule components; source/host policy stays explicit.
use craft_editor_model::is_allowed_link_uri;
use linkify::{LinkFinder, LinkKind};
use std::{collections::HashSet, ops::Range, sync::OnceLock};

/// A recognized source-compatible link with an explicit UTF-8 byte range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSpan {
    /// Exact byte boundaries in the supplied string; caller converts to scalar positions.
    pub bytes: Range<usize>,
    /// Canonical href including original http/mailto default protocol.
    pub href: String,
}
/// Find eligible URL/email spans without opening a URL, network request or filesystem path.
///
/// This is a component checkpoint. The external-paste transform and source corpus qualification
/// determine editor parity; exposing a scanner alone is not authoring/paste acceptance.
pub fn link_spans(text: &str) -> Vec<LinkSpan> {
    let mut finder = LinkFinder::new();
    finder.url_must_have_scheme(false);
    let mut result = Vec::new();
    for found in finder.links(text) {
        let value = found.as_str();
        if let Some((scheme, suffix)) = value.split_once(':')
            && valid_scheme(scheme)
            && !matches!(
                scheme.to_ascii_lowercase().as_str(),
                "http" | "https" | "ftp" | "ftps" | "mailto" | "file"
            )
        {
            // The source scanner recognizes a bare URL/email following an unregistered prefix,
            // rather than admitting an arbitrary new protocol into its canonical mark.
            let offset = found.start() + scheme.len() + 1;
            let prefix = suffix.len() - suffix.trim_start_matches('/').len();
            for nested in finder.links(&suffix[prefix..]) {
                if let Some(href) = href(nested.as_str(), nested.kind()) {
                    result.push(LinkSpan {
                        bytes: offset + prefix + nested.start()..offset + prefix + nested.end(),
                        href,
                    });
                }
            }
        } else if let Some(href) = href(value, found.kind()) {
            result.push(LinkSpan {
                bytes: found.start()..found.end(),
                href,
            });
        }
    }
    result
}
fn valid_scheme(value: &str) -> bool {
    value
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphabetic)
        && value
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || b"+.-".contains(&ch))
}
fn known_tld(host: &str) -> bool {
    static TLDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    let tlds = TLDS.get_or_init(|| {
        include_str!("../../third_party/linkifyjs/tlds-4.3.3.txt")
            .lines()
            .collect()
    });
    host.rsplit_once('.')
        .is_some_and(|(_, last)| tlds.contains(last.to_lowercase().as_str()))
}
fn href(value: &str, kind: &LinkKind) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    let explicit = ["http://", "https://", "ftp://", "ftps://"]
        .iter()
        .find(|prefix| lower.starts_with(**prefix));
    let href = if let Some(prefix) = explicit {
        // The installed source scanner does not recognize bracketed IPv6 authority literals.
        if value[prefix.len()..].starts_with('[') {
            return None;
        }
        value.to_owned()
    } else if lower.starts_with("mailto:") {
        let mail = &value[7..];
        if !mail
            .rsplit_once('@')
            .is_some_and(|(_, domain)| known_tld(domain))
        {
            return None;
        }
        value.to_owned()
    } else if kind == &LinkKind::Email {
        if !value
            .rsplit_once('@')
            .is_some_and(|(_, domain)| known_tld(domain))
        {
            return None;
        }
        format!("mailto:{value}")
    } else {
        if lower.starts_with("file:") {
            return None;
        }
        let authority = value.split(['/', '?', '#']).next()?;
        let host = authority.rsplit('@').next()?.split(':').next()?;
        if !known_tld(host) {
            return None;
        }
        format!("http://{value}")
    };
    is_allowed_link_uri(&href).then_some(href)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn urls_emails_default_protocol_and_shared_uri_policy_match_enabled_source_basics() {
        for (text, expected) in [
            ("example.com", Some("http://example.com")),
            ("a@example.com", Some("mailto:a@example.com")),
            ("https://example.com", Some("https://example.com")),
            ("ftp://example.com/a", Some("ftp://example.com/a")),
            ("example.invalid", None),
            ("a@example.invalid", None),
            ("127.0.0.1", None),
            ("localhost", None),
            ("javascript:bad", None),
            ("file:///etc/private", None),
        ] {
            let found = link_spans(text);
            assert_eq!(found.len(), usize::from(expected.is_some()), "{text}");
            if let Some(expected) = expected {
                assert_eq!(found[0].href, expected, "{text}");
                assert_eq!(found[0].bytes, 0..text.len());
            }
        }
    }
    #[test]
    fn link_boundaries_are_utf8_bytes_and_surrounding_unicode_stays_outside_href() {
        let text = "先😀 https://example.com/后 — a@example.com";
        let links = link_spans(text);
        assert_eq!(links.len(), 2);
        assert_eq!(&text[links[0].bytes.clone()], "https://example.com/后");
        assert_eq!(&text[links[1].bytes.clone()], "a@example.com");
        assert_eq!(text[..links[0].bytes.start].chars().count(), 3);
    }
}
