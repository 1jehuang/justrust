//! `cargo tree --format`: `src/ops/cargo_tree/format/` of cargo 1.101.0 (cad07cf8e5).

use std::fmt;

use anyhow::{Error, bail};

use super::graph::{Graph, Node, NodeId};

enum Chunk {
    Raw(String),
    Package,
    License,
    Repository,
    Features,
    LibName,
}

pub(crate) struct Pattern(Vec<Chunk>);

impl Pattern {
    pub(crate) fn new(format: &str) -> Result<Pattern, Error> {
        let mut chunks = vec![];
        for raw in Parser::new(format) {
            let chunk = match raw {
                RawChunk::Text(text) => Chunk::Raw(text.to_owned()),
                RawChunk::Argument("package" | "p") => Chunk::Package,
                RawChunk::Argument("license" | "l") => Chunk::License,
                RawChunk::Argument("repository" | "r") => Chunk::Repository,
                RawChunk::Argument("features" | "f") => Chunk::Features,
                RawChunk::Argument("lib") => Chunk::LibName,
                RawChunk::Argument(a) => bail!("unsupported pattern `{a}`"),
                RawChunk::Error(err) => bail!("{err}"),
            };
            chunks.push(chunk);
        }
        Ok(Pattern(chunks))
    }

    pub(crate) fn display<'a>(&'a self, graph: &'a Graph<'a>, node_index: NodeId) -> Display<'a> {
        Display { pattern: self, graph, node_index }
    }
}

pub(crate) struct Display<'a> {
    pattern: &'a Pattern,
    graph: &'a Graph<'a>,
    node_index: NodeId,
}

impl fmt::Display for Display<'_> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.graph.node(self.node_index) {
            Node::Package { package_id, features, .. } => {
                let package = self.graph.package(*package_id);
                for chunk in &self.pattern.0 {
                    match chunk {
                        Chunk::Raw(s) => fmt.write_str(s)?,
                        Chunk::Package => {
                            let proc_macro_suffix =
                                if package.any_proc_macro { " (proc-macro)" } else { "" };
                            write!(
                                fmt,
                                "{} v{}{}",
                                package_id.name(),
                                package_id.version(),
                                proc_macro_suffix
                            )?;
                            let source_id = package_id.source_id();
                            if !source_id.is_crates_io() {
                                write!(fmt, " ({source_id})")?;
                            }
                        }
                        Chunk::License => {
                            if let Some(license) = &package.license {
                                write!(fmt, "{license}")?;
                            }
                        }
                        Chunk::Repository => {
                            if let Some(repository) = &package.repository {
                                write!(fmt, "{repository}")?;
                            }
                        }
                        Chunk::Features => write!(fmt, "{}", features.join(","))?,
                        Chunk::LibName => {
                            if let Some(lib) = &package.lib {
                                write!(fmt, "{lib}")?;
                            }
                        }
                    }
                }
            }
            Node::Feature { name, node_index } => match self.graph.node(*node_index) {
                Node::Package { package_id, .. } => {
                    write!(fmt, "{} feature \"{}\"", package_id.name(), name)?;
                    if self.graph.is_cli_feature(self.node_index) {
                        write!(fmt, " (command-line)")?;
                    }
                }
                // The node_index in Node::Feature must point to a package node, see
                // `add_feature`.
                for_node => panic!("unexpected feature node {for_node:?}"),
            },
        }
        Ok(())
    }
}

enum RawChunk<'a> {
    /// Raw text to include in the output.
    Text(&'a str),
    /// A substitution to place in the output: `p` is the package's name and version.
    Argument(&'a str),
    /// What is wrong with the format string, for its reader.
    Error(&'static str),
}

/// The `--format` string: text with alphabetic names in braces, `{p}` or `{foo}`, that are
/// substituted, and `{{` and `}}` for braces themselves.
struct Parser<'a> {
    s: &'a str,
    it: std::iter::Peekable<std::str::CharIndices<'a>>,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Parser<'a> {
        Parser { s, it: s.char_indices().peekable() }
    }

    fn consume(&mut self, ch: char) -> bool {
        match self.it.peek() {
            Some(&(_, c)) if c == ch => {
                self.it.next();
                true
            }
            _ => false,
        }
    }

    fn name(&mut self) -> &'a str {
        let start = match self.it.peek() {
            Some(&(pos, ch)) if ch.is_alphabetic() => {
                self.it.next();
                pos
            }
            _ => return "",
        };
        loop {
            match self.it.peek() {
                Some(&(_, ch)) if ch.is_alphanumeric() => {
                    self.it.next();
                }
                Some(&(end, _)) => return &self.s[start..end],
                None => return &self.s[start..],
            }
        }
    }

    fn text(&mut self, start: usize) -> RawChunk<'a> {
        while let Some(&(pos, ch)) = self.it.peek() {
            match ch {
                '{' | '}' => return RawChunk::Text(&self.s[start..pos]),
                _ => {
                    self.it.next();
                }
            }
        }
        RawChunk::Text(&self.s[start..])
    }
}

impl<'a> Iterator for Parser<'a> {
    type Item = RawChunk<'a>;

    fn next(&mut self) -> Option<RawChunk<'a>> {
        match self.it.peek() {
            Some(&(_, '{')) => {
                self.it.next();
                if self.consume('{') {
                    Some(RawChunk::Text("{"))
                } else {
                    let chunk = RawChunk::Argument(self.name());
                    if self.consume('}') {
                        Some(chunk)
                    } else {
                        for _ in &mut self.it {}
                        Some(RawChunk::Error("expected '}'"))
                    }
                }
            }
            Some(&(_, '}')) => {
                self.it.next();
                if self.consume('}') {
                    Some(RawChunk::Text("}"))
                } else {
                    Some(RawChunk::Error("unexpected '}'"))
                }
            }
            Some(&(i, _)) => Some(self.text(i)),
            None => None,
        }
    }
}
