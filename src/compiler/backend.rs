//! Direct Linux x86-64 backend. No external compiler, assembler, or linker.
//!
//! Private ABI: arguments are evaluated left-to-right and pushed as eight-byte
//! stack words. The callee reads them in reverse order above rbp. eax carries
//! i32/bool results (unit is zero). All registers are caller-saved. Frames use
//! stable rbp-relative slots, independent of temporary pushes. At most 4096
//! parameters/arguments are accepted. This is not a platform interoperability ABI.
//! Aggregate layout is private: each scalar/unit leaf occupies eight bytes,
//! tuples concatenate their fields. Tuple arguments carry snapshot pointers.
//! Tuple results use a caller-owned destination, passed last at rbp+16.
use super::cfg;
use super::frontend::{self, BinaryOp, Program, UnaryOp};
use super::ir::{self, LocalId};
use std::collections::HashMap;

const MAX_ARGS: usize = 4096;
const MAX_FRAME_BYTES: i32 = 1024 * 1024;
const BASE: u64 = 0x400000;
const PAGE: usize = 4096;

fn value_size(ty: &ir::Ty) -> Result<i32, String> {
    match ty {
        ir::Ty::Tuple(fields) => fields.iter().try_fold(0_i32, |size, field| {
            size.checked_add(value_size(field)?)
                .filter(|size| *size <= MAX_FRAME_BYTES)
                .ok_or_else(|| "aggregate exceeds private frame limit".to_string())
        }),
        _ => Ok(8),
    }
}

fn field_offset(ty: &ir::Ty, index: usize) -> Result<(i32, &ir::Ty), String> {
    let ir::Ty::Tuple(fields) = ty else {
        return Err("invalid checked tuple projection".into());
    };
    let field = fields.get(index).ok_or("invalid checked tuple field")?;
    let offset = fields[..index].iter().try_fold(0_i32, |offset, ty| {
        offset
            .checked_add(value_size(ty)?)
            .ok_or_else(|| "aggregate layout too large".to_string())
    })?;
    Ok((offset, field))
}

fn allocate_value(frame: &mut i32, ty: &ir::Ty) -> Result<i32, String> {
    *frame = frame
        .checked_add(value_size(ty)?)
        .filter(|size| *size <= MAX_FRAME_BYTES)
        .ok_or("function exceeds private frame limit")?;
    Ok(-*frame)
}

/// Relocations refer to byte offsets within `code`: calls address a signed
/// rel32 displacement field, strings address an absolute u64 immediate field.
#[derive(Clone, Debug)]
pub struct Object {
    pub code: Vec<u8>,
    pub calls: Vec<(usize, String)>,
    pub strings: Vec<(usize, Vec<u8>)>,
}

