//! Native machine-code bridges between private snapshot carriers and SysV.
//! Both directions marshal bytes, not pointers to private aggregate storage.
use super::{
    CAbiSignature,
    classify::{Location, Transport},
    layout::{Layout, Scalar},
};
use crate::compiler::{backend::Object, ir};
const SP: u8 = 4;
const BP: u8 = 5;
const ARGS: [u8; 6] = [7, 6, 2, 1, 8, 9];
const RESULTS: [u8; 2] = [0, 2];
const SAVED: [u8; 5] = [3, 12, 13, 14, 15];
struct Code {
    object: Object,
}
impl Code {
    fn new() -> Self {
        Self {
            object: Object {
                code: Vec::new(),
                calls: Vec::new(),
                strings: Vec::new(),
                vtables: Vec::new(),
                exports: Vec::new(),
                receipt: None,
            },
        }
    }
    fn bytes(&mut self, b: &[u8]) {
        self.object.code.extend_from_slice(b);
    }
    fn disp(&mut self, n: usize) -> Result<(), String> {
        let n = i32::try_from(n).map_err(|_| "C bridge displacement overflow")?;
        self.bytes(&n.to_le_bytes());
        Ok(())
    }
    fn mem(
        &mut self,
        prefix: &[u8],
        opcode: &[u8],
        reg: u8,
        base: u8,
        offset: usize,
        wide: bool,
    ) -> Result<(), String> {
        self.bytes(prefix);
        let rex = 0x40 | if wide { 8 } else { 0 } | ((reg >> 3) << 2) | (base >> 3);
        if rex != 0x40 || (opcode == [0x88] && reg >= 4) {
            self.bytes(&[rex]);
        }
        self.bytes(opcode);
        self.bytes(&[0x80 | ((reg & 7) << 3) | (base & 7)]);
        if base & 7 == 4 {
            self.bytes(&[0x24]);
        }
        self.disp(offset)
    }
    fn load(&mut self, reg: u8, base: u8, offset: usize) -> Result<(), String> {
        self.mem(&[], &[0x8b], reg, base, offset, true)
    }
    fn store(&mut self, reg: u8, base: u8, offset: usize) -> Result<(), String> {
        self.mem(&[], &[0x89], reg, base, offset, true)
    }
    fn lea(&mut self, reg: u8, base: u8, offset: usize) -> Result<(), String> {
        self.mem(&[], &[0x8d], reg, base, offset, true)
    }
    fn xmm(&mut self, load: bool, reg: u8, offset: usize) -> Result<(), String> {
        self.mem(
            &[if load { 0xf3 } else { 0x66 }],
            &[0x0f, if load { 0x7e } else { 0xd6 }],
            reg,
            SP,
            offset,
            false,
        )
    }
    fn narrow_load(
        &mut self,
        base: u8,
        offset: usize,
        bytes: usize,
        signed: bool,
    ) -> Result<(), String> {
        match (bytes, signed) {
            (8, _) => self.load(0, base, offset),
            (4, false) => self.mem(&[], &[0x8b], 0, base, offset, false),
            (4, true) => self.mem(&[], &[0x63], 0, base, offset, true),
            (2, s) => self.mem(
                &[],
                &[0x0f, if s { 0xbf } else { 0xb7 }],
                0,
                base,
                offset,
                s,
            ),
            (1, s) => self.mem(
                &[],
                &[0x0f, if s { 0xbe } else { 0xb6 }],
                0,
                base,
                offset,
                s,
            ),
            _ => Err("invalid C scalar width".into()),
        }
    }
    fn narrow_store(&mut self, base: u8, offset: usize, bytes: usize) -> Result<(), String> {
        match bytes {
            8 => self.store(0, base, offset),
            4 => self.mem(&[], &[0x89], 0, base, offset, false),
            2 => self.mem(&[0x66], &[0x89], 0, base, offset, false),
            1 => self.mem(&[], &[0x88], 0, base, offset, false),
            _ => Err("invalid C scalar width".into()),
        }
    }
    fn zero(&mut self, offset: usize, size: usize) -> Result<(), String> {
        self.bytes(&[0x31, 0xc0]);
        for n in (0..size.div_ceil(8) * 8).step_by(8) {
            self.store(0, SP, offset + n)?;
        }
        Ok(())
    }
    fn call(&mut self, target: &str) {
        self.bytes(&[0xe8]);
        self.object
            .calls
            .push((self.object.code.len(), target.into()));
        self.bytes(&[0; 4]);
    }
    fn marshal(
        &mut self,
        layout: &Layout,
        c_base: u8,
        c: usize,
        private_base: u8,
        private: usize,
        to_c: bool,
    ) -> Result<(), String> {
        for leaf in &layout.leaves {
            if to_c {
                self.narrow_load(
                    private_base,
                    private + leaf.private_offset,
                    leaf.private_bytes,
                    matches!(leaf.scalar, Scalar::Integer { signed: true }),
                )?;
                self.narrow_store(c_base, c + leaf.c_offset, leaf.bytes)?;
            } else {
                self.narrow_load(
                    c_base,
                    c + leaf.c_offset,
                    leaf.bytes,
                    matches!(leaf.scalar, Scalar::Integer { signed: true }),
                )?;
                self.narrow_store(
                    private_base,
                    private + leaf.private_offset,
                    leaf.private_bytes,
                )?;
            }
        }
        Ok(())
    }
    fn registers(
        &mut self,
        transport: &Transport,
        buffer: usize,
        result: bool,
        load: bool,
    ) -> Result<(), String> {
        if let Transport::Registers(parts) = transport {
            for (offset, location) in parts {
                match *location {
                    Location::Integer(n) => {
                        let r = if result {
                            RESULTS[n as usize]
                        } else {
                            ARGS[n as usize]
                        };
                        if load {
                            self.load(r, SP, buffer + offset)?;
                        } else {
                            self.store(r, SP, buffer + offset)?;
                        }
                    }
                    Location::Sse(n) => self.xmm(load, n, buffer + offset)?,
                    Location::Stack(_) => return Err("invalid register transport".into()),
                }
            }
        }
        Ok(())
    }
}
struct Frame {
    cursor: usize,
}
impl Frame {
    fn alloc(&mut self, bytes: usize) -> Result<usize, String> {
        self.cursor = super::layout::align_up(self.cursor, 16)?;
        let at = self.cursor;
        self.cursor = self
            .cursor
            .checked_add(bytes.max(8).div_ceil(8) * 8)
            .filter(|v| *v <= super::layout::MAX_BYTES)
            .ok_or("resource limit: C bridge frame")?;
        Ok(at)
    }
}
/// `import=true`: private callers enter this thunk, which calls a native symbol.
/// `import=false`: C callers enter it, which calls a checked private body.
fn generate(
    signature: &CAbiSignature,
    target: &str,
    import: bool,
    budget: &ir::PlanBudget,
) -> Result<Object, String> {
    // Admit a task-scaled conservative code bound before any machine buffers.
    let leaves = signature
        .params
        .iter()
        .map(|p| p.leaves.len())
        .sum::<usize>()
        + signature.result.leaves.len();
    let storage = signature
        .params
        .iter()
        .chain(std::iter::once(&signature.result))
        .try_fold(0usize, |n, layout| {
            if layout.size > super::layout::MAX_BYTES
                || layout.private_size > super::layout::MAX_BYTES
            {
                return Err("resource limit: C bridge layout size");
            }
            n.checked_add(layout.size.max(8))
                .and_then(|n| n.checked_add(layout.private_size.max(8)))
                .ok_or("C bridge storage accounting overflow")
        })?;
    let bound = leaves
        .checked_mul(256)
        .and_then(|n| {
            signature
                .params
                .len()
                .checked_mul(256)
                .and_then(|p| n.checked_add(p))
        })
        .and_then(|n| storage.checked_mul(8).and_then(|s| n.checked_add(s)))
        .and_then(|n| n.checked_add(4096))
        .ok_or("C bridge code accounting overflow")?;
    budget.charge_work(bound)?;
    budget.retain_bytes(bound)?;
    if signature.plan != super::classify::plan(&signature.params, &signature.result)? {
        return Err("contract: forged C ABI classification".into());
    }
    let mut frame = Frame {
        cursor: if import {
            signature.plan.stack_bytes
        } else {
            (signature.params.len() + usize::from(signature.result.indirect)) * 8
        },
    };
    let mut c_buffers = Vec::with_capacity(signature.params.len());
    let mut private_buffers = Vec::with_capacity(signature.params.len());
    for layout in &signature.params {
        c_buffers.push(frame.alloc(layout.size)?);
        private_buffers.push(frame.alloc(layout.private_size)?);
    }
    let c_result = frame.alloc(signature.result.size)?;
    let private_result = frame.alloc(signature.result.private_size)?;
    let sret = frame.alloc(8)?;
    let saved = frame.alloc(40)?;
    let total = super::layout::align_up(frame.cursor, 16)?;
    let mut code = Code::new();
    code.object.code.reserve_exact(bound);
    code.bytes(&[
        0x55, 0x48, 0x89, 0xe5, 0x48, 0x83, 0xe4, 0xf0, 0x48, 0x81, 0xec,
    ]);
    code.disp(total)?;
    for (n, r) in SAVED.into_iter().enumerate() {
        code.store(r, SP, saved + n * 8)?;
    }
    // Save incoming registers before any conversion scratch can overwrite them.
    if !import {
        if signature.plan.sret {
            code.store(7, SP, sret)?;
        }
        for (transport, buffer) in signature.plan.arguments.iter().zip(&c_buffers) {
            code.registers(transport, *buffer, false, false)?;
        }
    }
    for (index, ((layout, transport), buffer)) in signature
        .params
        .iter()
        .zip(&signature.plan.arguments)
        .zip(&c_buffers)
        .enumerate()
    {
        if import {
            code.zero(*buffer, layout.size)?;
            let incoming = 16
                + (signature.params.len() - 1 - index + usize::from(signature.result.indirect)) * 8;
            if layout.indirect {
                code.load(10, BP, incoming)?;
                code.marshal(layout, SP, *buffer, 10, 0, true)?;
            } else {
                code.load(0, BP, incoming)?;
                code.store(0, SP, private_buffers[index])?;
                code.marshal(layout, SP, *buffer, SP, private_buffers[index], true)?;
            }
            if let Transport::Memory { offset } = transport {
                for n in (0..layout.size.div_ceil(8) * 8).step_by(8) {
                    code.load(0, SP, *buffer + n)?;
                    code.store(0, SP, offset + n)?;
                }
            }
        } else {
            let private = private_buffers[index];
            code.zero(private, layout.private_size)?;
            if let Transport::Memory { offset } = transport {
                code.marshal(layout, BP, 16 + offset, SP, private, false)?;
            } else {
                code.marshal(layout, SP, *buffer, SP, private, false)?;
            }
            if layout.indirect {
                code.lea(0, SP, private)?;
            } else {
                code.load(0, SP, private)?;
            }
            let outgoing =
                (signature.params.len() - 1 - index + usize::from(signature.result.indirect)) * 8;
            code.store(0, SP, outgoing)?;
        }
    }
    code.zero(c_result, signature.result.size)?;
    code.zero(private_result, signature.result.private_size)?;
    if import {
        for (transport, buffer) in signature.plan.arguments.iter().zip(&c_buffers) {
            code.registers(transport, *buffer, false, true)?;
        }
        if signature.plan.sret {
            code.lea(7, SP, c_result)?;
        }
        // Fixed-prototype C calls have no variadic %al protocol.
    } else if signature.result.indirect {
        code.lea(0, SP, private_result)?;
        code.store(0, SP, 0)?;
    }
    code.call(target);
    if import {
        code.registers(&signature.plan.result, c_result, true, false)?;
        if signature.result.indirect {
            code.load(10, BP, 16)?;
            code.marshal(&signature.result, SP, c_result, 10, 0, false)?;
            code.load(0, BP, 16)?;
        } else {
            code.marshal(&signature.result, SP, c_result, SP, private_result, false)?;
            code.load(0, SP, private_result)?;
        }
    } else {
        if !signature.result.indirect {
            code.store(0, SP, private_result)?;
        }
        if signature.plan.sret {
            code.load(10, SP, sret)?;
            code.marshal(&signature.result, 10, 0, SP, private_result, true)?;
            code.load(0, SP, sret)?;
        } else {
            code.marshal(&signature.result, SP, c_result, SP, private_result, true)?;
            code.registers(&signature.plan.result, c_result, true, true)?;
        }
    }
    for (n, r) in SAVED.into_iter().enumerate() {
        code.load(r, SP, saved + n * 8)?;
    }
    code.bytes(&[0xc9, 0xc3]);
    if code.object.code.len() > bound {
        return Err("resource limit: C bridge code bound".into());
    }
    Ok(code.object)
}
pub fn import(
    signature: &CAbiSignature,
    symbol: &str,
    budget: &ir::PlanBudget,
) -> Result<Object, String> {
    generate(signature, symbol, true, budget)
}
pub fn export(
    signature: &CAbiSignature,
    private_symbol: &str,
    budget: &ir::PlanBudget,
) -> Result<Object, String> {
    generate(signature, private_symbol, false, budget)
}
