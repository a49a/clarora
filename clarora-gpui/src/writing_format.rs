//! Convert visible HTML clipboard content to a small, editable Markdown subset.
use html5ever::{parse_document, tendril::TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

pub const MAX_HTML_BYTES: usize = 512 * 1024;

pub fn from_html(html: &str) -> Option<String> {
    if html.len() > MAX_HTML_BYTES {
        return None;
    }
    let fragment = html
        .split_once("<!--StartFragment-->")
        .and_then(|(_, body)| body.split_once("<!--EndFragment-->"))
        .map_or(html, |(fragment, _)| fragment);
    let dom = parse_document(RcDom::default(), Default::default()).one(fragment);
    let mut markdown = children(&dom.document, 0);
    // Adjacent containers may each contribute a block boundary.
    while markdown.contains("\n\n\n") {
        markdown = markdown.replace("\n\n\n", "\n\n");
    }
    let markdown = markdown.trim();
    (!markdown.is_empty()).then(|| markdown.to_owned())
}

fn children(node: &Handle, depth: usize) -> String {
    node.children
        .borrow()
        .iter()
        .map(|child| render(child, depth + 1))
        .collect()
}

pub fn escape_plain(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '*' | '_' | '[' | ']' | '`' | '~' | '<' | '>' | '#' | '|'
        ) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

fn collapse_whitespace(text: &str) -> String {
    let mut result = String::new();
    let mut in_space = false;
    for c in text.chars() {
        if c.is_ascii_whitespace() || c == '\u{a0}' {
            if !in_space {
                result.push(' ');
            }
            in_space = true;
        } else {
            result.push(c);
            in_space = false;
        }
    }
    result
}

fn block(body: &str) -> String {
    if body.trim().is_empty() {
        String::new()
    } else {
        format!("\n\n{}\n\n", body.trim())
    }
}

fn styled(body: String, marker: &str) -> String {
    let text = body.trim();
    if text.is_empty() {
        return body;
    }
    let leading = body.len() - body.trim_start().len();
    let trailing = body.trim_end().len();
    format!(
        "{}{marker}{text}{marker}{}",
        &body[..leading],
        &body[trailing..]
    )
}

fn render(node: &Handle, depth: usize) -> String {
    if depth > 64 {
        return String::new();
    }
    let NodeData::Element { name, attrs, .. } = &node.data else {
        return match &node.data {
            NodeData::Text { contents } => escape_plain(&collapse_whitespace(&contents.borrow())),
            _ => String::new(),
        };
    };
    let tag: &str = &name.local;
    let attrs = attrs.borrow();
    let attribute = |key: &str| {
        attrs
            .iter()
            .find(|a| a.name.local.as_ref() == key)
            .map(|a| a.value.as_ref())
    };
    let style = attribute("style").unwrap_or_default();
    let declarations = style
        .split(';')
        .filter_map(|s| s.split_once(':'))
        .map(|(key, val)| (key.trim(), val.trim()));
    if attribute("hidden").is_some()
        || declarations.clone().any(|(key, value)| {
            key.eq_ignore_ascii_case("display") && value.eq_ignore_ascii_case("none")
        })
    {
        return String::new();
    }
    match tag {
        "head" | "script" | "style" | "template" | "noscript" | "img" | "svg" | "iframe"
        | "object" => String::new(),
        "br" => "  \n".into(),
        "ul" | "ol" => {
            let start = attribute("start")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(1)
                .min(999_999_999);
            let list = node.children.borrow().iter().filter(|child| {
                matches!(&child.data, NodeData::Element { name, .. } if name.local.as_ref() == "li")
            }).enumerate().filter_map(|(index, child)| {
                let body = render(child, depth + 1);
                let body = body.trim();
                if body.is_empty() {
                    return None;
                }
                let prefix = if tag == "ol" {
                    format!("{}. ", start.saturating_add(index as u32))
                } else {
                    "- ".to_string()
                };
                let indent = " ".repeat(prefix.len());
                Some(format!("{prefix}{}", body.replace('\n', &format!("\n{indent}"))))
            }).collect::<Vec<_>>().join("\n");
            block(&list)
        }
        _ => {
            let mut body = children(node, depth);
            let bold = matches!(tag, "strong" | "b")
                || declarations.clone().any(|(key, value)| {
                    key.eq_ignore_ascii_case("font-weight")
                        && (value.eq_ignore_ascii_case("bold")
                            || value.parse::<u16>().is_ok_and(|weight| weight >= 600))
                });
            let italic = matches!(tag, "em" | "i")
                || declarations.clone().any(|(key, value)| {
                    key.eq_ignore_ascii_case("font-style")
                        && (value.eq_ignore_ascii_case("italic")
                            || value.eq_ignore_ascii_case("oblique"))
                });
            if italic {
                body = styled(body, "*");
            }
            if bold {
                body = styled(body, "**");
            }
            match tag {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" if !body.trim().is_empty() => {
                    let level = usize::from(tag.as_bytes()[1] - b'0');
                    block(&format!("{} {}", "#".repeat(level), body.trim()))
                }
                "p" | "div" | "section" | "article" | "blockquote" | "tr" | "pre" => block(&body),
                "td" | "th" => format!("{} ", body.trim()),
                _ => body,
            }
        }
    }
}