struct Emitter<'a> {
    object: Object,
    function: &'a cfg::Function,
    locals: Vec<i32>,
    aggregate_scratch: i32,
    returns: Vec<usize>,
    traps: Vec<usize>,
    edges: Vec<(usize, cfg::BlockId)>,
}
impl<'a> Emitter<'a> {
    fn bytes(&mut self, bytes: &[u8]) {
        self.object.code.extend_from_slice(bytes);
    }
    fn i32(&mut self, value: i32) {
        self.bytes(&value.to_le_bytes());
    }
    fn position(&self) -> usize {
        self.object.code.len()
    }
    fn immediate(&mut self, value: i32) {
        self.bytes(&[0xb8]);
        self.i32(value);
    }
    fn jump(&mut self, opcode: &[u8]) -> usize {
        self.bytes(opcode);
        let at = self.position();
        self.i32(0);
        at
    }
    fn patch(&mut self, at: usize, target: usize) -> Result<(), String> {
        let displacement = i32::try_from(target as i128 - at as i128 - 4)
            .map_err(|_| "function exceeds rel32 range".to_string())?;
        self.object.code[at..at + 4].copy_from_slice(&displacement.to_le_bytes());
        Ok(())
    }
    fn edge(&mut self, opcode: &[u8], target: cfg::BlockId) {
        let at = self.jump(opcode);
        self.edges.push((at, target));
    }
    fn trap_if(&mut self, condition: u8) {
        let at = self.jump(&[0x0f, condition]);
        self.traps.push(at);
    }
    fn local_offset(&self, local: LocalId) -> Result<i32, String> {
        self.locals
            .get(local.0)
            .copied()
            .ok_or_else(|| "invalid CFG local".to_string())
    }
    fn place(&self, place: &ir::Place) -> Result<(i32, &'a ir::Ty), String> {
        let mut offset = self.local_offset(place.local)?;
        let mut ty: &'a ir::Ty = &self.function.locals[place.local.0].ty;
        for &index in &place.fields {
            let (field, next) = field_offset(ty, index)?;
            offset = offset
                .checked_add(field)
                .ok_or("projection offset too large")?;
            ty = next;
        }
        Ok((offset, ty))
    }
    fn address(&mut self, offset: i32) {
        self.bytes(&[0x48, 0x8d, 0x85]);
        self.i32(offset);
    }
    fn load(&mut self, offset: i32) {
        self.bytes(&[0x8b, 0x85]);
        self.i32(offset);
    }
    fn copy_pointer_value(&mut self, size: i32) {
        // All private scalar/unit leaves are initialized eight-byte words.
        for offset in (0..size).step_by(8) {
            self.bytes(&[0x48, 0x8b, 0x8e]);
            self.i32(offset);
            self.bytes(&[0x48, 0x89, 0x8f]);
            self.i32(offset);
        }
    }
    fn store_value(&mut self, offset: i32, ty: &ir::Ty) -> Result<(), String> {
        if matches!(ty, ir::Ty::Tuple(_)) {
            self.bytes(&[0x48, 0x89, 0xc6, 0x48, 0x8d, 0xbd]);
            self.i32(offset);
            self.copy_pointer_value(value_size(ty)?);
            self.address(offset);
        } else {
            self.bytes(&[0x48, 0x89, 0x85]);
            self.i32(offset);
        }
        Ok(())
    }
    fn operand(&mut self, operand: &cfg::Operand) -> Result<(), String> {
        match operand {
            cfg::Operand::Constant(value) => self.immediate(match value {
                cfg::Constant::I32(value) => *value,
                cfg::Constant::Bool(value) => i32::from(*value),
                cfg::Constant::Unit => 0,
            }),
            cfg::Operand::Copy(place) | cfg::Operand::Move(place) => {
                let (offset, ty) = self.place(place)?;
                if matches!(ty, ir::Ty::Tuple(_)) {
                    self.address(offset);
                } else {
                    self.load(offset);
                }
            }
        }
        Ok(())
    }
    fn binary(&mut self, op: BinaryOp) -> Result<(), String> {
        match op {
            BinaryOp::Add => {
                self.bytes(&[0x01, 0xc8]);
                self.trap_if(0x80);
            }
            BinaryOp::Sub => {
                self.bytes(&[0x29, 0xc8]);
                self.trap_if(0x80);
            }
            BinaryOp::Mul => {
                self.bytes(&[0x0f, 0xaf, 0xc1]);
                self.trap_if(0x80);
            }
            BinaryOp::Div | BinaryOp::Rem => {
                self.bytes(&[0x85, 0xc9]);
                self.trap_if(0x84);
                self.bytes(&[0x3d]);
                self.i32(i32::MIN);
                let ordinary = self.jump(&[0x0f, 0x85]);
                self.bytes(&[0x83, 0xf9, 0xff]);
                self.trap_if(0x84);
                self.patch(ordinary, self.position())?;
                self.bytes(&[0x99, 0xf7, 0xf9]);
                if matches!(op, BinaryOp::Rem) {
                    self.bytes(&[0x89, 0xd0]);
                }
            }
            BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => {
                let cc = match op {
                    BinaryOp::Eq => 0x94,
                    BinaryOp::Ne => 0x95,
                    BinaryOp::Lt => 0x9c,
                    BinaryOp::Le => 0x9e,
                    BinaryOp::Gt => 0x9f,
                    _ => 0x9d,
                };
                self.bytes(&[0x39, 0xc8, 0x0f, cc, 0xc0, 0x0f, 0xb6, 0xc0]);
            }
            // Source short-circuiting is already represented by CFG edges.
            BinaryOp::And => self.bytes(&[0x21, 0xc8]),
            BinaryOp::Or => self.bytes(&[0x09, 0xc8]),
        }
        Ok(())
    }
    fn println(&mut self, text: &str) -> Result<(), String> {
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(b'\n');
        self.bytes(&[0x48, 0xbe]);
        let at = self.position();
        self.bytes(&[0; 8]);
        let length = u64::try_from(bytes.len()).map_err(|_| "literal too large")?;
        self.object.strings.push((at, bytes));
        self.bytes(&[0x48, 0xba]);
        self.bytes(&length.to_le_bytes());
        let retry = self.position();
        self.bytes(&[0xb8, 1, 0, 0, 0, 0xbf, 1, 0, 0, 0, 0x0f, 0x05]);
        self.bytes(&[0x48, 0x83, 0xf8, 0xfc]);
        let interrupted = self.jump(&[0x0f, 0x84]);
        self.patch(interrupted, retry)?;
        self.bytes(&[0x48, 0x85, 0xc0]);
        self.trap_if(0x8e);
        self.bytes(&[0x48, 0x01, 0xc6, 0x48, 0x29, 0xc2]);
        let more = self.jump(&[0x0f, 0x85]);
        self.patch(more, retry)?;
        self.immediate(0);
        Ok(())
    }
    fn rvalue(&mut self, value: &cfg::Rvalue) -> Result<(), String> {
        match value {
            cfg::Rvalue::Use(operand) => self.operand(operand)?,
            cfg::Rvalue::Unary { op, value } => {
                self.operand(value)?;
                match op {
                    UnaryOp::Neg => {
                        self.bytes(&[0xf7, 0xd8]);
                        self.trap_if(0x80);
                    }
                    UnaryOp::Not => self.bytes(&[0x83, 0xf0, 0x01]),
                }
            }
            cfg::Rvalue::Binary { op, left, right } => {
                self.operand(left)?;
                self.bytes(&[0x41, 0x89, 0xc0]); // r8d = left
                self.operand(right)?;
                self.bytes(&[0x89, 0xc1, 0x44, 0x89, 0xc0]); // ecx = right, eax = left
                self.binary(*op)?;
            }
            cfg::Rvalue::Aggregate { ty, fields } => {
                // Evaluate the whole aggregate into scratch, so destination
                // projections cannot overwrite a later Copy operand's source.
                for (index, operand) in fields.iter().enumerate() {
                    let (field, field_ty) = field_offset(ty, index)?;
                    self.operand(operand)?;
                    let offset = self
                        .aggregate_scratch
                        .checked_add(field)
                        .ok_or("aggregate offset too large")?;
                    self.store_value(offset, field_ty)?;
                }
                self.address(self.aggregate_scratch);
            }
            cfg::Rvalue::Println(text) => self.println(text)?,
        }
        Ok(())
    }
    fn statement(&mut self, statement: &cfg::Statement) -> Result<(), String> {
        match statement {
            // Copy-only locals have fixed storage for the whole invocation.
            cfg::Statement::StorageLive(_) | cfg::Statement::StorageDead(_) => {}
            cfg::Statement::Assign { destination, value } => {
                self.rvalue(value)?;
                let (offset, ty) = self.place(destination)?;
                self.store_value(offset, ty)?;
            }
        }
        Ok(())
    }
    fn terminator(&mut self, terminator: &cfg::Terminator) -> Result<(), String> {
        match terminator {
            cfg::Terminator::Goto(target) => self.edge(&[0xe9], *target),
            cfg::Terminator::Switch {
                condition,
                then_block,
                else_block,
            } => {
                self.operand(condition)?;
                self.bytes(&[0x85, 0xc0]);
                self.edge(&[0x0f, 0x85], *then_block);
                self.edge(&[0xe9], *else_block);
            }
            cfg::Terminator::Call {
                name,
                signature,
                args,
                destination,
                next,
            } => {
                if args.len() > MAX_ARGS {
                    return Err(format!("at most {MAX_ARGS} arguments are supported"));
                }
                for operand in args {
                    self.operand(operand)?;
                    self.bytes(&[0x50]);
                }
                let aggregate = matches!(signature.result, ir::Ty::Tuple(_));
                let (offset, ty) = self.place(destination)?;
                if aggregate {
                    self.address(offset);
                    self.bytes(&[0x50]);
                }
                let at = self.jump(&[0xe8]);
                self.object.calls.push((at, name.clone()));
                let words = args.len() + usize::from(aggregate);
                if words != 0 {
                    self.bytes(&[0x48, 0x81, 0xc4]);
                    self.i32((words * 8) as i32);
                }
                if aggregate {
                    self.address(offset);
                } else {
                    self.store_value(offset, ty)?;
                }
                self.edge(&[0xe9], *next);
            }
            cfg::Terminator::Return => {
                self.operand(&cfg::Operand::Copy(ir::Place {
                    local: self.function.return_local,
                    fields: Vec::new(),
                }))?;
                let at = self.jump(&[0xe9]);
                self.returns.push(at);
            }
            cfg::Terminator::Unreachable => self.bytes(&[0x0f, 0x0b]),
        }
        Ok(())
    }
}

