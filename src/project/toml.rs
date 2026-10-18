//! Native TOML 1.0 reader for project manifests and lockfiles.
//!
//! The parser is independent of Cargo's implementation. It accepts the TOML
//! 1.0 grammar used by manifests: bare/quoted/dotted keys, basic/literal and
//! multiline strings with escapes, decimal/hex/octal/binary integers, floats
//! including `inf`/`nan`, booleans, offset/local date-times (kept as validated
//! text), arrays, inline tables, standard tables and arrays of tables. It
//! enforces the definition rules: no duplicate keys, no redefinition of a
//! table, no extension of inline tables or static arrays, and dotted keys may
//! only extend tables they created. Every value retains its byte offset so
//! manifest diagnostics can point at the source.

use std::collections::BTreeMap;
use std::fmt;

/// Maximum nesting depth of arrays/inline tables, bounding recursion.
pub const MAX_DEPTH: usize = 128;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    /// RFC 3339 date, time or date-time, validated and kept verbatim.
    Datetime(String),
    Array(Vec<Spanned>),
    Table(Table),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Spanned {
    pub value: Value,
    /// Byte offset of the value's first character.
    pub offset: usize,
}

pub type Table = BTreeMap<String, Spanned>;

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::String(_) => "string",
            Value::Integer(_) => "integer",
            Value::Float(_) => "float",
            Value::Boolean(_) => "boolean",
            Value::Datetime(_) => "datetime",
            Value::Array(_) => "array",
            Value::Table(_) => "table",
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_table(&self) -> Option<&Table> {
        match self {
            Value::Table(t) => Some(t),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Spanned]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub offset: usize,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TOML parse error at line {}, column {}: {}",
            self.line, self.column, self.message
        )
    }
}

impl std::error::Error for ParseError {}

/// How a table came into existence, which governs later redefinition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    /// Created implicitly as a parent of a `[a.b]` header. May be defined later.
    Implicit,
    /// Defined by its own `[header]`. May not be defined again.
    Header,
    /// Created by a dotted key inside a table body. Closed to headers.
    Dotted,
    /// Inline `{ }` table or array-of-tables element: sealed / handled apart.
    Inline,
}

/// Parse state tree mirroring the final value tree, carrying origins.
#[derive(Debug)]
enum Node {
    Leaf(Spanned),
    Table {
        entries: BTreeMap<String, Node>,
        origin: Origin,
        offset: usize,
    },
    /// Array of tables defined by `[[header]]`.
    TableArray(Vec<(usize, BTreeMap<String, Node>)>),
}

impl Node {
    fn new_table(origin: Origin, offset: usize) -> Node {
        Node::Table {
            entries: BTreeMap::new(),
            origin,
            offset,
        }
    }
    fn finish(self) -> Spanned {
        match self {
            Node::Leaf(s) => s,
            Node::Table {
                entries, offset, ..
            } => Spanned {
                value: Value::Table(finish_map(entries)),
                offset,
            },
            Node::TableArray(items) => {
                let offset = items.first().map_or(0, |i| i.0);
                Spanned {
                    value: Value::Array(
                        items
                            .into_iter()
                            .map(|(offset, map)| Spanned {
                                value: Value::Table(finish_map(map)),
                                offset,
                            })
                            .collect(),
                    ),
                    offset,
                }
            }
        }
    }
}

fn finish_map(map: BTreeMap<String, Node>) -> Table {
    map.into_iter().map(|(k, v)| (k, v.finish())).collect()
}

