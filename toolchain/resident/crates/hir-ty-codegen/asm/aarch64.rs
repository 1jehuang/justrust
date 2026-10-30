//! The function around an `asm!` block on AArch64.
//!
//! The slot's address arrives in `x0` and is kept in `x19`, which `asm!` takes as no
//! operand and a function has to restore; `x29` and `x30` are saved with it, as a block may
//! call and may name `lr`.

use std::fmt::Write;

use hir_def::hir::{AsmOptions, AsmPiece, InlineAsmRegOrRegClass};

use super::{ObjectFormat, Operand, SYMBOL, Wrapper};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reg {
    /// `x` and `w` of one number are one register.
    Gpr(u8),
    /// `v`, `q`, `d`, `s`, `h` and `b` of one number are one register.
    Vector(u8),
    /// One that holds no value here and that a call does not keep: nothing to do.
    Scratch,
}

/// What `asm!` hands out for the class `reg`, the ones a call does not keep first. `x18` is
/// the platform's, `x19` holds the slot, `x29` is the frame pointer.
const OPERANDS: [u8; 28] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 20, 21, 22, 23, 24, 25, 26, 27,
    28, 30,
];
/// The general registers a function has to leave as it found them, other than the ones
/// the wrapper saves on its own stack.
const KEPT: std::ops::RangeInclusive<u8> = 20..=28;
/// The vector registers whose low eight bytes a function has to leave as it found them.
const KEPT_VECTORS: std::ops::RangeInclusive<u8> = 8..=15;

impl Reg {
    fn named(name: &str) -> Result<Reg, String> {
        let name = name.trim_matches('"');
        let numbered = |prefix: char| name.strip_prefix(prefix)?.parse::<u8>().ok();
        if name == "lr" {
            return Ok(Reg::Gpr(30));
        }
        if let Some(number) = numbered('x').or(numbered('w')) {
            return if OPERANDS.contains(&number) {
                Ok(Reg::Gpr(number))
            } else {
                Err(format!("inline assembly that uses `{name}`"))
            };
        }
        let vector = ['v', 'q', 'd', 's', 'h', 'b'].into_iter().find_map(numbered);
        if let Some(number) = vector.filter(|&it| it < 32) {
            return Ok(Reg::Vector(number));
        }
        // The predicate and matrix registers of SVE and SME, which a block can only clobber.
        let scratch = name == "ffr"
            || name == "za"
            || name == "zt0"
            || numbered('p').is_some()
            || numbered('z').is_some();
        if scratch { Ok(Reg::Scratch) } else { Err(format!("the register `{name}`")) }
    }

    /// The name by which an instruction moves `size` bytes through the register.
    fn sized(self, size: u32) -> Result<String, String> {
        let prefix = match (self, size) {
            (Reg::Gpr(_), 1 | 2 | 4) => 'w',
            (Reg::Gpr(_), 8) => 'x',
            (Reg::Vector(_), 1) => 'b',
            (Reg::Vector(_), 2) => 'h',
            (Reg::Vector(_), 4) => 's',
            (Reg::Vector(_), 8) => 'd',
            (Reg::Vector(_), 16) => 'q',
            _ => {
                return Err(format!(
                    "an inline assembly operand of {size} bytes in such a register"
                ));
            }
        };
        match self {
            Reg::Gpr(number) | Reg::Vector(number) => Ok(format!("{prefix}{number}")),
            Reg::Scratch => {
                Err("an inline assembly operand in a register with no value".to_owned())
            }
        }
    }

    /// The instruction that moves `size` bytes between the register and the slot.
    fn transfer(self, load: bool, size: u32, offset: u32) -> Result<String, String> {
        let narrow = match (self, size) {
            (Reg::Gpr(_), 1) => "b",
            (Reg::Gpr(_), 2) => "h",
            _ => "",
        };
        let instruction = if load { "ldr" } else { "str" };
        Ok(format!("{instruction}{narrow} {}, [x19, #{offset}]", self.sized(size)?))
    }
}

/// How LLVM's inline assembly names the general register `name` names, if it is one.
pub(super) fn llvm_register(name: &str) -> Option<String> {
    match Reg::named(name) {
        Ok(Reg::Gpr(number)) => Some(format!("{{x{number}}}")),
        _ => None,
    }
}