/// Emit an already-lowered immutable CFG without source or semantic rechecking.
/// Every local and lowering temporary has stable invocation-owned frame storage.
pub fn emit_cfg_function(function: &cfg::Function) -> Result<Object, String> {
    if function.params.len() > MAX_ARGS {
        return Err(format!("at most {MAX_ARGS} parameters are supported"));
    }
    let mut frame_bytes = 0;
    let locals = function
        .locals
        .iter()
        .map(|local| allocate_value(&mut frame_bytes, &local.ty))
        .collect::<Result<Vec<_>, _>>()?;
    let mut scratch_size = 0;
    for block in &function.blocks {
        for statement in &block.statements {
            if let cfg::Statement::Assign {
                value: cfg::Rvalue::Aggregate { ty, .. },
                ..
            } = &statement.value
            {
                scratch_size = scratch_size.max(value_size(ty)?);
            }
        }
    }
    frame_bytes = frame_bytes
        .checked_add(scratch_size)
        .filter(|size| *size <= MAX_FRAME_BYTES)
        .ok_or("function exceeds private frame limit")?;
    let aggregate_scratch = -frame_bytes;
    let aggregate_result = matches!(function.result, ir::Ty::Tuple(_));
    let result_size = value_size(&function.result)?;
    let mut emitter = Emitter {
        object: Object {
            code: Vec::new(),
            calls: Vec::new(),
            strings: Vec::new(),
        },
        function,
        locals,
        aggregate_scratch,
        returns: Vec::new(),
        traps: Vec::new(),
        edges: Vec::new(),
    };
    emitter.bytes(&[0x55, 0x48, 0x89, 0xe5, 0x48, 0x81, 0xec]);
    emitter.i32(frame_bytes);
    for (index, local) in function.params.iter().enumerate() {
        let offset =
            16 + usize::from(aggregate_result) * 8 + (function.params.len() - index - 1) * 8;
        emitter.bytes(&[0x48, 0x8b, 0x85]);
        emitter.i32(offset as i32);
        emitter.store_value(emitter.local_offset(*local)?, &function.locals[local.0].ty)?;
    }
    emitter.edge(&[0xe9], function.entry);
    let mut labels = Vec::with_capacity(function.blocks.len());
    for block in &function.blocks {
        labels.push(emitter.position());
        for statement in &block.statements {
            emitter.statement(&statement.value)?;
        }
        emitter.terminator(&block.terminator.value)?;
    }
    for (at, target) in std::mem::take(&mut emitter.edges) {
        emitter.patch(at, *labels.get(target.0).ok_or("invalid CFG block target")?)?;
    }
    let epilogue = emitter.position();
    if aggregate_result {
        emitter.bytes(&[0x48, 0x89, 0xc6, 0x48, 0x8b, 0xbd]);
        emitter.i32(16);
        emitter.copy_pointer_value(result_size);
        emitter.bytes(&[0x48, 0x89, 0xf8]);
    }
    emitter.bytes(&[0xc9, 0xc3]);
    for at in std::mem::take(&mut emitter.returns) {
        emitter.patch(at, epilogue)?;
    }
    let trap = emitter.position();
    emitter.bytes(&[
        0xbf, 101, 0, 0, 0, 0xb8, 60, 0, 0, 0, 0x0f, 0x05, 0x0f, 0x0b,
    ]);
    for at in std::mem::take(&mut emitter.traps) {
        emitter.patch(at, trap)?;
    }
    Ok(emitter.object)
}

/// Compatibility entry point for direct checked-body callers. Graph compilation
/// owns the lowered CFG fact and calls emit_cfg_function without lowering again.
pub fn emit_checked_function(function: &ir::CheckedFunction) -> Result<Object, String> {
    emit_cfg_function(&cfg::lower(function)?)
}

fn put16(bytes: &mut [u8], at: usize, n: u16) {
    bytes[at..at + 2].copy_from_slice(&n.to_le_bytes());
}
fn put32(bytes: &mut [u8], at: usize, n: u32) {
    bytes[at..at + 4].copy_from_slice(&n.to_le_bytes());
}
fn put64(bytes: &mut [u8], at: usize, n: u64) {
    bytes[at..at + 8].copy_from_slice(&n.to_le_bytes());
}
fn page_align(n: usize) -> Result<usize, String> {
    n.checked_add(PAGE - 1)
        .map(|n| n & !(PAGE - 1))
        .ok_or("ELF too large".into())
}
fn segment(bytes: &mut [u8], index: usize, kind: u32, flags: u32, offset: usize, size: usize) {
    let at = 64 + index * 56;
    put32(bytes, at, kind);
    put32(bytes, at + 4, flags);
    put64(bytes, at + 8, offset as u64);
    put64(bytes, at + 16, BASE + offset as u64);
    put64(bytes, at + 24, BASE + offset as u64);
    put64(bytes, at + 32, size as u64);
    put64(bytes, at + 40, size as u64);
    put64(bytes, at + 48, PAGE as u64);
}