/// Parse a complete TOML document into its root table.
pub fn parse(source: &str) -> Result<Table, ParseError> {
    let mut parser = Parser {
        src: source,
        bytes: source.as_bytes(),
        pos: 0,
    };
    let mut root: BTreeMap<String, Node> = BTreeMap::new();
    // Path of the current table header; `array` marks the last segment as [[ ]].
    let mut current: Vec<String> = Vec::new();
    if source.starts_with('\u{feff}') {
        parser.pos = 3;
    }
    loop {
        parser.skip_ws_comments_newlines()?;
        if parser.at_end() {
            break;
        }
        let start = parser.pos;
        if parser.peek() == Some(b'[') {
            let array = parser.bytes.get(parser.pos + 1) == Some(&b'[');
            parser.pos += if array { 2 } else { 1 };
            parser.skip_ws();
            let keys = parser.key()?;
            parser.skip_ws();
            if array {
                parser.expect_str("]]", "expected `]]` to close array of tables header")?;
            } else {
                parser.expect(b']', "expected `]` to close table header")?;
            }
            parser.end_of_line()?;
            define_header(&mut root, &keys, array, start).map_err(|m| parser.error_at(start, m))?;
            current = keys.into_iter().map(|k| k.0).collect();
        } else {
            let keys = parser.key()?;
            parser.skip_ws();
            parser.expect(b'=', "expected `=` after key")?;
            parser.skip_ws();
            let value = parser.value(0)?;
            parser.end_of_line()?;
            let table = current_table(&mut root, &current);
            insert_dotted(table, &keys, value).map_err(|(o, m)| parser.error_at(o, m))?;
        }
    }
    Ok(finish_map(root))
}

/// Resolve the table targeted by the current header path. Headers were
/// validated when defined, so this walk always succeeds.
fn current_table<'a>(
    root: &'a mut BTreeMap<String, Node>,
    path: &[String],
) -> &'a mut BTreeMap<String, Node> {
    let mut table = root;
    for key in path {
        table = match table.get_mut(key).expect("validated header path") {
            Node::Table { entries, .. } => entries,
            Node::TableArray(items) => &mut items.last_mut().expect("nonempty").1,
            Node::Leaf(_) => unreachable!("header path validated as tables"),
        };
    }
    table
}

fn define_header(
    root: &mut BTreeMap<String, Node>,
    keys: &[(String, usize)],
    array: bool,
    offset: usize,
) -> Result<(), String> {
    let (last, parents) = keys.split_last().expect("key has one segment");
    let mut table = root;
    for (key, _) in parents {
        let node = table
            .entry(key.clone())
            .or_insert_with(|| Node::new_table(Origin::Implicit, offset));
        table = match node {
            Node::Table {
                entries, origin, ..
            } => {
                if *origin == Origin::Inline {
                    return Err(format!("cannot extend inline table `{key}`"));
                }
                entries
            }
            Node::TableArray(items) => &mut items.last_mut().expect("nonempty").1,
            Node::Leaf(s) => {
                return Err(match s.value {
                    Value::Array(_) => format!("cannot extend static array `{key}`"),
                    _ => format!(
                        "key `{key}` is already defined as a {}",
                        s.value.type_name()
                    ),
                });
            }
        };
    }
    let key = &last.0;
    if array {
        match table
            .entry(key.clone())
            .or_insert_with(|| Node::TableArray(Vec::new()))
        {
            Node::TableArray(items) => items.push((offset, BTreeMap::new())),
            Node::Leaf(s) if matches!(s.value, Value::Array(_)) => {
                return Err(format!("cannot append to static array `{key}`"));
            }
            _ => {
                return Err(format!(
                    "key `{key}` is already defined and is not an array of tables"
                ));
            }
        }
        return Ok(());
    }
    match table.get_mut(key) {
        None => {
            table.insert(key.clone(), Node::new_table(Origin::Header, offset));
            Ok(())
        }
        Some(Node::Table {
            origin, offset: o, ..
        }) if *origin == Origin::Implicit => {
            *origin = Origin::Header;
            *o = offset;
            Ok(())
        }
        Some(Node::Table {
            origin: Origin::Dotted,
            ..
        }) => Err(format!("table `{key}` was already defined by dotted keys")),
        Some(_) => Err(format!("duplicate definition of table `{key}`")),
    }
}

