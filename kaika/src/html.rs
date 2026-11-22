//! Just enough HTML and URL handling to read a search results page: anchors, percent-encoding.

#[derive(Debug, Clone, PartialEq)]
pub struct Anchor {
    pub href: String,
    pub class: String,
    pub text: String,
}

/// The value of attribute `name` in an opening tag's text: double-quoted, single-quoted or bare, with any spacing
/// around the `=`, and the name in any case. A longer name that merely ends in it (`data-href`) is another attribute.
fn attr(tag: &str, name: &str) -> String {
    let mut rest = tag;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '/');
        let end = rest
            .find(|c: char| c.is_ascii_whitespace() || c == '=' || c == '/')
            .unwrap_or(rest.len());
        let (key, after) = rest.split_at(end);
        if key.is_empty() {
            return String::new();
        }
        let after = after.trim_start();
        let Some(value) = after.strip_prefix('=') else {
            rest = after; // a bare attribute with no value
            continue;
        };
        let value = value.trim_start();
        let (text, remainder) = match value.chars().next() {
            Some(q @ ('"' | '\'')) => value[1..].split_once(q).unwrap_or((&value[1..], "")),
            _ => value.split_at(
                value
                    .find(|c: char| c.is_ascii_whitespace())
                    .unwrap_or(value.len()),
            ),
        };
        if key.eq_ignore_ascii_case(name) {
            return text.to_string();
        }
        rest = remainder;
    }
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn anchors(html: &str) -> Vec<Anchor> {
    let lower = html.to_ascii_lowercase(); // ASCII lowercasing keeps byte offsets identical
    let mut out = vec![];
    let mut i = 0;
    while let Some(p) = lower[i..].find("<a") {
        let start = i + p;
        // `<a` must be followed by whitespace: `<abbr>` and `<article>` are other tags.
        if !lower
            .as_bytes()
            .get(start + 2)
            .is_some_and(u8::is_ascii_whitespace)
        {
            i = start + 2;
            continue;
        }
        let Some(gt) = html[start..].find('>') else {
            break;
        };
        let tag = &html[start + 3..start + gt];
        let body = &html[start + gt + 1..];
        let end = lower[start + gt + 1..].find("</a>").unwrap_or(body.len());
        out.push(Anchor {
            href: attr(tag, "href"),
            class: attr(tag, "class"),
            text: strip_tags(&body[..end]),
        });
        i = start + gt + 1 + end;
    }
    out
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |k: usize| b.get(k).and_then(|c| (*c as char).to_digit(16));
        match b[i] {
            b'%' if hex(i + 1).is_some() && hex(i + 2).is_some() => {
                out.push((hex(i + 1).unwrap() * 16 + hex(i + 2).unwrap()) as u8);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The host of a URL without a leading `www.`.
pub fn host(url: &str) -> String {
    let rest = url.split_once("://").map(|x| x.1).unwrap_or(url);
    let h = rest.split(['/', '?', '#']).next().unwrap_or("");
    let h = h
        .rsplit('@')
        .next()
        .unwrap_or(h)
        .split(':')
        .next()
        .unwrap_or(h);
    h.strip_prefix("www.").unwrap_or(h).to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_carry_href_class_and_clean_text() {
        let a = anchors(
            r#"<p>x</p><A HREF="/a" class="result__a big">Rect<b>angle</b> &amp; co</A><a href='/b'>B</a>"#,
        );
        assert_eq!(a.len(), 2);
        assert_eq!(
            (a[0].href.as_str(), a[0].class.as_str(), a[0].text.as_str()),
            ("/a", "result__a big", "Rectangle & co")
        );
        assert_eq!(a[1].href, "/b");
    }

    #[test]
    fn real_markup_is_read_whatever_the_spacing_quoting_or_case() {
        let page = "<a\nhref=\"/a.dmg\"\n   class=x>One</a><a\thref = '/b.dmg'>Two</a><a href=/c.dmg>Three</a><A\nHREF=\"/d.dmg\">Four</A><abbr href=\"/no\">x</abbr>";
        let hrefs: Vec<String> = anchors(page).into_iter().map(|a| a.href).collect();
        assert_eq!(hrefs, ["/a.dmg", "/b.dmg", "/c.dmg", "/d.dmg"]);
    }

    #[test]
    fn a_data_href_is_not_an_href() {
        assert_eq!(
            anchors(r#"<a data-href="/no" href="/yes">x</a>"#)[0].href,
            "/yes"
        );
    }

    #[test]
    fn broken_html_does_not_panic() {
        assert!(anchors("<a href=\"x").is_empty());
        assert_eq!(anchors("<a href=\"x\">unclosed").len(), 1);
        assert!(anchors("").is_empty());
    }

    #[test]
    fn percent_coding_round_trips() {
        assert_eq!(
            percent_decode("https%3A%2F%2Fghost.example%2Fa%20b+c"),
            "https://ghost.example/a b c"
        );
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%é%4"), "%é%4");
        assert_eq!(url_encode("ghost app"), "ghost+app");
        assert_eq!(url_encode("a/b&c"), "a%2Fb%26c");
    }

    #[test]
    fn hosts_drop_www_port_and_credentials() {
        assert_eq!(
            host("https://www.Ghost.example:8080/x?y#z"),
            "ghost.example"
        );
        assert_eq!(host("https://user@a.example/"), "a.example");
        assert_eq!(host("github.com/o/r"), "github.com");
    }
}
