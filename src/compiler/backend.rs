//! Direct Linux x86-64 backend. No external compiler, assembler, or linker.
//!
//! Private ABI: arguments are evaluated left-to-right and pushed as eight-byte
//! stack words. The callee reads them in reverse order above rbp. eax carries
//! i32/bool results (unit is zero). All registers are caller-saved. Frames use
//! stable rbp-relative slots, independent of temporary pushes. At most 4096
//! parameters/arguments are accepted. This is not a platform interoperability ABI.
use super::frontend::{self, BinaryOp, Program, UnaryOp};
use super::ir::{self, ExprKind, LocalId, LoopId};
use std::collections::HashMap;

const MAX_ARGS: usize = 4096;
const BASE: u64 = 0x400000;
const PAGE: usize = 4096;

/// Relocations refer to byte offsets within `code`: calls address a signed
/// rel32 displacement field, strings address an absolute u64 immediate field.
#[derive(Clone, Debug)]
pub struct Object {
    pub code: Vec<u8>,
    pub calls: Vec<(usize, String)>,
    pub strings: Vec<(usize, Vec<u8>)>,
}

struct LoopFrame {
    id: LoopId,
    head: usize,
    temporary_depth: usize,
    breaks: Vec<usize>,
}

struct Emitter<'a> {
    object: Object,
    function: &'a ir::CheckedFunction,
    returns: Vec<usize>,
    traps: Vec<usize>,
    loops: Vec<LoopFrame>,
    temporary_depth: usize,
}
impl Emitter<'_> {
    fn bytes(&mut self, bytes: &[u8]) {
        self.object.code.extend_from_slice(bytes);
    }
    fn i32(&mut self, n: i32) {
        self.bytes(&n.to_le_bytes());
    }
    fn position(&self) -> usize {
        self.object.code.len()
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
    fn trap_if(&mut self, condition: u8) {
        let at = self.jump(&[0x0f, condition]);
        self.traps.push(at);
    }
    fn immediate(&mut self, n: i32) {
        self.bytes(&[0xb8]);
        self.i32(n);
    }
    fn local_offset(&self, local: LocalId) -> Result<i32, String> {
        i32::try_from(local.0)
            .ok()
            .and_then(|index| index.checked_add(1))
            .and_then(|slot| slot.checked_mul(-8))
            .ok_or_else(|| "local frame too large".to_string())
    }
    fn load(&mut self, offset: i32) {
        self.bytes(&[0x8b, 0x85]);
        self.i32(offset);
    }
    fn store(&mut self, offset: i32) {
        self.bytes(&[0x89, 0x85]);
        self.i32(offset);
    }
    fn push_temporary(&mut self) -> Result<(), String> {
        self.temporary_depth = self
            .temporary_depth
            .checked_add(8)
            .ok_or("temporary stack too large")?;
        self.bytes(&[0x50]);
        Ok(())
    }
    fn discard_stack_bytes(&mut self, bytes: usize) -> Result<(), String> {
        if bytes != 0 {
            let bytes = i32::try_from(bytes).map_err(|_| "temporary stack too large")?;
            self.bytes(&[0x48, 0x81, 0xc4]);
            self.i32(bytes);
        }
        Ok(())
    }
    fn enter_loop(&mut self, id: LoopId) {
        self.loops.push(LoopFrame {
            id,
            head: self.position(),
            temporary_depth: self.temporary_depth,
            breaks: Vec::new(),
        });
    }
    fn exit_loop(&mut self) -> Result<(), String> {
        let frame = self.loops.pop().ok_or("missing backend loop frame")?;
        for at in frame.breaks {
            self.patch(at, self.position())?;
        }
        Ok(())
    }
    fn loop_jump(&mut self, target: LoopId, is_break: bool) -> Result<(), String> {
        let index = self
            .loops
            .iter()
            .rposition(|frame| frame.id == target)
            .ok_or("missing backend loop target")?;
        let depth = self.loops[index].temporary_depth;
        let bytes = self
            .temporary_depth
            .checked_sub(depth)
            .ok_or("invalid backend temporary depth")?;
        // This cleanup belongs only to the abrupt runtime edge. Keep lexical
        // accounting unchanged so unreachable code and sibling branches compile
        // with their own pending operands/arguments intact. eax holds break's value.
        self.discard_stack_bytes(bytes)?;
        let at = self.jump(&[0xe9]);
        if is_break {
            self.loops[index].breaks.push(at);
        } else {
            self.patch(at, self.loops[index].head)?;
        }
        Ok(())
    }
    fn block(&mut self, block: &ir::Block) -> Result<(), String> {
        for statement in &block.statements {
            match statement {
                ir::Stmt::Let { local, value } | ir::Stmt::Assign { local, value } => {
                    self.expr(*value)?;
                    self.store(self.local_offset(*local)?);
                }
                ir::Stmt::Expr(value) => self.expr(*value)?,
                ir::Stmt::Return(value) => {
                    if let Some(value) = value {
                        self.expr(*value)?;
                    } else {
                        self.immediate(0);
                    }
                    let at = self.jump(&[0xe9]);
                    self.returns.push(at);
                }
                ir::Stmt::While {
                    id,
                    condition,
                    body,
                } => {
                    let start = self.position();
                    self.enter_loop(*id);
                    self.expr(*condition)?;
                    self.bytes(&[0x85, 0xc0]);
                    let end = self.jump(&[0x0f, 0x84]);
                    self.block(body)?;
                    let back = self.jump(&[0xe9]);
                    self.patch(back, start)?;
                    self.patch(end, self.position())?;
                    self.exit_loop()?;
                    self.immediate(0);
                }
            }
        }
        if let Some(tail) = &block.tail {
            self.expr(*tail)?;
        } else {
            self.immediate(0);
        }
        Ok(())
    }
    fn expr(&mut self, expression: ir::ExprId) -> Result<(), String> {
        match &self.function.exprs[expression.0].kind {
            ExprKind::Int(n) => self.immediate(*n),
            ExprKind::Bool(value) => self.immediate(i32::from(*value)),
            ExprKind::Local(local) => self.load(self.local_offset(*local)?),
            ExprKind::Unary { op, value } => {
                self.expr(*value)?;
                match op {
                    UnaryOp::Neg => {
                        self.bytes(&[0xf7, 0xd8]);
                        self.trap_if(0x80);
                    }
                    UnaryOp::Not => self.bytes(&[0x83, 0xf0, 0x01]),
                }
            }
            ExprKind::Binary { op, left, right } => {
                self.expr(*left)?;
                if matches!(op, BinaryOp::And | BinaryOp::Or) {
                    self.bytes(&[0x85, 0xc0]);
                    let done = self.jump(&[
                        0x0f,
                        if matches!(op, BinaryOp::And) {
                            0x84
                        } else {
                            0x85
                        },
                    ]);
                    self.expr(*right)?;
                    self.patch(done, self.position())?;
                    return Ok(());
                }
                self.push_temporary()?;
                self.expr(*right)?;
                self.bytes(&[0x89, 0xc1, 0x58]); // ecx = right, pop left
                self.temporary_depth = self
                    .temporary_depth
                    .checked_sub(8)
                    .ok_or("invalid backend temporary depth")?;
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
                        self.bytes(&[0x99, 0xf7, 0xf9]); // cdq; idiv ecx
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
                    BinaryOp::And | BinaryOp::Or => unreachable!(),
                }
            }
            ExprKind::Call { name, args, .. } => {
                if args.len() > MAX_ARGS {
                    return Err(format!("at most {MAX_ARGS} arguments are supported"));
                }
                for arg in args {
                    self.expr(*arg)?;
                    self.push_temporary()?;
                }
                let at = self.jump(&[0xe8]);
                self.object.calls.push((at, name.clone()));
                let bytes = args.len() * 8;
                self.discard_stack_bytes(bytes)?;
                self.temporary_depth = self
                    .temporary_depth
                    .checked_sub(bytes)
                    .ok_or("invalid backend temporary depth")?;
            }
            ExprKind::If {
                condition,
                then_block,
                else_block,
            } => {
                self.expr(*condition)?;
                self.bytes(&[0x85, 0xc0]);
                let other = self.jump(&[0x0f, 0x84]);
                self.block(then_block)?;
                let done = self.jump(&[0xe9]);
                self.patch(other, self.position())?;
                self.block(else_block)?;
                self.patch(done, self.position())?;
            }
            ExprKind::Block(block) => self.block(block)?,
            ExprKind::Loop { id, body } => {
                let head = self.position();
                self.enter_loop(*id);
                self.block(body)?;
                let back = self.jump(&[0xe9]);
                self.patch(back, head)?;
                self.exit_loop()?;
            }
            ExprKind::Break { target, value } => {
                if let Some(value) = value {
                    self.expr(*value)?;
                } else {
                    self.immediate(0);
                }
                self.loop_jump(*target, true)?;
            }
            ExprKind::Continue { target } => self.loop_jump(*target, false)?,
            ExprKind::Println(text) => {
                let mut bytes = text.as_bytes().to_vec();
                bytes.push(b'\n');
                self.bytes(&[0x48, 0xbe]); // movabs rsi, literal
                let at = self.position();
                self.bytes(&[0; 8]);
                let length = u64::try_from(bytes.len()).map_err(|_| "literal too large")?;
                self.object.strings.push((at, bytes));
                self.bytes(&[0x48, 0xba]);
                self.bytes(&length.to_le_bytes());
                let retry = self.position();
                self.bytes(&[0xb8, 1, 0, 0, 0, 0xbf, 1, 0, 0, 0, 0x0f, 0x05]);
                // Retry EINTR, handle partial writes, fail on all other errors or zero progress.
                self.bytes(&[0x48, 0x83, 0xf8, 0xfc]);
                let interrupted = self.jump(&[0x0f, 0x84]);
                self.patch(interrupted, retry)?;
                self.bytes(&[0x48, 0x85, 0xc0]);
                self.trap_if(0x8e);
                self.bytes(&[0x48, 0x01, 0xc6, 0x48, 0x29, 0xc2]);
                let more = self.jump(&[0x0f, 0x85]);
                self.patch(more, retry)?;
                self.immediate(0);
            }
        }
        Ok(())
    }
}

