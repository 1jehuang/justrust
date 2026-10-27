//! Exact graph expectations from the same real embedded source files.
//! Direct frontend/authority tests intentionally remain user-only.
#![allow(dead_code)]
use justrust_engine::Key;
use justrust_engine::compiler::{frontend, source, token_primitives::ExpansionBudget};
use std::sync::Arc;

pub fn parse(text: &str) -> Result<frontend::Program, String> {
    parse_configured(text, &source::SourceOptions::default())
}

pub fn parse_configured(
    text: &str,
    options: &source::SourceOptions,
) -> Result<frontend::Program, String> {
    let (prepared, _) = prepare(text, options)?;
    let parsed = frontend::parse_prepared(&prepared).map_err(|error| error.to_string())?;
    frontend::resolve(&parsed)
}

// Some graph tests only need declarations, others also assert the entire
// per-file inventory. Both are derived from actual source preparation.
#[allow(dead_code)]
pub fn library_source_keys() -> Vec<Key> {
    prepare("fn main() {}", &source::SourceOptions::default())
        .expect("embedded library source units prepare")
        .1
}

fn prepare(
    text: &str,
    options: &source::SourceOptions,
) -> Result<(Box<source::PreparedSource>, Vec<Key>), String> {
    let budget = ExpansionBudget::default();
    let mut assembler = source::SourceAssembler::new(options);
    let mut step = assembler
        .provide(Arc::new(source::prepare_text_unit("<text>", text, options)))
        .map_err(|error| error.to_string())?;
    let mut library_keys = Vec::new();
    loop {
        match step {
            source::AssemblyStep::Done(prepared) => return Ok((prepared, library_keys)),
            source::AssemblyStep::Need(need) => {
                library_keys.push(Key::new(107, need.ordinal));
                let unit = match need.parent {
                    None => source::prepare_prelude_unit(options, &budget),
                    Some(parent) => {
                        source::prepare_child_unit(&parent, need.index, options, &budget)
                    }
                };
                step = assembler
                    .provide(Arc::new(unit))
                    .map_err(|error| error.to_string())?;
            }
        }
    }
}