pub(super) fn wrapper(
    format: ObjectFormat,
    template: &[AsmPiece],
    options: AsmOptions,
    operands: &[Operand<'_>],
) -> Result<Wrapper, String> {
    // The registers the operands name come first: a class takes one of what is left.
    let mut regs = vec![None; operands.len()];
    for (index, operand) in operands.iter().enumerate() {
        if let Operand::Reg { reg: InlineAsmRegOrRegClass::Reg(name), .. } = operand {
            regs[index] = Some(Reg::named(name.as_str())?);
        }
    }
    for (index, operand) in operands.iter().enumerate() {
        let Operand::Reg { reg: InlineAsmRegOrRegClass::RegClass(class), .. } = operand else {
            continue;
        };
        let class = class.as_str();
        let free = |reg: &Reg| !regs.contains(&Some(*reg));
        let found = match class {
            "reg" => OPERANDS.into_iter().map(Reg::Gpr).find(free),
            "vreg" => (0..32).map(Reg::Vector).find(free),
            "vreg_low16" => (0..16).map(Reg::Vector).find(free),
            _ => return Err(format!("the register class `{class}`")),
        };
        let Some(found) = found else {
            return Err(format!("inline assembly with more `{class}` operands than registers"));
        };
        regs[index] = Some(found);
    }

    let kept: Vec<Reg> = KEPT
        .map(Reg::Gpr)
        .chain(KEPT_VECTORS.map(Reg::Vector))
        .filter(|reg| regs.contains(&Some(*reg)))
        .collect();
    let mut slot_size = kept.len() as u32 * 8;
    let mut offsets = vec![None; operands.len()];
    for (index, operand) in operands.iter().enumerate() {
        let Operand::Reg { size: Some(size), .. } = *operand else { continue };
        if !size.is_power_of_two() || size > 16 {
            return Err(format!("an inline assembly operand of {size} bytes"));
        }
        slot_size = slot_size.next_multiple_of(size);
        offsets[index] = Some(slot_size);
        slot_size += size;
    }
    let values = || {
        operands.iter().enumerate().filter_map(|(index, operand)| match *operand {
            Operand::Reg { size: Some(size), input, output, .. } => {
                Some((regs[index]?, size, offsets[index]?, input, output))
            }
            _ => None,
        })
    };

    let mut text = format!(
        ".globl {SYMBOL}\n{SYMBOL}:\nstp x29, x30, [sp, #-32]!\nmov x29, sp\n\
         str x19, [sp, #16]\nmov x19, x0\n"
    );
    for (at, reg) in kept.iter().enumerate() {
        _ = writeln!(text, "{}", reg.transfer(false, 8, at as u32 * 8)?);
    }
    for (reg, size, offset, input, _) in values() {
        if input {
            _ = writeln!(text, "{}", reg.transfer(true, size, offset)?);
        }
    }
    for piece in template {
        let (index, modifier) = match piece {
            AsmPiece::Lit(lit) => {
                text.push_str(lit);
                continue;
            }
            AsmPiece::Operand { index, modifier } => (*index, *modifier),
        };
        let prefix = match (operands.get(index), regs.get(index).copied().flatten()) {
            (Some(Operand::Const(value)), _) => {
                _ = write!(text, "{value}");
                continue;
            }
            (Some(Operand::Label), _) => {
                text.push_str(&super::label(format, index));
                continue;
            }
            (_, Some(Reg::Gpr(number))) => {
                match modifier {
                    None | Some('x') => _ = write!(text, "x{number}"),
                    Some('w') => _ = write!(text, "w{number}"),
                    Some(other) => return Err(format!("the operand modifier `{other}`")),
                }
                continue;
            }
            (_, Some(Reg::Vector(number))) => match modifier {
                None => ('v', number),
                Some(it @ ('v' | 'q' | 'd' | 's' | 'h' | 'b')) => (it, number),
                Some(other) => return Err(format!("the operand modifier `{other}`")),
            },
            _ => return Err("an inline assembly template that names no operand".to_owned()),
        };
        _ = write!(text, "{}{}", prefix.0, prefix.1);
    }
    text.push('\n');
    let returns = !options.contains(AsmOptions::NORETURN);
    if returns {
        for (reg, size, offset, _, output) in values() {
            if output {
                _ = writeln!(text, "{}", reg.transfer(false, size, offset)?);
            }
        }
        if super::labels(operands).next().is_some() {
            text.push_str("mov x0, #0\n");
        }
    } else {
        text.push_str("brk #1\n");
    }
    let mut jumped = false;
    for (at, index) in super::labels(operands).enumerate() {
        if returns || jumped {
            _ = writeln!(text, "b {}", super::done(format));
        }
        _ = writeln!(text, "{}:\nmov x0, #{}", super::label(format, index), at + 1);
        jumped = true;
    }
    if returns || jumped {
        _ = writeln!(text, "{}:", super::done(format));
        for (at, reg) in kept.iter().enumerate() {
            _ = writeln!(text, "{}", reg.transfer(true, 8, at as u32 * 8)?);
        }
        text.push_str("ldr x19, [sp, #16]\nldp x29, x30, [sp], #32\nret\n");
    }
    Ok(Wrapper { text, slot_size: slot_size.next_multiple_of(16).max(16), offsets })
}