fn insert_dotted(
    table: &mut BTreeMap<String, Node>,
    keys: &[(String, usize)],
    value: Spanned,
) -> Result<(), (usize, String)> {
    let (last, parents) = keys.split_last().expect("key has one segment");
    let mut table = table;
    for (key, offset) in parents {
        let node = table
            .entry(key.clone())
            .or_insert_with(|| Node::new_table(Origin::Dotted, *offset));
        table = match node {
            Node::Table {
                entries,
                origin: Origin::Dotted,
                ..
            } => entries,
            Node::Table { .. } | Node::TableArray(_) => {
                return Err((
                    *offset,
                    format!("cannot extend table `{key}` with dotted keys"),
                ));
            }
            Node::Leaf(s) => {
                return Err((
                    *offset,
                    format!(
                        "key `{key}` is already defined as a {}",
                        s.value.type_name()
                    ),
                ));
            }
        };
    }
    if table.contains_key(&last.0) {
        return Err((last.1, format!("duplicate key `{}`", last.0)));
    }
    table.insert(last.0.clone(), Node::Leaf(value));
    Ok(())
}

struct Parser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
    fn error_at(&self, offset: usize, message: impl Into<String>) -> ParseError {
        let offset = offset.min(self.src.len());
        let before = &self.src[..floor_char_boundary(self.src, offset)];
        let line = before.matches('\n').count() + 1;
        let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        ParseError {
            offset,
            line,
            column,
            message: message.into(),
        }
    }
    fn error(&self, message: impl Into<String>) -> ParseError {
        self.error_at(self.pos, message)
    }
    fn expect(&mut self, byte: u8, message: &str) -> Result<(), ParseError> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.error(message))
        }
    }
    fn expect_str(&mut self, text: &str, message: &str) -> Result<(), ParseError> {
        if self.bytes[self.pos..].starts_with(text.as_bytes()) {
            self.pos += text.len();
            Ok(())
        } else {
            Err(self.error(message))
        }
    }
    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.pos += 1;
        }
    }
    fn comment(&mut self) -> Result<(), ParseError> {
        if self.peek() != Some(b'#') {
            return Ok(());
        }
        while let Some(b) = self.peek() {
            if b == b'\n' || (b == b'\r' && self.bytes.get(self.pos + 1) == Some(&b'\n')) {
                break;
            }
            if is_control(b) && b != b'\t' {
                return Err(self.error("control character in comment"));
            }
            self.pos += 1;
        }
        Ok(())
    }
    fn newline(&mut self) -> bool {
        match self.peek() {
            Some(b'\n') => {
                self.pos += 1;
                true
            }
            Some(b'\r') if self.bytes.get(self.pos + 1) == Some(&b'\n') => {
                self.pos += 2;
                true
            }
            _ => false,
        }
    }
    fn skip_ws_comments_newlines(&mut self) -> Result<(), ParseError> {
        loop {
            self.skip_ws();
            self.comment()?;
            if !self.newline() {
                return Ok(());
            }
        }
    }
    fn end_of_line(&mut self) -> Result<(), ParseError> {
        self.skip_ws();
        self.comment()?;
        if self.at_end() || self.newline() {
            Ok(())
        } else {
            Err(self.error("expected newline after value"))
        }
    }

    /// A possibly dotted key. Each segment keeps its offset.
    fn key(&mut self) -> Result<Vec<(String, usize)>, ParseError> {
        let mut keys = Vec::new();
        loop {
            self.skip_ws();
            let offset = self.pos;
            let segment = match self.peek() {
                Some(b'"') => {
                    if self.bytes[self.pos..].starts_with(b"\"\"\"") {
                        return Err(self.error("multiline strings are not allowed as keys"));
                    }
                    self.basic_string()?
                }
                Some(b'\'') => {
                    if self.bytes[self.pos..].starts_with(b"'''") {
                        return Err(self.error("multiline strings are not allowed as keys"));
                    }
                    self.literal_string()?
                }
                Some(b) if is_bare(b) => {
                    let start = self.pos;
                    while self.peek().is_some_and(is_bare) {
                        self.pos += 1;
                    }
                    self.src[start..self.pos].to_string()
                }
                _ => return Err(self.error("expected a key")),
            };
            keys.push((segment, offset));
            self.skip_ws();
            if self.peek() == Some(b'.') {
                self.pos += 1;
            } else {
                return Ok(keys);
            }
        }
    }

    fn value(&mut self, depth: usize) -> Result<Spanned, ParseError> {
        if depth >= MAX_DEPTH {
            return Err(self.error("value nesting exceeds the supported depth"));
        }
        let offset = self.pos;
        let rest = &self.bytes[self.pos..];
        let value = match self.peek() {
            Some(b'"') if rest.starts_with(b"\"\"\"") => Value::String(self.ml_basic_string()?),
            Some(b'"') => Value::String(self.basic_string()?),
            Some(b'\'') if rest.starts_with(b"'''") => Value::String(self.ml_literal_string()?),
            Some(b'\'') => Value::String(self.literal_string()?),
            Some(b'[') => self.array(depth)?,
            Some(b'{') => self.inline_table(depth)?,
            Some(b't')
                if rest.starts_with(b"true") && !rest.get(4).is_some_and(|b| is_bare(*b)) =>
            {
                self.pos += 4;
                Value::Boolean(true)
            }
            Some(b'f')
                if rest.starts_with(b"false") && !rest.get(5).is_some_and(|b| is_bare(*b)) =>
            {
                self.pos += 5;
                Value::Boolean(false)
            }
            Some(_) => self.scalar()?,
            None => return Err(self.error("expected a value")),
        };
        Ok(Spanned { value, offset })
    }

    fn array(&mut self, depth: usize) -> Result<Value, ParseError> {
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.skip_ws_comments_newlines()?;
            if self.peek() == Some(b']') {
                self.pos += 1;
                return Ok(Value::Array(items));
            }
            items.push(self.value(depth + 1)?);
            self.skip_ws_comments_newlines()?;
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.error("expected `,` or `]` in array")),
            }
        }
    }

    fn inline_table(&mut self, depth: usize) -> Result<Value, ParseError> {
        self.pos += 1;
        let mut table: BTreeMap<String, Node> = BTreeMap::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Value::Table(Table::new()));
        }
        loop {
            self.skip_ws();
            let keys = self.key()?;
            self.skip_ws();
            self.expect(b'=', "expected `=` after key")?;
            self.skip_ws();
            let value = self.value(depth + 1)?;
            insert_dotted(&mut table, &keys, value).map_err(|(o, m)| self.error_at(o, m))?;
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    self.skip_ws();
                    if self.peek() == Some(b'}') {
                        return Err(self.error("trailing comma is not allowed in inline table"));
                    }
                }
                Some(b'}') => {
                    self.pos += 1;
                    // Dotted tables inside an inline table are sealed with it.
                    seal(&mut table);
                    return Ok(Value::Table(finish_map(table)));
                }
                Some(b'\n' | b'\r') => {
                    return Err(self.error("newline is not allowed in inline table"));
                }
                _ => return Err(self.error("expected `,` or `}` in inline table")),
            }
        }
    }

    fn basic_string(&mut self) -> Result<String, ParseError> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            match self.peek() {
                None | Some(b'\n') => return Err(self.error("unterminated string")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => self.escape(&mut out)?,
                Some(b) if is_control(b) && b != b'\t' => {
                    return Err(self.error("control character in string"));
                }
                Some(_) => self.push_char(&mut out),
            }
        }
    }

    fn ml_basic_string(&mut self) -> Result<String, ParseError> {
        self.pos += 3;
        self.newline();
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err(self.error("unterminated multiline string")),
                Some(b'"') if self.bytes[self.pos..].starts_with(b"\"\"\"") => {
                    // Up to two quotes may directly precede the delimiter.
                    let mut quotes = 3;
                    while quotes < 5 && self.bytes.get(self.pos + quotes) == Some(&b'"') {
                        quotes += 1;
                    }
                    out.extend(std::iter::repeat_n('"', quotes - 3));
                    self.pos += quotes;
                    return Ok(out);
                }
                Some(b'\\') => {
                    // Line-ending backslash trims following whitespace/newlines.
                    let mut probe = self.pos + 1;
                    while matches!(self.bytes.get(probe), Some(b' ' | b'\t')) {
                        probe += 1;
                    }
                    let crlf = self.bytes[probe..].starts_with(b"\r\n");
                    if self.bytes.get(probe) == Some(&b'\n') || crlf {
                        self.pos = probe;
                        while matches!(self.peek(), Some(b' ' | b'\t')) || self.newline() {
                            if matches!(self.peek(), Some(b' ' | b'\t')) {
                                self.pos += 1;
                            }
                        }
                    } else {
                        self.escape(&mut out)?;
                    }
                }
                Some(b'\r') => {
                    if !self.newline() {
                        return Err(self.error("bare carriage return in string"));
                    }
                    out.push('\n');
                }
                Some(b'\n') => {
                    self.pos += 1;
                    out.push('\n');
                }
                Some(b) if is_control(b) && b != b'\t' => {
                    return Err(self.error("control character in string"));
                }
                Some(_) => self.push_char(&mut out),
            }
        }
    }

    fn literal_string(&mut self) -> Result<String, ParseError> {
        self.pos += 1;
        let start = self.pos;
        loop {
            match self.peek() {
                None | Some(b'\n') => return Err(self.error("unterminated literal string")),
                Some(b'\'') => {
                    let text = self.src[start..self.pos].to_string();
                    self.pos += 1;
                    return Ok(text);
                }
                Some(b) if is_control(b) && b != b'\t' => {
                    return Err(self.error("control character in string"));
                }
                Some(_) => self.pos += 1,
            }
        }
    }

    fn ml_literal_string(&mut self) -> Result<String, ParseError> {
        self.pos += 3;
        self.newline();
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err(self.error("unterminated multiline literal string")),
                Some(b'\'') if self.bytes[self.pos..].starts_with(b"'''") => {
                    let mut quotes = 3;
                    while quotes < 5 && self.bytes.get(self.pos + quotes) == Some(&b'\'') {
                        quotes += 1;
                    }
                    out.extend(std::iter::repeat_n('\'', quotes - 3));
                    self.pos += quotes;
                    return Ok(out);
                }
                Some(b'\r') => {
                    if !self.newline() {
                        return Err(self.error("bare carriage return in string"));
                    }
                    out.push('\n');
                }
                Some(b'\n') => {
                    self.pos += 1;
                    out.push('\n');
                }
                Some(b) if is_control(b) && b != b'\t' => {
                    return Err(self.error("control character in string"));
                }
                Some(_) => self.push_char(&mut out),
            }
        }
    }

    fn push_char(&mut self, out: &mut String) {
        let c = self.src[self.pos..].chars().next().expect("not at end");
        out.push(c);
        self.pos += c.len_utf8();
    }

    fn escape(&mut self, out: &mut String) -> Result<(), ParseError> {
        let at = self.pos;
        self.pos += 1;
        let c = match self.peek() {
            Some(b'b') => '\u{8}',
            Some(b't') => '\t',
            Some(b'n') => '\n',
            Some(b'f') => '\u{c}',
            Some(b'r') => '\r',
            Some(b'"') => '"',
            Some(b'\\') => '\\',
            Some(b @ (b'u' | b'U')) => {
                let len = if b == b'u' { 4 } else { 8 };
                let digits = self
                    .src
                    .get(self.pos + 1..self.pos + 1 + len)
                    .filter(|d| d.bytes().all(|b| b.is_ascii_hexdigit()))
                    .ok_or_else(|| self.error_at(at, "invalid unicode escape"))?;
                let code = u32::from_str_radix(digits, 16).expect("hex digits");
                self.pos += len;
                char::from_u32(code)
                    .ok_or_else(|| self.error_at(at, "escape is not a Unicode scalar value"))?
            }
            _ => return Err(self.error_at(at, "invalid escape sequence")),
        };
        self.pos += 1;
        out.push(c);
        Ok(())
    }

    /// Numbers, special floats and date-times.
    fn scalar(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;
        // Date-times may contain one space between date and time.
        while let Some(b) = self.peek() {
            let joins_time = b == b' '
                && is_date(&self.src[start..self.pos])
                && self.bytes.get(self.pos + 1).is_some_and(u8::is_ascii_digit);
            if b.is_ascii_alphanumeric()
                || matches!(b, b'+' | b'-' | b'_' | b'.' | b':')
                || joins_time
            {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text = &self.src[start..self.pos];
        if text.is_empty() {
            return Err(self.error("expected a value"));
        }
        if let Some(value) = parse_integer(text) {
            return value
                .map(Value::Integer)
                .map_err(|m| self.error_at(start, m));
        }
        if let Some(value) = parse_float(text) {
            return Ok(Value::Float(value));
        }
        if is_datetime(text) {
            return Ok(Value::Datetime(text.to_string()));
        }
        Err(self.error_at(start, format!("invalid value `{text}`")))
    }
}

/// Mark dotted subtables created inside an inline table as sealed.
fn seal(table: &mut BTreeMap<String, Node>) {
    for node in table.values_mut() {
        if let Node::Table {
            entries, origin, ..
        } = node
        {
            *origin = Origin::Inline;
            seal(entries);
        }
    }
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn is_bare(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn is_control(b: u8) -> bool {
    b < 0x20 || b == 0x7f
}

fn digits_ok(body: &str, radix: u32) -> bool {
    !body.is_empty()
        && !body.starts_with('_')
        && !body.ends_with('_')
        && !body.contains("__")
        && body.chars().all(|c| c == '_' || c.is_digit(radix))
}

/// `None` when `text` is not integer-shaped, `Some(Err)` when it is but is
/// out of range or malformed in an integer-specific way.
fn parse_integer(text: &str) -> Option<Result<i64, String>> {
    for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
        if let Some(body) = text.strip_prefix(prefix) {
            if !digits_ok(body, radix) {
                return Some(Err(format!("invalid integer `{text}`")));
            }
            return Some(
                i64::from_str_radix(&body.replace('_', ""), radix)
                    .map_err(|_| format!("integer `{text}` is out of range")),
            );
        }
    }
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    if !digits_ok(unsigned, 10) {
        return None;
    }
    if unsigned.len() > 1 && unsigned.starts_with('0') {
        return Some(Err(format!("leading zeros are not allowed in `{text}`")));
    }
    Some(
        text.replace('_', "")
            .parse::<i64>()
            .map_err(|_| format!("integer `{text}` is out of range")),
    )
}

fn parse_float(text: &str) -> Option<f64> {
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    let negative = text.starts_with('-');
    match unsigned {
        "inf" => {
            return Some(if negative {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            });
        }
        "nan" => return Some(if negative { -f64::NAN } else { f64::NAN }),
        _ => {}
    }
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(i) => (&unsigned[..i], Some(&unsigned[i + 1..])),
        None => (unsigned, None),
    };
    let (int, frac) = match mantissa.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (mantissa, None),
    };
    if frac.is_none() && exponent.is_none() {
        return None;
    }
    if !digits_ok(int, 10) || (int.len() > 1 && int.starts_with('0')) {
        return None;
    }
    if frac.is_some_and(|f| !digits_ok(f, 10)) {
        return None;
    }
    if let Some(e) = exponent
        && !digits_ok(e.strip_prefix(['+', '-']).unwrap_or(e), 10)
    {
        return None;
    }
    text.replace('_', "").parse().ok()
}

