//! Byte-preserving reader/editor for Valve KeyValues text files (`.vdf`, `.acf`).
//!
//! Steam's config files are edited *in place*: we parse the file into a tree that remembers
//! the byte span of every token, and a change rewrites only the span of the value being changed.
//! Everything else (whitespace, ordering, comments, unknown keys) stays byte-for-byte identical.
//! Key lookups are case-insensitive, because Steam itself is inconsistent ("apps" vs "Apps").

use anyhow::{bail, Context, Result};
use std::ops::Range;

#[derive(Debug, Clone)]
pub struct Node {
    pub key: String,
    pub value: Value,
}

#[derive(Debug, Clone)]
pub enum Value {
    /// `span` covers the whole token, including surrounding quotes if any.
    Str { text: String, span: Range<usize> },
    /// `close` is the byte offset of the closing `}`.
    Obj { children: Vec<Node>, close: usize },
}

impl Node {
    pub fn get(&self, key: &str) -> Option<&Node> {
        match &self.value {
            Value::Obj { children, .. } => {
                children.iter().find(|c| c.key.eq_ignore_ascii_case(key))
            }
            Value::Str { .. } => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match &self.value {
            Value::Str { text, .. } => Some(text),
            Value::Obj { .. } => None,
        }
    }

    pub fn children(&self) -> &[Node] {
        match &self.value {
            Value::Obj { children, .. } => children,
            Value::Str { .. } => &[],
        }
    }
}

/// A parsed document: the original text plus the tree of top-level nodes.
#[derive(Debug, Clone)]
pub struct Doc {
    pub text: String,
    pub roots: Vec<Node>,
}

impl Doc {
    pub fn parse(text: &str) -> Result<Doc> {
        let mut p = Parser {
            s: text.as_bytes(),
            i: 0,
        };
        let roots = p.parse_children(true)?;
        Ok(Doc {
            text: text.to_string(),
            roots,
        })
    }

    /// Look up a node by a case-insensitive key path, starting at the top level.
    pub fn lookup(&self, path: &[&str]) -> Option<&Node> {
        let (first, rest) = path.split_first()?;
        let mut node = self
            .roots
            .iter()
            .find(|n| n.key.eq_ignore_ascii_case(first))?;
        for k in rest {
            node = node.get(k)?;
        }
        Some(node)
    }

    pub fn get_str(&self, path: &[&str]) -> Option<&str> {
        self.lookup(path).and_then(|n| n.str())
    }

    /// Set a string value at `path`, creating missing intermediate objects.
    /// Returns the new document text; `self` is not modified. The result is re-parsed
    /// before being returned so a broken file can never be produced.
    pub fn with_str(&self, path: &[&str], value: &str) -> Result<String> {
        let quoted = format!("\"{}\"", escape(value));
        let out = match self.lookup(path) {
            Some(Node {
                value: Value::Str { span, .. },
                ..
            }) => splice(&self.text, span.clone(), &quoted),
            Some(_) => bail!("{} is an object, not a string", path.join("/")),
            None => self.insert_missing(path, &quoted)?,
        };
        Doc::parse(&out).context("edited VDF failed to re-parse (refusing to write)")?;
        Ok(out)
    }

