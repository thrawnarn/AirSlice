//! Model descriptions arrive as HTML. They are shown as plain text blocks,
//! never as markup: the app shares Home Assistant's origin, so remote HTML
//! must not reach the page.

use crate::models::TextBlock;

/// At most this many blocks and characters of a description.
const MAX_BLOCKS: usize = 200;
const MAX_CHARS: usize = 20_000;

/// Paragraphs, headings and list items from an HTML fragment. Inline markup
/// is dropped (its text kept), as are images, scripts and styles.
pub fn to_blocks(html: &str) -> Vec<TextBlock> {
    let mut out: Vec<TextBlock> = Vec::new();
    let mut text = String::new();
    let mut kind = "p";
    let mut skip_until: Option<&'static str> = None;
    let mut total = 0;
    let mut rest = html;

    let flush = |text: &mut String, kind: &str, out: &mut Vec<TextBlock>, total: &mut usize| {
        let t = collapse(&decode(text));
        text.clear();
        if !t.is_empty() && out.len() < MAX_BLOCKS && *total < MAX_CHARS {
            *total += t.len();
            out.push(TextBlock { kind: kind.to_string(), text: t });
        }
    };

    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            if skip_until.is_none() {
                text.push_str(rest);
            }
            break;
        };
        if skip_until.is_none() {
            text.push_str(&rest[..lt]);
        }
        rest = &rest[lt..];
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        let closing = tag.starts_with('/');
        let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        if let Some(end) = skip_until {
            if closing && name == end {
                skip_until = None;
            }
            continue;
        }
        match name.as_str() {
            "script" | "style" | "noscript" if !closing => skip_until = Some(match name.as_str() {
                "script" => "script",
                "style" => "style",
                _ => "noscript",
            }),
            "p" | "div" | "figure" | "blockquote" | "tr" | "ul" | "ol" | "table" | "section" => {
                flush(&mut text, kind, &mut out, &mut total);
                kind = "p";
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                flush(&mut text, kind, &mut out, &mut total);
                kind = if closing { "p" } else { "h" };
            }
            "li" => {
                flush(&mut text, kind, &mut out, &mut total);
                kind = if closing { "p" } else { "li" };
            }
            "br" => text.push('\n'),
            "td" | "th" if closing => text.push(' '),
            _ => {}
        }
    }
    flush(&mut text, kind, &mut out, &mut total);
    out
}

/// Whitespace runs become one space, but a `<br>` line break is kept.
fn collapse(s: &str) -> String {
    s.split('\n').map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

/// The entities descriptions use, named and numeric.
pub fn decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..semi];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "ndash" => Some('–'),
            "mdash" => Some('—'),
            "hellip" => Some('…'),
            _ => ent
                .strip_prefix("#x")
                .or_else(|| ent.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(kind: &str, text: &str) -> TextBlock {
        TextBlock { kind: kind.into(), text: text.into() }
    }

    #[test]
    fn keeps_text_drops_markup() {
        let html = r#"<figure class="image">&#34;</figure><p>This <strong>box</strong> needs&nbsp;no glue.</p><p><strong>Sizes:</strong></p><ul><li>9.5×5 cm</li><li>8&amp;9 cm</li></ul><h2>Tips</h2><p>Line one<br>line two</p><script>alert(1)</script><img src=x onerror=alert(1)><p>It&#39;s fine</p>"#;
        assert_eq!(
            to_blocks(html),
            vec![
                b("p", "\""),
                b("p", "This box needs no glue."),
                b("p", "Sizes:"),
                b("li", "9.5×5 cm"),
                b("li", "8&9 cm"),
                b("h", "Tips"),
                b("p", "Line one\nline two"),
                b("p", "It's fine"),
            ]
        );
    }

    #[test]
    fn markup_in_text_stays_text() {
        // Decoded entities are text for the page, not tags.
        assert_eq!(to_blocks("<p>&lt;script&gt;x&lt;/script&gt;</p>"), vec![b("p", "<script>x</script>")]);
        assert_eq!(decode("a & b &bogus; &#x1F996;"), "a & b &bogus; 🦖");
    }
}
