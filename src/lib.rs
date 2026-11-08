//! Syntax frontend. Spans are half-open UTF-8 byte offsets in one source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub span: Span,
    pub kind: Kind,
}
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Name(String),
    Number(String),
    String(String),
    Bool(bool),
    Array(Vec<Node>),
    Group(Box<Node>),
    Unary(String, Box<Node>),
    Binary(String, Box<Node>, Box<Node>),
    Call(Box<Node>, Vec<Node>),
    Index(Box<Node>, Box<Node>),
    Member(Box<Node>, Box<Node>),
    Assign(Box<Node>, Box<Node>),
    Return(Option<Box<Node>>),
    Function {
        name: Box<Node>,
        params: Vec<Node>,
        body: Vec<Node>,
    },
    Object {
        name: Box<Node>,
        body: Vec<Node>,
    },
    Use(Box<Node>),
    Public(Box<Node>),
    Match {
        subject: Box<Node>,
        arms: Vec<Node>,
    },
    Arm {
        pattern: Box<Node>,
        value: Box<Node>,
    },
    Wildcard,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub span: Span,
    pub message: String,
}
/// One-based line, Unicode scalar column. Editors using UTF-16 should convert byte spans.
pub fn position(source: &str, offset: usize) -> (usize, usize) {
    let prefix = &source[..offset];
    (
        prefix.bytes().filter(|b| *b == b'\n').count() + 1,
        prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1,
    )
}
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub text: String,
    pub span: Span,
    pub value: Option<String>,
}
fn error(code: &'static str, span: Span, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        code,
        span,
        message: message.into(),
    }
}
pub fn lex(source: &str) -> Result<Vec<Token>, Diagnostic> {
    let b = source.as_bytes();
    let mut i = 0;
    let mut out: Vec<Token> = Vec::new();
    let mut delimiters = Vec::new();
    let mut match_delimiters = Vec::new();
    while i < b.len() {
        let start = i;
        let mut value = None;
        match b[i] {
            b' ' | b'\t' => {
                i += 1;
                continue;
            }
            b'#' => {
                while i < b.len() && !matches!(b[i], b'\r' | b'\n') {
                    i += 1;
                }
                continue;
            }
            b'\r' | b'\n' => {
                i += 1;
                if b[start] == b'\r' && b.get(i) == Some(&b'\n') {
                    i += 1;
                }
                if (!delimiters.is_empty() && match_delimiters.last() != Some(&delimiters.len()))
                    || out.last().is_some_and(|t| t.text == "->")
                {
                    continue;
                }
                out.push(Token {
                    text: "\n".into(),
                    span: Span { start, end: i },
                    value: None,
                });
                continue;
            }
            b'"' => {
                i += 1;
                let mut decoded = String::new();
                while i < b.len() && b[i] != b'"' {
                    if matches!(b[i], b'\r' | b'\n') {
                        return Err(error(
                            "E_STRING",
                            Span { start, end: i },
                            "raw newline in string",
                        ));
                    }
                    if b[i] == b'\\' {
                        let esc = i;
                        i += 1;
                        decoded.push(match b.get(i) {
                            Some(b'n') => '\n',
                            Some(b'r') => '\r',
                            Some(b't') => '\t',
                            Some(b'\\') => '\\',
                            Some(b'"') => '"',
                            _ => {
                                return Err(error(
                                    "E_ESCAPE",
                                    Span {
                                        start: esc,
                                        end: (i + 1).min(b.len()),
                                    },
                                    "unknown or incomplete escape",
                                ))
                            }
                        });
                        i += 1;
                    } else {
                        let ch = source[i..].chars().next().unwrap();
                        decoded.push(ch);
                        i += ch.len_utf8();
                    }
                }
                if i == b.len() {
                    return Err(error(
                        "E_STRING",
                        Span { start, end: i },
                        "unterminated string",
                    ));
                }
                i += 1;
                value = Some(decoded);
            }
            b'0'..=b'9' => {
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                if b.get(i) == Some(&b'.') && b.get(i + 1).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                i += 1;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
            }
            b'(' | b'[' => {
                delimiters.push(b[i]);
                i += 1;
            }
            b')' | b']' => {
                let expected = if b[i] == b')' { b'(' } else { b'[' };
                if delimiters.pop() != Some(expected) {
                    return Err(error(
                        "E_SYNTAX",
                        Span { start, end: i + 1 },
                        "unmatched delimiter",
                    ));
                }
                i += 1;
            }
            b'+' | b'-' | b'*' | b'/' | b'%' | b'^' | b'=' | b'!' | b'<' | b'>' | b'.' | b','
            | b':' => {
                i += 1;
                if (matches!(b[start], b'=' | b'!' | b'<' | b'>') && b.get(i) == Some(&b'='))
                    || (b[start] == b'-' && b.get(i) == Some(&b'>'))
                {
                    i += 1;
                }
                if b[start] == b'!' && i == start + 1 {
                    return Err(error("E_SYNTAX", Span { start, end: i }, "use not or !="));
                }
            }
            _ => {
                let end = i + source[i..].chars().next().unwrap().len_utf8();
                return Err(error(
                    "E_SYNTAX",
                    Span { start, end },
                    "unexpected character",
                ));
            }
        }
        if &source[start..i] == "match" {
            match_delimiters.push(delimiters.len());
        }
        if &source[start..i] == "end" && match_delimiters.last() == Some(&delimiters.len()) {
            match_delimiters.pop();
        }
        out.push(Token {
            text: source[start..i].into(),
            span: Span { start, end: i },
            value,
        });
    }
    if !delimiters.is_empty() {
        return Err(error(
            "E_SYNTAX",
            Span { start: i, end: i },
            "unclosed delimiter",
        ));
    }
    out.push(Token {
        text: String::new(),
        span: Span { start: i, end: i },
        value: None,
    });
    Ok(out)
}
struct Parser {
    tokens: Vec<Token>,
    at: usize,
    depth: usize,
}
type ResultNode = Result<Node, Diagnostic>;
impl Parser {
    fn token(&self) -> &Token {
        &self.tokens[self.at]
    }
    fn is(&self, s: &str) -> bool {
        self.token().text == s
    }
    fn bump(&mut self) -> Token {
        let t = self.token().clone();
        if !self.is("") {
            self.at += 1;
        }
        t
    }
    fn fail(&self, message: &str) -> Diagnostic {
        error("E_SYNTAX", self.token().span, message)
    }
    fn take(&mut self, s: &str) -> Result<Token, Diagnostic> {
        if self.is(s) {
            Ok(self.bump())
        } else {
            Err(self.fail(&format!("expected {s:?}")))
        }
    }
    fn node(&self, start: usize, kind: Kind) -> Node {
        Node {
            span: Span {
                start,
                end: self.tokens[self.at.saturating_sub(1)].span.end,
            },
            kind,
        }
    }
    fn nl(&mut self) {
        while self.is("\n") {
            self.bump();
        }
    }
    fn separator(&mut self) -> Result<(), Diagnostic> {
        if !self.is("\n") && !self.is("") {
            return Err(self.fail("expected newline"));
        }
        self.nl();
        Ok(())
    }
    fn name(&mut self) -> ResultNode {
        let t = self.token();
        if !t
            .text
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
            || matches!(
                t.text.as_str(),
                "fn" | "return"
                    | "end"
                    | "object"
                    | "match"
                    | "use"
                    | "pub"
                    | "true"
                    | "false"
                    | "and"
                    | "or"
                    | "not"
            )
        {
            return Err(self.fail("expected identifier"));
        }
        let t = self.bump();
        Ok(Node {
            span: t.span,
            kind: Kind::Name(t.text),
        })
    }
    fn body(&mut self, context: u8) -> Result<Vec<Node>, Diagnostic> {
        let mut body = Vec::new();
        self.nl();
        while !self.is("") && !(context != 0 && self.is("end")) {
            if context == 0
                && self.is("use")
                && body.iter().any(|n: &Node| !matches!(n.kind, Kind::Use(_)))
            {
                return Err(self.fail("use must precede other top-level forms"));
            }
            body.push(self.statement(context)?);
            self.separator()?;
        }
        if context != 0 {
            if self.is("") {
                return Err(error("E_END", self.token().span, "expected end before EOF"));
            }
            self.bump();
        }
        Ok(body)
    }
    fn statement(&mut self, context: u8) -> ResultNode {
        let start = self.token().span.start;
        if self.is("pub") && context == 0 {
            self.bump();
            let n = self.statement(context)?;
            if !matches!(
                n.kind,
                Kind::Function { .. } | Kind::Object { .. } | Kind::Assign(..)
            ) {
                return Err(self.fail("pub requires a definition"));
            }
            return Ok(self.node(start, Kind::Public(Box::new(n))));
        }
        if self.is("use") && context == 0 {
            self.bump();
            let n = self.name()?;
            return Ok(self.node(start, Kind::Use(Box::new(n))));
        }
        if self.is("fn") && context != 1 {
            self.bump();
            let name = Box::new(self.name()?);
            self.take("(")?;
            let mut params = Vec::new();
            if !self.is(")") {
                loop {
                    params.push(self.name()?);
                    if !self.is(",") {
                        break;
                    }
                    self.bump();
                }
            }
            self.take(")")?;
            self.take("\n")?;
            let body = self.body(1)?;
            return Ok(self.node(start, Kind::Function { name, params, body }));
        }
        if self.is("object") && context == 0 {
            self.bump();
            let name = Box::new(self.name()?);
            self.take("\n")?;
            let body = self.body(2)?;
            return Ok(self.node(start, Kind::Object { name, body }));
        }
        if self.is("return") && context != 2 {
            self.bump();
            let value = if self.is("\n") || self.is("") {
                None
            } else {
                Some(Box::new(self.expr(0)?))
            };
            return Ok(self.node(start, Kind::Return(value)));
        }
        let left = self.expr(0)?;
        if self.is("=") {
            fn target(n: &Node) -> bool {
                match &n.kind {
                    Kind::Name(_) => true,
                    Kind::Index(n, _) => target(n),
                    _ => false,
                }
            }
            if !target(&left) {
                return Err(self.fail("assignment target must be a name or indexed name"));
            }
            self.bump();
            let right = self.expr(0)?;
            return Ok(self.node(start, Kind::Assign(Box::new(left), Box::new(right))));
        }
        if context == 2 {
            return Err(self.fail("object body requires binding or function"));
        }
        Ok(left)
    }
    fn args(&mut self, close: &str) -> Result<Vec<Node>, Diagnostic> {
        let mut args = Vec::new();
        if !self.is(close) {
            loop {
                args.push(self.expr(0)?);
                if !self.is(",") {
                    break;
                }
                self.bump();
            }
        }
        self.take(close)?;
        Ok(args)
    }
    fn qualified(&mut self) -> ResultNode {
        let mut n = self.name()?;
        while self.is(".") {
            self.bump();
            let member = self.name()?;
            n = self.node(n.span.start, Kind::Member(Box::new(n), Box::new(member)));
        }
        Ok(n)
    }
    fn expr(&mut self, min: u8) -> ResultNode {
        if self.depth >= 128 {
            return Err(self.fail("expression nesting limit (128) exceeded"));
        }
        self.depth += 1;
        let result = self.expression(min);
        self.depth -= 1;
        result
    }
    fn expression(&mut self, min: u8) -> ResultNode {
        let start = self.token().span.start;
        let mut left = if self.is("-") || self.is("not") {
            let op = self.bump().text;
            let n = self.expr(7)?;
            self.node(start, Kind::Unary(op, Box::new(n)))
        } else if self.is("(") {
            self.bump();
            let n = self.expr(0)?;
            self.take(")")?;
            self.node(start, Kind::Group(Box::new(n)))
        } else if self.is("[") {
            self.bump();
            let args = self.args("]")?;
            self.node(start, Kind::Array(args))
        } else if self.is("match") {
            self.match_expr()?
        } else if self.token().value.is_some() {
            let t = self.bump();
            self.node(start, Kind::String(t.value.clone().unwrap_or_default()))
        } else if self.is("true") || self.is("false") {
            let t = self.bump();
            self.node(start, Kind::Bool(t.text == "true"))
        } else if self
            .token()
            .text
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_digit)
        {
            let t = self.bump();
            self.node(start, Kind::Number(t.text))
        } else {
            self.qualified()?
        };
        let mut compared = false;
        loop {
            if min <= 9 && (self.is("(") || self.is("[")) {
                let call = self.is("(");
                self.bump();
                left = if call {
                    let args = self.args(")")?;
                    self.node(start, Kind::Call(Box::new(left), args))
                } else {
                    let index = self.expr(0)?;
                    self.take("]")?;
                    self.node(start, Kind::Index(Box::new(left), Box::new(index)))
                };
                continue;
            }
            let p = match self.token().text.as_str() {
                "->" => 1,
                "or" => 2,
                "and" => 3,
                "==" | "!=" | "<" | ">" | "<=" | ">=" => 4,
                "+" | "-" => 5,
                "*" | "/" | "%" => 6,
                "^" => 8,
                _ => 0,
            };
            if p == 0 || p < min {
                break;
            }
            if p == 4 && compared {
                return Err(self.fail("comparisons cannot be chained"));
            }
            compared |= p == 4;
            let op = self.bump().text;
            let right = if p == 1 {
                let n = self.qualified()?;
                if self.is("(") {
                    self.bump();
                    let args = self.args(")")?;
                    self.node(n.span.start, Kind::Call(Box::new(n), args))
                } else {
                    n
                }
            } else {
                self.expr(if p == 8 { 7 } else { p + 1 })?
            };
            left = self.node(start, Kind::Binary(op, Box::new(left), Box::new(right)));
        }
        Ok(left)
    }
    fn match_expr(&mut self) -> ResultNode {
        let start = self.bump().span.start;
        let subject = Box::new(self.expr(0)?);
        self.take("\n")?;
        self.nl();
        let mut arms = Vec::new();
        while !self.is("end") && !self.is("") {
            let arm_start = self.token().span.start;
            let pattern = if self.is("*") {
                self.bump();
                self.node(arm_start, Kind::Wildcard)
            } else {
                let comparator = if matches!(
                    self.token().text.as_str(),
                    "==" | "!=" | "<" | ">" | "<=" | ">="
                ) {
                    Some(self.bump().text)
                } else {
                    None
                };
                let negative = if self.is("-") {
                    self.bump();
                    true
                } else {
                    false
                };
                let t = self.bump();
                let kind = if !negative && t.value.is_some() {
                    Kind::String(t.value.clone().unwrap_or_default())
                } else if matches!(t.text.as_str(), "true" | "false") && !negative {
                    Kind::Bool(t.text == "true")
                } else if t.text.as_bytes().first().is_some_and(u8::is_ascii_digit) {
                    Kind::Number(t.text)
                } else {
                    return Err(self.fail("expected literal pattern"));
                };
                let mut n = self.node(arm_start, kind);
                if negative {
                    n = self.node(arm_start, Kind::Unary("-".into(), Box::new(n)));
                }
                if let Some(op) = comparator {
                    n = self.node(arm_start, Kind::Unary(op, Box::new(n)));
                }
                n
            };
            self.take(":")?;
            let value = self.expr(0)?;
            arms.push(self.node(
                arm_start,
                Kind::Arm {
                    pattern: Box::new(pattern),
                    value: Box::new(value),
                },
            ));
            self.take("\n")?;
            self.nl();
        }
        if self.is("") {
            return Err(error("E_END", self.token().span, "expected match end"));
        }
        if arms.is_empty() {
            return Err(self.fail("match requires at least one arm"));
        }
        self.take("end")?;
        Ok(self.node(start, Kind::Match { subject, arms }))
    }
}
pub fn parse(source: &str) -> Result<Vec<Node>, Diagnostic> {
    Parser {
        tokens: lex(source)?,
        at: 0,
        depth: 0,
    }
    .body(0)
}
