//! What a module rustc built has for the code compiled here to use: the program is linked
//! into it, over its memory and its function table, as a patch is into the module before it.
//!
//! The module is linked with `--export-all`, for its functions to be had by name, and with
//! `--emit-relocs`, which leaves in it the linker's symbol table: that has where every static
//! is, those of no name outside their object file as well.

use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
pub struct Host {
    /// The functions exported, each with its signature as [`signature`] writes it.
    pub functions: Vec<(String, String)>,
    /// The address of each data symbol.
    pub data: Vec<(String, u32)>,
}

/// A signature as text: a letter per parameter, `:`, a letter per result.
pub fn signature(params: &[u8], results: &[u8]) -> String {
    let letter = |ty: &u8| match ty {
        0x7f => 'i',
        0x7e => 'I',
        0x7d => 'f',
        0x7c => 'F',
        _ => '?',
    };
    let mut text: String = params.iter().map(letter).collect();
    text.push(':');
    text.extend(results.iter().map(letter));
    text
}

#[cfg(not(target_family = "wasm"))]
impl Host {
    /// Reads the exports and the symbol table of a linked module.
    pub fn read(module: &[u8]) -> Result<Host, String> {
        use wasmparser::{
            DataKind, ExternalKind, KnownCustom, Linking, Operator, Parser, Payload, SymbolInfo,
            TypeRef, ValType,
        };
        let byte = |ty: &ValType| match ty {
            ValType::I32 => 0x7f,
            ValType::I64 => 0x7e,
            ValType::F32 => 0x7d,
            ValType::F64 => 0x7c,
            _ => 0,
        };
        let error = |e: wasmparser::BinaryReaderError| format!("host: {e}");
        let mut types = Vec::new();
        // The type of each function, the imported ones first.
        let mut functions = Vec::new();
        let mut exported = Vec::new();
        let mut segments = Vec::new();
        let mut data: Vec<(String, u32)> = Vec::new();
        let mut placed = Vec::new();
        for payload in Parser::new(0).parse_all(module) {
            match payload.map_err(error)? {
                Payload::TypeSection(reader) => {
                    for ty in reader.into_iter_err_on_gc_types() {
                        let ty = ty.map_err(error)?;
                        let params: Vec<u8> = ty.params().iter().map(byte).collect();
                        let results: Vec<u8> = ty.results().iter().map(byte).collect();
                        types.push(signature(&params, &results));
                    }
                }
                Payload::ImportSection(reader) => {
                    for import in reader.into_imports() {
                        if let TypeRef::Func(ty) = import.map_err(error)?.ty {
                            functions.push(ty);
                        }
                    }
                }
                Payload::FunctionSection(reader) => {
                    for ty in reader {
                        functions.push(ty.map_err(error)?);
                    }
                }
                Payload::ExportSection(reader) => {
                    for export in reader {
                        let export = export.map_err(error)?;
                        if export.kind == ExternalKind::Func {
                            exported.push((export.name.to_owned(), export.index));
                        }
                    }
                }
                Payload::DataSection(reader) => {
                    for segment in reader {
                        let DataKind::Active { offset_expr, .. } = segment.map_err(error)?.kind
                        else {
                            segments.push(None);
                            continue;
                        };
                        let mut ops = offset_expr.get_operators_reader();
                        segments.push(match ops.read().map_err(error)? {
                            Operator::I32Const { value } => Some(value as u32),
                            _ => None,
                        });
                    }
                }
                Payload::CustomSection(reader) => {
                    let KnownCustom::Linking(linking) = reader.as_known() else { continue };
                    for subsection in linking.subsections() {
                        let Linking::SymbolTable(symbols) = subsection.map_err(error)? else {
                            continue;
                        };
                        for symbol in symbols {
                            if let SymbolInfo::Data { name, symbol: Some(at), .. } =
                                symbol.map_err(error)?
                            {
                                placed.push((name.to_owned(), at.index, at.offset));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        for (name, segment, offset) in placed {
            // The symbol table is before the data in some modules and after it in others.
            if let Some(Some(start)) = segments.get(segment as usize) {
                data.push((name, start + offset));
            }
        }
        // Two statics of one name, each private to its object, are not told apart by it.
        data.sort();
        let mut unique: Vec<(String, u32)> = Vec::new();
        let mut at = 0;
        while at < data.len() {
            let same = data[at..].iter().take_while(|it| it.0 == data[at].0).count();
            if same == 1 {
                unique.push(data[at].clone());
            }
            at += same;
        }
        let functions = exported
            .into_iter()
            .filter_map(|(name, index)| {
                let ty = *functions.get(index as usize)?;
                Some((name, types.get(ty as usize)?.clone()))
            })
            .collect();
        Ok(Host { functions, data: unique })
    }
}