fn fixed_digits(s: &str, n: usize) -> Option<u32> {
    (s.len() == n && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().expect("digits"))
}

fn is_date(s: &str) -> bool {
    let mut parts = s.split('-');
    let (Some(y), Some(m), Some(d), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let (Some(y), Some(m), Some(d)) = (fixed_digits(y, 4), fixed_digits(m, 2), fixed_digits(d, 2))
    else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&d)
}

fn is_time(s: &str) -> bool {
    let (clock, frac) = match s.split_once('.') {
        Some((c, f)) => (c, Some(f)),
        None => (s, None),
    };
    if frac.is_some_and(|f| f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit())) {
        return false;
    }
    let mut parts = clock.split(':');
    let (Some(h), Some(m), Some(sec), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    matches!(
        (fixed_digits(h, 2), fixed_digits(m, 2), fixed_digits(sec, 2)),
        (Some(h), Some(m), Some(s)) if h < 24 && m < 60 && s <= 60
    )
}

fn is_offset(s: &str) -> bool {
    if s == "Z" || s == "z" {
        return true;
    }
    let Some(body) = s.strip_prefix(['+', '-']) else {
        return false;
    };
    matches!(body.split_once(':'), Some((h, m))
        if fixed_digits(h, 2).is_some_and(|h| h < 24) && fixed_digits(m, 2).is_some_and(|m| m < 60))
}

