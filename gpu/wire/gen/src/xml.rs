//! Just enough XML for Khronos's registries: elements, attributes, text, the five entities, and comments and
//! declarations skipped.

/// An element: its name, attributes, and children in order.
#[derive(Debug, Default)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

#[derive(Debug)]
pub enum Node {
    Element(Element),
    Text(String),
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter_map(move |n| match n {
            Node::Element(e) if e.name == name => Some(e),
            _ => None,
        })
    }

    pub fn child<'a>(&'a self, name: &'a str) -> Option<&'a Element> {
        self.children_named(name).next()
    }

    /// All the text inside, children's included, comments' not.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for n in &self.children {
            match n {
                Node::Text(t) => out.push_str(t),
                Node::Element(e) if e.name != "comment" => out.push_str(&e.text()),
                Node::Element(_) => {}
            }
        }
        out
    }

    /// Every element inside, depth first.
    pub fn descendants(&self) -> Vec<&Element> {
        let mut out = Vec::new();
        for n in &self.children {
            if let Node::Element(e) = n {
                out.push(e);
                out.extend(e.descendants());
            }
        }
        out
    }
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// The document's root element.
pub fn parse(text: &str) -> Result<Element, String> {
    let mut stack: Vec<Element> = vec![Element::default()];
    let mut rest = text;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            push_text(&mut stack, rest);
            break;
        };
        push_text(&mut stack, &rest[..lt]);
        rest = &rest[lt..];
        if let Some(after) = rest.strip_prefix("<!--") {
            let end = after.find("-->").ok_or("a comment that doesn't end")?;
            rest = &after[end + 3..];
        } else if rest.starts_with("<?") || rest.starts_with("<!") {
            let end = rest.find('>').ok_or("a declaration that doesn't end")?;
            rest = &rest[end + 1..];
        } else if let Some(after) = rest.strip_prefix("</") {
            let end = after.find('>').ok_or("a closing tag that doesn't end")?;
            let e = stack.pop().ok_or("a closing tag with nothing open")?;
            if e.name != after[..end].trim() {
                return Err(format!("</{}> closes <{}>", &after[..end], e.name));
            }
            stack.last_mut().ok_or("nothing open")?.children.push(Node::Element(e));
            rest = &after[end + 1..];
        } else {
            let end = tag_end(rest).ok_or("a tag that doesn't end")?;
            let inner = &rest[1..end];
            let empty = inner.ends_with('/');
            let inner = inner.trim_end_matches('/');
            let (name, attrs) = inner.split_once(char::is_whitespace).unwrap_or((inner, ""));
            let e = Element { name: name.to_string(), attrs: attributes(attrs), children: Vec::new() };
            if empty {
                stack.last_mut().ok_or("nothing open")?.children.push(Node::Element(e));
            } else {
                stack.push(e);
            }
            rest = &rest[end + 1..];
        }
    }
    let doc = stack.pop().ok_or("no document")?;
    if !stack.is_empty() {
        return Err("elements left open at the end".into());
    }
    doc.children
        .into_iter()
        .find_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
        .ok_or_else(|| "no root element".into())
}

/// Where a tag's `>` is, past any in its quoted attribute values.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

fn attributes(mut s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    while let Some(eq) = s.find('=') {
        let name = s[..eq].trim().to_string();
        let after = s[eq + 1..].trim_start();
        let Some(q) = after.chars().next() else { break };
        let Some(close) = after[1..].find(q) else { break };
        out.push((name, unescape(&after[1..1 + close])));
        s = &after[close + 2..];
    }
    out
}

fn push_text(stack: &mut [Element], text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(top) = stack.last_mut() {
        top.children.push(Node::Text(unescape(text)));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn elements_attributes_and_text() {
        let doc = super::parse("<?xml version=\"1.0\"?><a x=\"1 &amp; 2\"><!-- c --><b>hi <c/></b>tail</a>").unwrap();
        assert_eq!(doc.name, "a");
        assert_eq!(doc.attr("x"), Some("1 & 2"));
        assert_eq!(doc.child("b").unwrap().text(), "hi ");
        assert_eq!(doc.text(), "hi tail");
    }
}