    fn insert_missing(&self, path: &[&str], quoted_value: &str) -> Result<String> {
        // Find the deepest existing object along the path.
        let (first, _) = path.split_first().context("empty path")?;
        let root = self
            .roots
            .iter()
            .find(|n| n.key.eq_ignore_ascii_case(first))
            .with_context(|| format!("top-level key {first} not found"))?;
        let mut node = root;
        let mut depth = 1;
        while depth < path.len() {
            match node.get(path[depth]) {
                Some(child) if matches!(child.value, Value::Obj { .. }) => {
                    node = child;
                    depth += 1;
                }
                Some(_) if depth == path.len() - 1 => unreachable!("handled by caller"),
                Some(_) => bail!(
                    "{} is a string, expected an object",
                    path[..=depth].join("/")
                ),
                None => break,
            }
        }
        let close = match &node.value {
            Value::Obj { close, .. } => *close,
            Value::Str { .. } => bail!("not an object"),
        };
        let indent = line_indent(&self.text, close);
        let missing = &path[depth..];
        let mut block = String::new();
        for (i, key) in missing.iter().enumerate() {
            let pad = format!("{indent}\t{}", "\t".repeat(i));
            if i == missing.len() - 1 {
                block.push_str(&format!("{pad}\"{}\"\t\t{quoted_value}\n", escape(key)));
            } else {
                block.push_str(&format!("{pad}\"{}\"\n{pad}{{\n", escape(key)));
            }
        }
        for i in (0..missing.len().saturating_sub(1)).rev() {
            let pad = format!("{indent}\t{}", "\t".repeat(i));
            block.push_str(&format!("{pad}}}\n"));
        }
        // Insert at the start of the closing brace's line, so its indentation is kept.
        let line_start = self.text[..close].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let at = if self.text[line_start..close].trim().is_empty() {
            line_start
        } else {
            close
        };
        let block = if at == close {
            format!("\n{block}{indent}")
        } else {
            block
        };
        Ok(splice(&self.text, at..at, &block))
    }
}

fn splice(text: &str, range: Range<usize>, with: &str) -> String {
    let mut out = String::with_capacity(text.len() + with.len());
    out.push_str(&text[..range.start]);
    out.push_str(with);
    out.push_str(&text[range.end..]);
    out
}

fn line_indent(text: &str, pos: usize) -> String {
    let line_start = text[..pos].rfind('\n').map(|p| p + 1).unwrap_or(0);
    text[line_start..pos]
        .chars()
        .take_while(|c| *c == '\t' || *c == ' ')
        .collect()
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        loop {
            while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            if self.s[self.i..].starts_with(b"//") {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
                continue;
            }
            // Skip a UTF-8 BOM if present.
            if self.s[self.i..].starts_with(&[0xEF, 0xBB, 0xBF]) {
                self.i += 3;
                continue;
            }
            break;
        }
    }

    /// Skips an optional platform conditional like `[$WIN32]`.
    fn skip_conditional(&mut self) {
        self.skip_ws_inline();
        if self.i < self.s.len() && self.s[self.i] == b'[' {
            while self.i < self.s.len() && self.s[self.i] != b']' {
                self.i += 1;
            }
            self.i += 1;
        }
    }

    fn skip_ws_inline(&mut self) {
        while self.i < self.s.len() && (self.s[self.i] == b' ' || self.s[self.i] == b'\t') {
            self.i += 1;
        }
    }