fn is_datetime(text: &str) -> bool {
    if is_date(text) || is_time(text) {
        return true;
    }
    let Some(sep) = text.find(['T', 't', ' ']) else {
        return false;
    };
    let (date, rest) = (&text[..sep], &text[sep + 1..]);
    if !is_date(date) {
        return false;
    }
    let split = rest.find(['Z', 'z', '+', '-']).unwrap_or(rest.len());
    let (time, offset) = rest.split_at(split);
    is_time(time) && (offset.is_empty() || is_offset(offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get<'a>(t: &'a Table, path: &str) -> &'a Value {
        let mut value = None;
        let mut table = t;
        for key in path.split('.') {
            let v = &table
                .get(key)
                .unwrap_or_else(|| panic!("missing {key}"))
                .value;
            if let Value::Table(inner) = v {
                table = inner;
            }
            value = Some(v);
        }
        value.unwrap()
    }

    #[test]
    fn manifest_shapes() {
        let t = parse(
            r#"
# comment
[package]
name = "demo" # trailing
version = '0.1.0'
authors = ["a", "b",]
edition.workspace = true

[dependencies]
serde = { version = "1", features = ["derive"], default-features = false }
local.path = "../local"

[target.'cfg(unix)'.dependencies]
libc = "0.2"

[[bin]]
name = "one"
[[bin]]
name = "two"
"#,
        )
        .unwrap();
        assert_eq!(get(&t, "package.name"), &Value::String("demo".into()));
        assert_eq!(get(&t, "package.edition.workspace"), &Value::Boolean(true));
        assert_eq!(
            get(&t, "dependencies.serde.default-features"),
            &Value::Boolean(false)
        );
        assert_eq!(
            get(&t, "dependencies.local.path"),
            &Value::String("../local".into())
        );
        assert_eq!(
            get(&t, "target.cfg(unix).dependencies.libc"),
            &Value::String("0.2".into())
        );
        assert_eq!(get(&t, "bin").as_array().unwrap().len(), 2);
    }

    #[test]
    fn strings_and_escapes() {
        let t = parse(
            "a = \"x\\ty\\u00e9\\U0001F600\"\nb = '''\nraw\\n'''\nc = \"\"\"\nline \\\n   joined\"\"\"\nd = \"\"\"q\"\"\"\"\"\n",
        )
        .unwrap();
        assert_eq!(t["a"].value, Value::String("x\ty\u{e9}\u{1F600}".into()));
        assert_eq!(t["b"].value, Value::String("raw\\n".into()));
        assert_eq!(t["c"].value, Value::String("line joined".into()));
        assert_eq!(t["d"].value, Value::String("q\"\"".into()));
        assert!(parse("a = \"\\uD800\"").is_err());
        assert!(parse("a = \"\\q\"").is_err());
        assert!(parse("a = \"open").is_err());
    }

    #[test]
    fn numbers_and_datetimes() {
        let t = parse(
            "a = 1_000\nb = -0x10\nc = 0o17\nd = 0b101\ne = 6.5e-1\nf = -inf\ng = 1979-05-27T07:32:00Z\nh = 1979-05-27 07:32:00.5-07:00\ni = 07:32:00\n",
        );
        // Hex literals may not be signed.
        assert!(t.is_err());
        let t = parse(
            "a = 1_000\nb = 0x1F\nc = 0o17\nd = 0b101\ne = 6.5e-1\nf = -inf\ng = 1979-05-27T07:32:00Z\nh = 1979-05-27 07:32:00.5-07:00\ni = 07:32:00\nj = +12\n",
        )
        .unwrap();
        assert_eq!(t["a"].value, Value::Integer(1000));
        assert_eq!(t["b"].value, Value::Integer(31));
        assert_eq!(t["c"].value, Value::Integer(15));
        assert_eq!(t["d"].value, Value::Integer(5));
        assert_eq!(t["e"].value, Value::Float(0.65));
        assert_eq!(t["f"].value, Value::Float(f64::NEG_INFINITY));
        assert_eq!(t["g"].value, Value::Datetime("1979-05-27T07:32:00Z".into()));
        assert_eq!(
            t["h"].value,
            Value::Datetime("1979-05-27 07:32:00.5-07:00".into())
        );
        assert_eq!(t["i"].value, Value::Datetime("07:32:00".into()));
        assert_eq!(t["j"].value, Value::Integer(12));
        for bad in [
            "a = 01",
            "a = 1__0",
            "a = _1",
            "a = 1.",
            "a = .5",
            "a = 9223372036854775808",
            "a = 2023-02-29",
            "a = 25:00:00",
            "a = truex",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert_eq!(
            parse("a = -9223372036854775808").unwrap()["a"].value,
            Value::Integer(i64::MIN)
        );
    }

    #[test]
    fn definition_rules() {
        for bad in [
            "a = 1\na = 2",
            "[a]\n[a]",
            "[a]\nb = 1\n[a.b]",
            "a = {b = 1}\n[a]",
            "a = {b = 1}\n[a.c]",
            "a.b = 1\n[a]",
            "a.b = 1\n[a.b]",
            "a = [1]\n[[a]]",
            "[[a]]\n[a]",
            "a = 1\na.b = 2",
            "a = {b = 1, b = 2}",
            "a = {b = 1,}",
            "a = {b = 1\n}",
            "[a]\nb.c = 1\n[a.b.c]",
            "a = 1 b = 2",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        // Implicit parent tables may be defined later exactly once.
        let t = parse("[a.b]\nx = 1\n[a]\ny = 2\n").unwrap();
        assert_eq!(get(&t, "a.b.x"), &Value::Integer(1));
        assert!(parse("[a.b]\n[a]\n[a]").is_err());
        // Subtables under array-of-tables elements attach to the last element.
        let t = parse("[[p]]\nn = 1\n[p.sub]\nk = 1\n[[p]]\nn = 2\n").unwrap();
        let items = t["p"].value.as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert!(items[0].value.as_table().unwrap().contains_key("sub"));
        assert!(!items[1].value.as_table().unwrap().contains_key("sub"));
    }

    #[test]
    fn errors_have_positions_and_depth_is_bounded() {
        let e = parse("[package]\nname = \n").unwrap_err();
        assert_eq!((e.line, e.column), (2, 8));
        let deep = format!(
            "a = {}{}",
            "[".repeat(MAX_DEPTH + 1),
            "]".repeat(MAX_DEPTH + 1)
        );
        assert!(parse(&deep).unwrap_err().message.contains("depth"));
        let ok = format!("a = {}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
        assert!(parse(&ok).is_ok());
        assert!(parse("a = \"\u{1}\"").is_err());
        assert!(parse("a = 1 # \u{7f}").is_err());
    }
}
