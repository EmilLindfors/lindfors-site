//! Is this an address worth sending mail to?
//!
//! Carried over from the Worker unchanged, tests included. The check gates a message
//! leaving the server, not a row in a list: mail to a domain that cannot resolve comes
//! straight back as a bounce, which is the signal that costs a self-hosted sender its
//! delivery reputation. Deliberately not a full RFC 5322 parser: quoted local parts,
//! comments and address literals are all legal and all rejected, because none of them
//! belong in a newsletter signup box and each is a way to smuggle odd bytes downstream.

pub fn is_valid_email(email: &str) -> bool {
    if email.len() > 254 {
        return false;
    }
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    is_valid_local_part(local) && is_valid_domain(domain)
}

fn is_valid_local_part(local: &str) -> bool {
    !local.is_empty()
        && local.len() <= 64
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(
                    c,
                    '.' | '!'
                        | '#'
                        | '$'
                        | '%'
                        | '&'
                        | '\''
                        | '*'
                        | '+'
                        | '-'
                        | '/'
                        | '='
                        | '?'
                        | '^'
                        | '_'
                        | '`'
                        | '{'
                        | '|'
                        | '}'
                        | '~'
                )
        })
}

fn is_valid_domain(domain: &str) -> bool {
    if domain.len() > 253 {
        return false;
    }
    let labels: Vec<&str> = domain.split('.').collect();

    // At least one dot: a bare hostname is unroutable from the public internet.
    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        // An all-digit final label means this is an IP address rather than a name.
        // Punycode TLDs (`xn--p1ai`) carry digits and hyphens, so the test is
        // "not entirely digits" rather than "all letters".
        && labels
            .last()
            .is_some_and(|tld| tld.len() >= 2 && !tld.chars().all(|c| c.is_ascii_digit()))
}

/// Normalise what a form sent: trimmed, lower-cased. Addresses are case-insensitive
/// in practice and the primary key on `subscribers` is exact.
pub fn normalise(email: &str) -> String {
    email.trim().to_lowercase()
}

/// A newsletter slug: the file name under `static/newsletter/`, which is also a URL
/// path segment fetched from the site. Lowercase letters, digits and hyphens only.
pub fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 120
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_addresses_pass() {
        for ok in [
            "a@b.co",
            "first.last@example.com",
            "user+tag@sub.example.org",
            "o'neil@example.ie",
            "x@xn--p1ai.ru",
        ] {
            assert!(is_valid_email(ok), "{ok}");
        }
    }

    #[test]
    fn what_a_signup_box_should_not_accept_is_refused() {
        for bad in [
            "",
            "plain",
            "@example.com",
            "a@",
            "a@localhost",
            "a b@example.com",
            "\"quoted\"@example.com",
            ".dot@example.com",
            "dot.@example.com",
            "a..b@example.com",
            "a@example-.com",
            "a@-example.com",
            "a@example.c",
            "a@1.2.3.4",
            "a@[127.0.0.1]",
        ] {
            assert!(!is_valid_email(bad), "{bad}");
        }
    }

    #[test]
    fn lengths_are_bounded() {
        let local = "a".repeat(64);
        assert!(is_valid_email(&format!("{local}@example.com")));
        assert!(!is_valid_email(&format!("{local}a@example.com")));
        let long = format!("a@{}.com", "b".repeat(250));
        assert!(!is_valid_email(&long));
    }

    #[test]
    fn normalising_folds_case_and_space() {
        assert_eq!(normalise("  Emil@Example.COM \n"), "emil@example.com");
    }

    #[test]
    fn slugs_are_file_names_and_nothing_else() {
        assert!(is_valid_slug("two-defensible-answers"));
        assert!(!is_valid_slug(""));
        assert!(!is_valid_slug("../etc"));
        assert!(!is_valid_slug("Slug"));
        assert!(!is_valid_slug("a b"));
    }
}