    fn parse_children(&mut self, top: bool) -> Result<Vec<Node>> {
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            if self.i >= self.s.len() {
                if top {
                    return Ok(out);
                }
                bail!("unexpected end of file: missing '}}'");
            }
            if self.s[self.i] == b'}' {
                if top {
                    bail!("unexpected '}}' at byte {}", self.i);
                }
                return Ok(out);
            }
            let (key, _) = self.token().context("expected a key")?;
            self.skip_ws();
            if self.i >= self.s.len() {
                bail!("key {key:?} has no value");
            }
            let value = if self.s[self.i] == b'{' {
                self.i += 1;
                let children = self.parse_children(false)?;
                let close = self.i;
                self.i += 1; // consume '}'
                Value::Obj { children, close }
            } else {
                let (text, span) = self
                    .token()
                    .with_context(|| format!("expected a value for {key:?}"))?;
                Value::Str { text, span }
            };
            self.skip_conditional();
            out.push(Node { key, value });
        }
    }

    fn token(&mut self) -> Result<(String, Range<usize>)> {
        let start = self.i;
        if self.s[self.i] == b'"' {
            self.i += 1;
            let mut buf = Vec::new();
            while self.i < self.s.len() {
                match self.s[self.i] {
                    b'"' => {
                        self.i += 1;
                        let text = String::from_utf8(buf).context("invalid UTF-8 in string")?;
                        return Ok((text, start..self.i));
                    }
                    b'\\' if self.i + 1 < self.s.len() => {
                        match self.s[self.i + 1] {
                            b'n' => buf.push(b'\n'),
                            b't' => buf.push(b'\t'),
                            b'\\' => buf.push(b'\\'),
                            b'"' => buf.push(b'"'),
                            other => buf.extend_from_slice(&[b'\\', other]),
                        }
                        self.i += 2;
                    }
                    c => {
                        buf.push(c);
                        self.i += 1;
                    }
                }
            }
            bail!("unterminated string starting at byte {start}");
        }
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c.is_ascii_whitespace() || c == b'{' || c == b'}' || c == b'"' {
                break;
            }
            self.i += 1;
        }
        if self.i == start {
            bail!("unexpected character at byte {start}");
        }
        let text = String::from_utf8(self.s[start..self.i].to_vec())?;
        Ok((text, start..self.i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCALCONFIG: &str = "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"apps\"\n\t\t\t\t{\n\t\t\t\t\t\"1240440\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LastPlayed\"\t\t\"1763900000\"\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"PROTON_LOG=1 %command% -name \\\"x y\\\"\"\n\t\t\t\t\t}\n\t\t\t\t\t\"570\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LastPlayed\"\t\t\"1\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n";

    const APPS: [&str; 5] = ["UserLocalConfigStore", "Software", "Valve", "Steam", "Apps"];

    fn path<'a>(app: &'a str, key: &'a str) -> Vec<&'a str> {
        let mut p = APPS.to_vec();
        p.push(app);
        p.push(key);
        p
    }

    #[test]
    fn reads_escaped_values_case_insensitively() {
        let d = Doc::parse(LOCALCONFIG).unwrap();
        assert_eq!(
            d.get_str(&path("1240440", "launchoptions")),
            Some("PROTON_LOG=1 %command% -name \"x y\"")
        );
    }

    #[test]
    fn replacing_a_value_changes_only_that_span() {
        let d = Doc::parse(LOCALCONFIG).unwrap();
        let out = d
            .with_str(&path("1240440", "LaunchOptions"), "%command%")
            .unwrap();
        let expected = LOCALCONFIG.replace(
            "\"PROTON_LOG=1 %command% -name \\\"x y\\\"\"",
            "\"%command%\"",
        );
        assert_eq!(out, expected);
    }

    #[test]
    fn setting_back_the_original_value_is_byte_identical() {
        let d = Doc::parse(LOCALCONFIG).unwrap();
        let p = path("1240440", "LaunchOptions");
        let changed = Doc::parse(&d.with_str(&p, "something else").unwrap()).unwrap();
        let back = changed.with_str(&p, d.get_str(&p).unwrap()).unwrap();
        assert_eq!(back, LOCALCONFIG);
    }

    #[test]
    fn inserts_missing_key_with_matching_indentation() {
        let d = Doc::parse(LOCALCONFIG).unwrap();
        let out = d.with_str(&path("570", "LaunchOptions"), "-novid").unwrap();
        assert!(out.contains("\t\t\t\t\t\t\"LastPlayed\"\t\t\"1\"\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"-novid\"\n\t\t\t\t\t}"), "{out}");
        assert_eq!(
            Doc::parse(&out)
                .unwrap()
                .get_str(&path("570", "LaunchOptions")),
            Some("-novid")
        );
    }

    #[test]
    fn inserts_missing_app_block() {
        let d = Doc::parse(LOCALCONFIG).unwrap();
        let out = d
            .with_str(&path("999", "LaunchOptions"), "%command%")
            .unwrap();
        let d2 = Doc::parse(&out).unwrap();
        assert_eq!(d2.get_str(&path("999", "LaunchOptions")), Some("%command%"));
        assert_eq!(
            d2.get_str(&path("1240440", "LastPlayed")),
            Some("1763900000")
        );
    }

    #[test]
    fn rejects_broken_files() {
        assert!(Doc::parse("\"a\"\n{\n\t\"b\"\t\"c\"\n").is_err());
        assert!(Doc::parse("\"a\"\t\"unterminated").is_err());
    }

    #[test]
    fn tolerates_comments_conditionals_and_bom() {
        let src = "\u{feff}// comment\n\"a\"\n{\n\t\"b\"\t\"1\" [$WIN32]\n\t\"c\"\t\"2\"\n}\n";
        let d = Doc::parse(src).unwrap();
        assert_eq!(d.get_str(&["a", "c"]), Some("2"));
    }
}