/// Compile one checked function independently, suitable for parallel graph workers.
/// Semantic analysis is complete. Local slots are `-8 * (LocalId + 1)` and
/// parameters are copied into those slots from the incoming private ABI.
pub fn emit_checked_function(function: &ir::CheckedFunction) -> Result<Object, String> {
    if function.params.len() > MAX_ARGS {
        return Err(format!("at most {MAX_ARGS} parameters are supported"));
    }
    let mut emitter = Emitter {
        object: Object {
            code: Vec::new(),
            calls: Vec::new(),
            strings: Vec::new(),
        },
        function,
        returns: Vec::new(),
        traps: Vec::new(),
        loops: Vec::new(),
        temporary_depth: 0,
    };
    emitter.bytes(&[0x55, 0x48, 0x89, 0xe5, 0x48, 0x81, 0xec]);
    let frame = emitter.position();
    emitter.i32(0);
    for (index, local) in function.params.iter().enumerate() {
        let offset = 16 + (function.params.len() - index - 1) * 8;
        emitter.load(offset as i32);
        emitter.store(emitter.local_offset(*local)?);
    }
    emitter.block(&function.body)?;
    let epilogue = emitter.position();
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
    let frame_bytes = i32::try_from(function.locals.len())
        .ok()
        .and_then(|slots| slots.checked_mul(8))
        .ok_or("frame too large")?;
    emitter.object.code[frame..frame + 4].copy_from_slice(&frame_bytes.to_le_bytes());
    Ok(emitter.object)
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
        file.write_all(&emit(program).unwrap()).unwrap();
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
        assert_eq!(
            i32::from_le_bytes(original.code[7..11].try_into().unwrap()),
            16
        );
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
}