/// Link private objects into a fixed-address ET_EXEC with separate R headers,
/// RX code and R literals. GNU_STACK explicitly requests a non-executable stack.
pub fn link(functions: &[(String, Object)]) -> Result<Vec<u8>, String> {
    let mut code = vec![
        0xe8, 0, 0, 0, 0, 0x31, 0xff, 0xb8, 60, 0, 0, 0, 0x0f, 0x05, 0x0f, 0x0b,
    ];
    let mut symbols = HashMap::new();
    for (name, object) in functions {
        if symbols.insert(name.as_str(), code.len()).is_some() {
            return Err(format!("duplicate function `{name}`"));
        }
        code.extend_from_slice(&object.code);
    }
    let main = *symbols.get("main").ok_or("missing main function")?;
    let main_displacement =
        i32::try_from(main as i128 - 5).map_err(|_| "main outside rel32 range")?;
    code[1..5].copy_from_slice(&main_displacement.to_le_bytes());
    let data_offset = page_align(PAGE.checked_add(code.len()).ok_or("ELF too large")?)?;
    let mut data = Vec::new();
    for (name, object) in functions {
        let start = symbols[name.as_str()];
        for (at, callee) in &object.calls {
            if at.checked_add(4).is_none_or(|end| end > object.code.len()) {
                return Err("invalid call relocation".into());
            }
            let target = *symbols
                .get(callee.as_str())
                .ok_or_else(|| format!("unknown function `{callee}`"))?;
            let delta = i32::try_from(target as i128 - (start + at + 4) as i128)
                .map_err(|_| "call outside rel32 range")?;
            code[start + at..start + at + 4].copy_from_slice(&delta.to_le_bytes());
        }
        for (at, literal) in &object.strings {
            if at.checked_add(8).is_none_or(|end| end > object.code.len()) {
                return Err("invalid string relocation".into());
            }
            let address = BASE
                .checked_add(data_offset as u64)
                .and_then(|a| a.checked_add(data.len() as u64))
                .ok_or("ELF address overflow")?;
            code[start + at..start + at + 8].copy_from_slice(&address.to_le_bytes());
            data.extend_from_slice(literal);
        }
    }
    let length = data_offset.checked_add(data.len()).ok_or("ELF too large")?;
    let mut elf = vec![0; length];
    elf[..16].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    put16(&mut elf, 16, 2);
    put16(&mut elf, 18, 62);
    put32(&mut elf, 20, 1);
    put64(&mut elf, 24, BASE + PAGE as u64);
    put64(&mut elf, 32, 64);
    put16(&mut elf, 52, 64);
    put16(&mut elf, 54, 56);
    put16(&mut elf, 56, 4);
    segment(&mut elf, 0, 1, 4, 0, PAGE);
    segment(&mut elf, 1, 1, 5, PAGE, code.len());
    segment(&mut elf, 2, 1, 4, data_offset, data.len());
    segment(&mut elf, 3, 0x6474e551, 6, 0, 0);
    elf[PAGE..PAGE + code.len()].copy_from_slice(&code);
    elf[data_offset..].copy_from_slice(&data);
    Ok(elf)
}

/// Compatibility entry point for AST callers. Build signatures once and lower
/// each body once, then use the same checked emitter as graph compilation.
pub fn emit(program: &Program) -> Result<Vec<u8>, String> {
    let signatures = frontend::signature_table(program)?;
    let objects = program
        .functions
        .iter()
        .enumerate()
        .map(|(index, f)| {
            let checked = frontend::type_function_with_signatures(program, index, &signatures)?;
            Ok((f.name.clone(), emit_checked_function(&checked)?))
        })
        .collect::<Result<Vec<_>, String>>()?;
    link(&objects)
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::super::frontend::{Block, Expr, Function, Stmt, Type};
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    use std::process::{Command, Output, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn block(statements: Vec<Stmt>, tail: Option<Expr>) -> Block {
        Block {
            statements,
            tail: tail.map(Box::new),
        }
    }
    fn function(name: &str, params: Vec<(String, Type)>, result: Type, body: Block) -> Function {
        Function {
            name: name.into(),
            params,
            result,
            body,
        }
    }
    fn binary(op: BinaryOp, left: Expr, right: Expr) -> Expr {
        Expr::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        }
    }
    fn var(name: &str) -> Expr {
        Expr::Var(name.into())
    }
    fn call(name: &str, args: Vec<Expr>) -> Expr {
        Expr::Call {
            name: name.into(),
            args,
        }
    }
    fn crash() -> Expr {
        binary(BinaryOp::Div, Expr::Int(1), Expr::Int(0))
    }
    fn assertion(condition: Expr) -> Stmt {
        Stmt::Expr(Expr::If {
            condition: Box::new(condition),
            then_block: block(vec![], None),
            else_block: block(vec![Stmt::Expr(crash())], None),
        })
    }
    fn bool_equals(value: Expr, expected: bool) -> Stmt {
        assertion(binary(BinaryOp::Eq, value, Expr::Bool(expected)))
    }
    fn crash_bool() -> Expr {
        binary(BinaryOp::Eq, crash(), Expr::Int(0))
    }
    fn equals(value: Expr, expected: i32) -> Stmt {
        assertion(binary(BinaryOp::Eq, value, Expr::Int(expected)))
    }
    fn program(statements: Vec<Stmt>, mut others: Vec<Function>) -> Program {
        others.push(function(
            "main",
            vec![],
            Type::Unit,
            block(statements, None),
        ));
        Program { functions: others }
    }
    fn executable(program: &Program) -> std::path::PathBuf {
        executable_bytes(&emit(program).unwrap())
    }
    fn executable_bytes(bytes: &[u8]) -> std::path::PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(std::path::PathBuf::from)
            .filter(|path| path.is_dir())
            .unwrap_or_else(std::env::temp_dir);
        let directory = root.join(format!(
            "jr-native-backend-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        let path = directory.join("program");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&path)
            .unwrap();
        file.write_all(bytes).unwrap();
        drop(file);
        path
    }
    fn run(program: &Program) -> Output {
        let path = executable(program);
        let output = execute(Command::new(&path).stdout(Stdio::piped()));
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
        output
    }

    fn execute(command: &mut Command) -> Output {
        command.stdin(Stdio::null()).stderr(Stdio::piped());
        // A concurrent test's fork can briefly inherit the writer's descriptor
        // before its CLOEXEC takes effect. Linux returns ETXTBSY in that window.
        for _ in 0..100 {
            match command.spawn() {
                Ok(mut child) => {
                    fn drain<R: Read + Send + 'static>(
                        reader: Option<R>,
                    ) -> std::thread::JoinHandle<Vec<u8>> {
                        std::thread::spawn(move || {
                            let mut output = Vec::new();
                            if let Some(mut reader) = reader {
                                reader.read_to_end(&mut output).unwrap();
                            }
                            output
                        })
                    }
                    let stdout = drain(child.stdout.take());
                    let stderr = drain(child.stderr.take());
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                    let (status, timed_out) = loop {
                        if let Some(status) = child.try_wait().unwrap() {
                            break (status, false);
                        }
                        if std::time::Instant::now() >= deadline {
                            child.kill().unwrap();
                            break (child.wait().unwrap(), true);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    };
                    let output = Output {
                        status,
                        stdout: stdout.join().unwrap(),
                        stderr: stderr.join().unwrap(),
                    };
                    assert!(!timed_out, "native executable timed out: {output:?}");
                    return output;
                }
                Err(error) if error.raw_os_error() == Some(26) => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(error) => panic!("cannot execute native test: {error}"),
            }
        }
        panic!("native executable remained busy after bounded retries");
    }

    #[test]
    fn checked_arithmetic_traps() {
        let cases = vec![
            binary(BinaryOp::Add, Expr::Int(i32::MAX), Expr::Int(1)),
            binary(BinaryOp::Sub, Expr::Int(i32::MIN), Expr::Int(1)),
            binary(BinaryOp::Mul, Expr::Int(i32::MAX), Expr::Int(2)),
            Expr::Unary {
                op: UnaryOp::Neg,
                value: Box::new(Expr::Int(i32::MIN)),
            },
            crash(),
            binary(BinaryOp::Rem, Expr::Int(1), Expr::Int(0)),
            binary(BinaryOp::Div, Expr::Int(i32::MIN), Expr::Int(-1)),
            binary(BinaryOp::Rem, Expr::Int(i32::MIN), Expr::Int(-1)),
        ];
        for expression in cases {
            let output = run(&program(vec![Stmt::Expr(expression)], vec![]));
            assert_eq!(output.status.code(), Some(101), "{output:?}");
        }
    }

    #[test]
    fn arithmetic_comparisons_and_short_circuit() {
        let mut statements = vec![
            equals(binary(BinaryOp::Add, Expr::Int(7), Expr::Int(-3)), 4),
            equals(binary(BinaryOp::Sub, Expr::Int(7), Expr::Int(-3)), 10),
            equals(binary(BinaryOp::Mul, Expr::Int(-7), Expr::Int(3)), -21),
            equals(binary(BinaryOp::Div, Expr::Int(-7), Expr::Int(3)), -2),
            equals(binary(BinaryOp::Rem, Expr::Int(-7), Expr::Int(3)), -1),
            equals(
                binary(BinaryOp::Div, Expr::Int(i32::MIN), Expr::Int(1)),
                i32::MIN,
            ),
            equals(
                Expr::Unary {
                    op: UnaryOp::Neg,
                    value: Box::new(Expr::Int(7)),
                },
                -7,
            ),
            bool_equals(
                Expr::Unary {
                    op: UnaryOp::Not,
                    value: Box::new(Expr::Bool(true)),
                },
                false,
            ),
            bool_equals(
                binary(BinaryOp::And, Expr::Bool(false), crash_bool()),
                false,
            ),
            bool_equals(binary(BinaryOp::Or, Expr::Bool(true), crash_bool()), true),
            bool_equals(
                binary(BinaryOp::And, Expr::Bool(true), Expr::Bool(false)),
                false,
            ),
            bool_equals(
                binary(BinaryOp::Or, Expr::Bool(false), Expr::Bool(true)),
                true,
            ),
        ];
        for (op, expected) in [
            (BinaryOp::Eq, 0),
            (BinaryOp::Ne, 1),
            (BinaryOp::Lt, 1),
            (BinaryOp::Le, 1),
            (BinaryOp::Gt, 0),
            (BinaryOp::Ge, 0),
        ] {
            statements.push(bool_equals(
                binary(op, Expr::Int(-7), Expr::Int(3)),
                expected != 0,
            ));
        }
        let output = run(&program(statements, vec![]));
        assert_eq!(output.status.code(), Some(0), "{output:?}");
    }

    #[test]
    fn recursion_many_arguments_returns_and_stable_scopes() {
        let factorial = function(
            "factorial",
            vec![("n".into(), Type::I32)],
            Type::I32,
            block(
                vec![],
                Some(Expr::If {
                    condition: Box::new(binary(BinaryOp::Le, var("n"), Expr::Int(1))),
                    then_block: block(vec![Stmt::Return(Some(Expr::Int(1)))], None),
                    else_block: block(
                        vec![],
                        Some(binary(
                            BinaryOp::Mul,
                            var("n"),
                            call(
                                "factorial",
                                vec![binary(BinaryOp::Sub, var("n"), Expr::Int(1))],
                            ),
                        )),
                    ),
                }),
            ),
        );
        let pick = function(
            "pick",
            (0..12).map(|i| (format!("p{i}"), Type::I32)).collect(),
            Type::I32,
            block(vec![], Some(binary(BinaryOp::Sub, var("p0"), var("p11")))),
        );
        let local = |name: &str, n| Stmt::Let {
            name: name.into(),
            mutable: true,
            ty: Type::I32,
            value: Expr::Int(n),
        };
        let statements = vec![
            local("x", 3),
            equals(
                binary(
                    BinaryOp::Add,
                    Expr::Int(2),
                    Expr::Block(block(vec![local("x", 9)], Some(var("x")))),
                ),
                11,
            ),
            equals(var("x"), 3),
            Stmt::While {
                condition: binary(BinaryOp::Lt, var("x"), Expr::Int(6)),
                body: block(
                    vec![
                        Stmt::Assign {
                            name: "x".into(),
                            value: binary(BinaryOp::Add, var("x"), Expr::Int(1)),
                        },
                        local("temporary", 100),
                    ],
                    None,
                ),
            },
            equals(var("x"), 6),
            equals(call("factorial", vec![Expr::Int(6)]), 720),
            equals(call("pick", (0..12).map(Expr::Int).collect()), -11),
        ];
        let output = run(&program(statements, vec![factorial, pick]));
        assert_eq!(output.status.code(), Some(0), "{output:?}");
    }

    #[test]
    fn literal_writes_and_write_failures() {
        let p = program(
            vec![
                Stmt::Expr(Expr::Println("Hello, 世界 🦀".into())),
                Stmt::Expr(Expr::Println(String::new())),
            ],
            vec![],
        );
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(output.stdout, "Hello, 世界 🦀\n\n".as_bytes());
        let path = executable(&p);
        let full = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .unwrap();
        let status = execute(Command::new(&path).stdout(Stdio::from(full))).status;
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
        assert_eq!(status.code(), Some(101));
    }

    #[test]
    fn argument_bound_and_early_return_with_temporaries() {
        let pick = function(
            "pick",
            (0..MAX_ARGS)
                .map(|i| (format!("p{i}"), Type::I32))
                .collect(),
            Type::I32,
            block(vec![], Some(binary(BinaryOp::Sub, var("p0"), var("p4095")))),
        );
        let early = function(
            "early",
            vec![],
            Type::I32,
            block(
                vec![],
                Some(binary(
                    BinaryOp::Add,
                    Expr::Int(99),
                    Expr::Block(block(
                        vec![Stmt::Return(Some(Expr::Int(7)))],
                        Some(Expr::Int(0)),
                    )),
                )),
            ),
        );
        let p = program(
            vec![
                equals(
                    call("pick", (0..MAX_ARGS as i32).map(Expr::Int).collect()),
                    -4095,
                ),
                equals(
                    binary(BinaryOp::Add, Expr::Int(1), call("early", vec![])),
                    8,
                ),
            ],
            vec![pick.clone(), early],
        );
        assert_eq!(run(&p).status.code(), Some(0));
        let mut too_many = pick;
        too_many.params.push(("extra".into(), Type::I32));
        assert!(
            emit(&program(vec![], vec![too_many]))
                .unwrap_err()
                .contains("4096")
        );
        assert!(
            emit(&program(
                vec![Stmt::Expr(call("pick", vec![Expr::Int(0); MAX_ARGS + 1]))],
                vec![p.functions[0].clone()]
            ))
            .unwrap_err()
            .contains("4096")
        );
    }

    #[test]
    fn unit_arguments_nested_calls_and_left_to_right_effects() {
        let unit = function(
            "unit",
            vec![],
            Type::Unit,
            block(vec![Stmt::Expr(Expr::Println("unit".into()))], None),
        );
        let identity = function(
            "identity",
            vec![("n".into(), Type::I32)],
            Type::I32,
            block(vec![], Some(var("n"))),
        );
        let combine = function(
            "combine",
            vec![
                ("a".into(), Type::I32),
                ("u".into(), Type::Unit),
                ("b".into(), Type::I32),
            ],
            Type::I32,
            block(
                vec![],
                Some(binary(
                    BinaryOp::Add,
                    binary(BinaryOp::Mul, var("a"), Expr::Int(10)),
                    var("b"),
                )),
            ),
        );
        let early = function(
            "early",
            vec![],
            Type::I32,
            block(
                vec![],
                Some(call(
                    "combine",
                    vec![
                        Expr::Int(1),
                        Expr::Block(block(vec![Stmt::Return(Some(Expr::Int(9)))], None)),
                        Expr::Int(3),
                    ],
                )),
            ),
        );
        let effect = |label: &str, value| {
            call(
                "identity",
                vec![Expr::Block(block(
                    vec![Stmt::Expr(Expr::Println(label.into()))],
                    Some(Expr::Int(value)),
                ))],
            )
        };
        let p = program(
            vec![
                equals(
                    binary(
                        BinaryOp::Add,
                        call(
                            "combine",
                            vec![effect("first", 2), call("unit", vec![]), effect("last", 3)],
                        ),
                        call("identity", vec![Expr::Int(4)]),
                    ),
                    27,
                ),
                equals(
                    binary(BinaryOp::Mul, Expr::Int(2), call("early", vec![])),
                    18,
                ),
            ],
            vec![unit, identity, combine, early],
        );
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"first\nunit\nlast\n");
    }

    #[test]
    fn large_literal_drains_without_pipe_deadlock() {
        let literal = "🦀abcdefgh".repeat(32 * 1024);
        let output = run(&program(
            vec![Stmt::Expr(Expr::Println(literal.clone()))],
            vec![],
        ));
        assert_eq!(output.status.code(), Some(0));
        let mut expected = literal.into_bytes();
        expected.push(b'\n');
        assert_eq!(output.stdout, expected);
    }

    #[test]
    fn elf_permissions_and_relocation_errors() {
        let p = program(vec![], vec![]);
        let bytes = emit(&p).unwrap();
        assert_eq!(&bytes[..4], b"\x7fELF");
        for index in 0..4 {
            let flags =
                u32::from_le_bytes(bytes[68 + index * 56..72 + index * 56].try_into().unwrap());
            assert_ne!(flags & 3, 3, "writable executable segment");
        }
        assert_eq!(run(&p).status.code(), Some(0));
        assert!(link(&[]).is_err());
        let signatures = frontend::signature_table(&p).unwrap();
        let checked = frontend::type_function_with_signatures(&p, 0, &signatures).unwrap();
        let object = emit_checked_function(&checked).unwrap();
        assert!(link(&[("main".into(), object.clone()), ("main".into(), object)]).is_err());
        assert!(emit(&program(vec![Stmt::Expr(call("missing", vec![]))], vec![])).is_err());
    }

    #[test]
    fn checked_emission_uses_local_ids_not_display_names() {
        let p = program(
            vec![equals(call("shadow", vec![Expr::Int(17)]), 19)],
            vec![function(
                "shadow",
                vec![("x".into(), Type::I32)],
                Type::I32,
                block(
                    vec![Stmt::Let {
                        name: "x".into(),
                        mutable: false,
                        ty: Type::I32,
                        value: binary(BinaryOp::Add, var("x"), Expr::Int(2)),
                    }],
                    Some(var("x")),
                ),
            )],
        );
        assert_eq!(run(&p).status.code(), Some(0));
        let signatures = frontend::signature_table(&p).unwrap();
        let mut checked = frontend::type_function_with_signatures(&p, 0, &signatures).unwrap();
        let original = emit_checked_function(&checked).unwrap();
        for local in &mut checked.locals {
            local.name = "not_a_source_binding".into();
        }
        let renamed = emit_checked_function(&checked).unwrap();
        assert_eq!(original.code, renamed.code);
        assert_eq!(original.calls, renamed.calls);
        assert_eq!(original.strings, renamed.strings);
        // Both the parameter and shadow binding own a distinct monotonic slot.
        let frame = i32::from_le_bytes(original.code[7..11].try_into().unwrap());
        assert!(frame >= 16 && frame % 8 == 0);
        assert!(
            original
                .code
                .windows(6)
                .any(|bytes| bytes == [0x89, 0x85, 0xf8, 0xff, 0xff, 0xff])
        );
        assert!(
            original
                .code
                .windows(6)
                .any(|bytes| bytes == [0x89, 0x85, 0xf0, 0xff, 0xff, 0xff])
        );
    }

    #[test]
    fn loop_values_preserve_outer_operands_and_prior_call_arguments() {
        let p = frontend::parse(
            r#"
            fn pair(a: i32, b: i32) -> i32 { a * 10 + b }
            fn with_unit(a: (), b: i32) -> i32 { b }
            fn main() {
                let x = 100 + loop { break 7 + loop { break 2; }; };
                if x != 109 { 1 / 0; }
                let y = pair(3, loop { break pair(4, loop { break 5; }); });
                if y != 75 { 1 / 0; }
                let z = loop { break { break 11; }; };
                if z != 11 { 1 / 0; }
                let b = loop { break true; };
                if !b { 1 / 0; }
                loop { break; }
                let u = with_unit(loop { break println!("unit"); }, 8);
                if u != 8 { 1 / 0; }
                println!("values");
            }
            "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"unit\nvalues\n");
    }

    #[test]
    fn loop_jumps_discard_only_target_pending_temporaries() {
        let p = frontend::parse(
            r#"
            fn pair(a: i32, b: i32) -> i32 { a * 10 + b }
            fn work() -> i32 {
                let mut n = 0;
                let value = 50 + loop {
                    n = n + 1;
                    if n < 3 { pair(100, { continue; }); }
                    pair(200, { break 4; });
                };
                if n != 3 { 1 / 0; }
                value
            }
            fn main() {
                if work() != 54 { 1 / 0; }
                let mut n = 0;
                let value = loop {
                    n = n + 1;
                    if n < 3 { 90 + { continue; 0 }; }
                    80 + { break 6; 0 };
                };
                if value != 6 { 1 / 0; }
                if n != 3 { 1 / 0; }
                let mut turns = 0;
                let nested = loop {
                    turns = turns + 1;
                    let inner = 10 + loop { break 2; };
                    if inner != 12 { 1 / 0; }
                    if turns == 2 { break inner; }
                    continue;
                };
                if nested != 12 { 1 / 0; }
                println!("cleanup");
            }
            "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"cleanup\n");
    }

    #[test]
    fn while_nested_condition_loops_and_body_jumps_preserve_temporaries() {
        let p = frontend::parse(
            r#"
            fn pair(a: i32, b: i32) -> i32 { a + b }
            fn main() {
                let mut n = 0;
                let mut bodies = 0;
                let result = 100 + {
                    while {
                        n = n + 1;
                        let ready = loop {
                            if n < 3 {
                                n = n + 1;
                                pair(30, { continue; });
                            }
                            99 + { break n < 5; 0 };
                        };
                        ready
                    } {
                        bodies = bodies + 1;
                    }
                    7
                };
                if result != 107 { 1 / 0; }
                if n != 5 { 1 / 0; }
                if bodies != 2 { 1 / 0; }
                let mut k = 0;
                while k < 4 {
                    k = k + 1;
                    if k < 4 { pair(10, { continue; }); }
                    20 + { break; 0 };
                }
                if k != 4 { 1 / 0; }
                println!("while");
            }
            "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"while\n");
    }

    #[test]
    fn tuples_snapshot_fields_arguments_and_local_assignment() {
        let p = frontend::parse(
            r#"
            fn check(a: ((i32, bool), ((), i32)), b: i32) -> i32 {
                if a.0.0 != 1 { 1 / 0; }
                if !a.0.1 { 1 / 0; }
                if a.1.1 != 3 { 1 / 0; }
                let mut own = a;
                own.0.0 = 99;
                own.1.0 = ();
                b
            }
            fn main() {
                let mut t = ((1, true), ((), 3));
                let old = t;
                let pair = (t, { t.0.0 = 8; t.0.1 = false; 9 });
                if pair.0.0.0 != 1 { 1 / 0; }
                if !pair.0.0.1 { 1 / 0; }
                if pair.1 != 9 { 1 / 0; }
                t = old;
                let answer = check(t, { t.1.1 = 7; 11 });
                if answer != 11 { 1 / 0; }
                if t.0.0 != 1 { 1 / 0; }
                if t.1.1 != 7 { 1 / 0; }
                t.0 = { t.1.1 = 12; old.0 };
                if t.0.0 != 1 { 1 / 0; }
                if t.1.1 != 12 { 1 / 0; }
                let single = (42,);
                if single.0 != 42 { 1 / 0; }
                println!("snapshots");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"snapshots\n");
    }

    #[test]
    fn tuple_returns_recursion_branch_blocks_and_break_values() {
        let p = frontend::parse(
            r#"
            fn descend(n: i32, t: (i32, (bool, ()))) -> (i32, (bool, ())) {
                if n == 0 { return t; }
                descend(n - 1, (t.0 + 1, t.1))
            }
            fn make(n: i32) -> (i32, (bool, ())) {
                let mut own = (n, (true, ()));
                let result = if n > 0 { own } else { (0, (false, ())) };
                own.0 = 99;
                result
            }
            fn consume(a: (i32, (bool, ())), b: (i32, (bool, ())), c: i32) -> i32 {
                a.0 * 100 + b.0 * 10 + c
            }
            fn main() {
                let t = descend(4, make(2));
                if t.0 != 6 { 1 / 0; }
                if !t.1.0 { 1 / 0; }
                if make(5).0 != 5 { 1 / 0; }
                if ({ make(3) }).0 != 3 { 1 / 0; }
                let mut turns = 0;
                let value = consume(make(1), loop {
                    turns = turns + 1;
                    if turns < 3 { consume(make(8), { continue; }, 0); }
                    consume(make(9), { break make(2); }, 0);
                }, 3);
                if value != 123 { 1 / 0; }
                let projected = (loop { break (make(7), true); }).0.0;
                if projected != 7 { 1 / 0; }
                println!("returns");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"returns\n");
        let signatures = frontend::signature_table(&p).unwrap();
        let index = p.functions.iter().position(|f| f.name == "make").unwrap();
        let checked = frontend::type_function_with_signatures(&p, index, &signatures).unwrap();
        let object = emit_checked_function(&checked).unwrap();
        assert!(
            object
                .code
                .windows(5)
                .any(|bytes| bytes == [0x48, 0x89, 0xf8, 0xc9, 0xc3]),
            "aggregate epilogue must return the caller destination pointer"
        );
    }

    #[test]
    fn aggregate_layout_and_frame_limits_are_checked() {
        use std::sync::Arc;
        let mixed = ir::Ty::Tuple(Arc::from([
            ir::Ty::Bool,
            ir::Ty::Tuple(Arc::from([ir::Ty::Unit, ir::Ty::I32])),
        ]));
        assert_eq!(value_size(&mixed).unwrap(), 24);
        assert_eq!(field_offset(&mixed, 1).unwrap().0, 8);
        let mut frame = MAX_FRAME_BYTES - 8;
        assert_eq!(
            allocate_value(&mut frame, &ir::Ty::Unit).unwrap(),
            -MAX_FRAME_BYTES
        );
        assert!(allocate_value(&mut frame, &ir::Ty::Bool).is_err());
        let too_large = ir::Ty::Tuple(vec![ir::Ty::I32; MAX_FRAME_BYTES as usize / 8 + 1].into());
        assert!(value_size(&too_large).is_err());
        let checked = ir::CheckedFunction {
            name: "oversized".into(),
            params: vec![],
            result: ir::Ty::Unit,
            locals: vec![ir::LocalDecl {
                name: "t".into(),
                ty: too_large,
                mutable: false,
            }],
            exprs: vec![],
            body: ir::Block {
                statements: vec![],
                tail: None,
            },
        };
        assert!(
            emit_checked_function(&checked)
                .unwrap_err()
                .contains("limit")
        );
    }

    #[test]
    fn cfg_entry_edges_and_aggregate_assignment_snapshot_reads() {
        fn located<T>(value: T) -> cfg::Located<T> {
            cfg::Located {
                value,
                origin: None,
            }
        }
        fn place(local: usize, fields: &[usize]) -> ir::Place {
            ir::Place {
                local: LocalId(local),
                fields: fields.to_vec(),
            }
        }
        fn constant(value: i32) -> cfg::Operand {
            cfg::Operand::Constant(cfg::Constant::I32(value))
        }
        fn assign(destination: ir::Place, value: cfg::Rvalue) -> cfg::Located<cfg::Statement> {
            located(cfg::Statement::Assign { destination, value })
        }
        let pair = ir::Ty::Tuple(vec![ir::Ty::I32, ir::Ty::I32].into());
        let mut function = cfg::Function {
            name: "main".into(),
            params: vec![],
            result: ir::Ty::Unit,
            locals: vec![
                ir::LocalDecl {
                    name: "result".into(),
                    ty: ir::Ty::Unit,
                    mutable: true,
                },
                ir::LocalDecl {
                    name: "pair".into(),
                    ty: pair.clone(),
                    mutable: true,
                },
                ir::LocalDecl {
                    name: "condition".into(),
                    ty: ir::Ty::Bool,
                    mutable: true,
                },
            ],
            entry: cfg::BlockId(1),
            return_local: LocalId(0),
            blocks: vec![
                cfg::BasicBlock {
                    statements: vec![],
                    terminator: located(cfg::Terminator::Unreachable),
                },
                cfg::BasicBlock {
                    statements: vec![
                        located(cfg::Statement::StorageLive(LocalId(1))),
                        assign(
                            place(1, &[]),
                            cfg::Rvalue::Aggregate {
                                ty: pair.clone(),
                                fields: vec![constant(1), constant(2)],
                            },
                        ),
                    ],
                    terminator: located(cfg::Terminator::Goto(cfg::BlockId(3))),
                },
                cfg::BasicBlock {
                    statements: vec![
                        located(cfg::Statement::StorageDead(LocalId(1))),
                        assign(place(0, &[]), cfg::Rvalue::Println("cfg".into())),
                    ],
                    terminator: located(cfg::Terminator::Return),
                },
                cfg::BasicBlock {
                    statements: vec![
                        // A direct aggregate assignment must snapshot both
                        // operands before destination field zero is replaced.
                        assign(
                            place(1, &[]),
                            cfg::Rvalue::Aggregate {
                                ty: pair,
                                fields: vec![
                                    cfg::Operand::Copy(place(1, &[1])),
                                    cfg::Operand::Move(place(1, &[0])),
                                ],
                            },
                        ),
                        assign(
                            place(2, &[]),
                            cfg::Rvalue::Binary {
                                op: BinaryOp::Eq,
                                left: cfg::Operand::Copy(place(1, &[0])),
                                right: constant(2),
                            },
                        ),
                    ],
                    terminator: located(cfg::Terminator::Switch {
                        condition: cfg::Operand::Copy(place(2, &[])),
                        then_block: cfg::BlockId(4),
                        else_block: cfg::BlockId(0),
                    }),
                },
                cfg::BasicBlock {
                    statements: vec![assign(
                        place(2, &[]),
                        cfg::Rvalue::Binary {
                            op: BinaryOp::Eq,
                            left: cfg::Operand::Copy(place(1, &[1])),
                            right: constant(1),
                        },
                    )],
                    terminator: located(cfg::Terminator::Switch {
                        condition: cfg::Operand::Copy(place(2, &[])),
                        then_block: cfg::BlockId(2),
                        else_block: cfg::BlockId(0),
                    }),
                },
            ],
        };
        let object = emit_cfg_function(&function).unwrap();
        let bytes = link(&[("main".into(), object.clone())]).unwrap();
        let path = executable_bytes(&bytes);
        let output = execute(Command::new(&path).stdout(Stdio::piped()));
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"cfg\n");
        // Provenance is metadata, not a source/type-checking/codegen input.
        for block in &mut function.blocks {
            block.terminator.origin = Some(ir::ExprId(usize::MAX));
            for statement in &mut block.statements {
                statement.origin = Some(ir::ExprId(usize::MAX));
            }
        }
        assert_eq!(emit_cfg_function(&function).unwrap().code, object.code);
    }
}
