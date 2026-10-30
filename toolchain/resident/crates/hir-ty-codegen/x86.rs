//! x86's intrinsics of LLVM that `core::arch` calls and that LLVM selects one instruction
//! (or a few, for a flag made a value) for: the SSE and AVX operations whose NaNs, rounding,
//! saturation and reciprocal estimates are the instruction's, the conversions that read
//! MXCSR, the tests that set flags, the crypto instructions, and the instructions on the
//! machine's state (fences, MXCSR, XSAVE, the time stamp counter, random numbers). That
//! instruction is the lowering that agrees with LLVM's on every input, so [`lower`] says it.
//! The code generator calls it as a function of its own (`asm::x86_function`), which loads
//! each argument into the register [`Lowered::args`] names, runs the text, and writes each
//! register [`Lowered::results`] names where the result goes.

/// An intrinsic as instructions on the registers `asm::x86_function` loads.
pub(super) struct Lowered {
    pub(super) text: String,
    /// The register each argument is read into, `None` for an immediate, which the text
    /// names as `{imm}`.
    pub(super) args: Vec<Option<&'static str>>,
    /// The registers of the results, one for each field of a result that is a struct.
    pub(super) results: Vec<&'static str>,
    /// The registers the text writes that are neither an argument's nor a result's.
    pub(super) clobbers: Vec<&'static str>,
}

fn lowered(
    text: impl Into<String>,
    args: &[&'static str],
    results: &[&'static str],
    clobbers: &[&'static str],
) -> Option<Lowered> {
    Some(Lowered {
        text: text.into(),
        args: args.iter().map(|&it| Some(it)).collect(),
        results: results.to_vec(),
        clobbers: clobbers.to_vec(),
    })
}

/// The vector register `xmm`, `ymm` or `zmm` `k` of `bytes`.
fn vector(bytes: u32, k: u32) -> &'static str {
    const NAMES: [[&str; 3]; 3] =
        [["xmm0", "xmm1", "xmm2"], ["ymm0", "ymm1", "ymm2"], ["zmm0", "zmm1", "zmm2"]];
    NAMES[match bytes {
        16 => 0,
        32 => 1,
        _ => 2,
    }][k as usize]
}

/// The bytes of a vector of `llvm.x86.*.{width}`: 16 without one.
fn width(name: &str) -> u32 {
    match name.rsplit('.').next() {
        Some("256") => 32,
        Some("512") => 64,
        _ => 16,
    }
}

/// The flag an instruction set made a 32-bit 0 or 1, as LLVM reads it with `setcc`; `eq`
/// and `neq` of a comparison of floats also read the parity flag, which says the two were
/// unordered.
fn flag(condition: &str) -> String {
    match condition {
        "eq" => "setnp al\nsete cl\nand al, cl\nmovzx eax, al".to_owned(),
        "neq" => "setp al\nsetne cl\nor al, cl\nmovzx eax, al".to_owned(),
        _ => format!("set{condition} al\nmovzx eax, al"),
    }
}

/// The instructions of the intrinsic `llvm.{name}`, or `None` for one this does not know.
pub(super) fn lower(name: &str) -> Option<Lowered> {
    let op = name.strip_prefix("x86.")?;
    let (x0, x1) = ("xmm0", "xmm1");
    match op {
        // Lane by lane into the first operand, and the lanes above the first of the first
        // operand for the scalar ones: a NaN, the order of the two zeros, the rounding of
        // `cvtsd2ss`, the saturation of `pmulhrsw` and the counts of the shifts past the
        // lane's width are the instruction's. The functions of `core::arch` that call these
        // have SSE's features and not AVX's, for which LLVM selects the SSE encoding.
        "sse.max.ps"
        | "sse.max.ss"
        | "sse.min.ps"
        | "sse.min.ss"
        | "sse2.max.pd"
        | "sse2.max.sd"
        | "sse2.min.pd"
        | "sse2.min.sd"
        | "sse2.cvtsd2ss"
        | "sse2.psll.w"
        | "sse2.psll.d"
        | "sse2.psll.q"
        | "sse2.psra.w"
        | "sse2.psra.d"
        | "sse2.psrl.w"
        | "sse2.psrl.d"
        | "sse2.psrl.q"
        | "sse4a.extrq"
        | "sse4a.insertq"
        | "ssse3.psign.b.128"
        | "ssse3.psign.w.128"
        | "ssse3.psign.d.128"
        | "ssse3.pmul.hr.sw.128"
        | "aesni.aesenc"
        | "aesni.aesenclast"
        | "aesni.aesdec"
        | "aesni.aesdeclast"
        | "vgf2p8mulb.128" => {
            let mnemonic = match op {
                "vgf2p8mulb.128" => "gf2p8mulb".to_owned(),
                _ => op.trim_end_matches(".128").split('.').skip(1).collect(),
            };
            lowered(format!("{mnemonic} xmm0, xmm1"), &[x0, x1], &[x0], &[])
        }
        // Of one operand, the scalar ones keeping the lanes above the first.
        "sse.rcp.ps" | "sse.rcp.ss" | "sse.rsqrt.ps" | "sse.rsqrt.ss" | "aesni.aesimc"
        | "sse2.cvtpd2dq" | "sse2.cvtps2dq" | "sse2.cvttpd2dq" | "sse2.cvttps2dq" => {
            let mnemonic = op.split('.').skip(1).collect::<String>();
            lowered(format!("{mnemonic} xmm0, xmm0"), &[x0], &[x0], &[])
        }
        // The first lane to an integer, rounded as MXCSR says (`t` toward zero); past the
        // integer's range or a NaN, its least.
        "sse.cvtss2si" | "sse.cvttss2si" | "sse2.cvtsd2si" | "sse2.cvttsd2si" => {
            let mnemonic = op.split('.').nth(1)?;
            lowered(format!("{mnemonic} eax, xmm0"), &[x0], &["eax"], &[])
        }
        "sse.cvtss2si64" | "sse.cvttss2si64" | "sse2.cvtsd2si64" | "sse2.cvttsd2si64" => {
            let mnemonic = op.split('.').nth(1)?.trim_end_matches("64");
            lowered(format!("{mnemonic} rax, xmm0"), &[x0], &["rax"], &[])
        }
        // The first lanes compared, as LLVM reads the flags `comis[sd]` and `ucomis[sd]`
        // set: `lt` and `le` are `gt` and `ge` of the two the other way round.
        _ if let Some((condition, mnemonic)) = comparison(op) => {
            let (l, r) = if matches!(condition, "lt" | "le") { (x1, x0) } else { (x0, x1) };
            let condition = match condition {
                "gt" | "lt" => "a",
                "ge" | "le" => "ae",
                other => other,
            };
            let text = format!("{mnemonic} {l}, {r}\n{}", flag(condition));
            lowered(text, &[x0, x1], &["eax"], &["ecx"])
        }
        // Bits of the two tested together: `c` is the carry flag (no bit of the second where
        // the first has none), `z` the zero flag (no bit in both), `nzc` neither.
        "sse41.ptestnzc"
        | "avx.ptestnzc.256"
        | "avx.vtestc.pd.256"
        | "avx.vtestc.ps.256"
        | "avx.vtestz.pd.256"
        | "avx.vtestz.ps.256"
        | "avx.vtestnzc.pd"
        | "avx.vtestnzc.ps"
        | "avx.vtestnzc.pd.256"
        | "avx.vtestnzc.ps.256" => {
            let bytes = width(op);
            let mut words = op.split('.');
            let family = words.next()?;
            let test = words.next()?;
            let mnemonic = match (family, words.next()) {
                ("sse41", _) => "ptest",
                (_, Some("pd")) => "vtestpd",
                (_, Some("ps")) => "vtestps",
                _ => "vptest",
            };
            let condition = match test.trim_start_matches("vtest").trim_start_matches("ptest") {
                "c" => "b",
                "z" => "e",
                _ => "a",
            };
            let (a, b) = (vector(bytes, 0), vector(bytes, 1));
            lowered(format!("{mnemonic} {a}, {b}\n{}", flag(condition)), &[a, b], &["eax"], &[])
        }
        // The first lane of the second operand rounded as the immediate says, the others
        // the first operand's.
        "sse41.round.ss" | "sse41.round.sd" => Some(Lowered {
            text: format!("round{} xmm0, xmm1, {{imm}}", &op[op.len() - 2..]),
            args: vec![Some(x0), Some(x1), None],
            results: vec![x0],
            clobbers: Vec::new(),
        }),
        // AVX's and AVX2's, of three operands.
        "avx.max.pd.256"
        | "avx.max.ps.256"
        | "avx.min.pd.256"
        | "avx.min.ps.256"
        | "avx.vpermilvar.pd"
        | "avx.vpermilvar.ps"
        | "avx.vpermilvar.pd.256"
        | "avx.vpermilvar.ps.256"
        | "avx2.psign.b"
        | "avx2.psign.w"
        | "avx2.psign.d"
        | "avx2.pmul.hr.sw"
        | "aesni.aesenc.256"
        | "aesni.aesenclast.256"
        | "aesni.aesdec.256"
        | "aesni.aesdeclast.256"
        | "aesni.aesenc.512"
        | "aesni.aesenclast.512"
        | "aesni.aesdec.512"
        | "aesni.aesdeclast.512"
        | "vgf2p8mulb.256"
        | "vgf2p8mulb.512" => {
            // AVX2's integer forms are of 256 bits without a suffix.
            let bytes = if op.starts_with("avx2.") { 32 } else { width(op) };
            let words: Vec<&str> = op.split('.').collect();
            let mnemonic = match words.as_slice() {
                ["avx", "vpermilvar", lanes, ..] => format!("vpermil{lanes}"),
                ["avx", op, lanes, ..] => format!("v{op}{lanes}"),
                ["avx2", "psign", lanes] => format!("vpsign{lanes}"),
                ["avx2", "pmul", "hr", "sw"] => "vpmulhrsw".to_owned(),
                ["aesni", op, _] => format!("v{op}"),
                _ => "vgf2p8mulb".to_owned(),
            };
            let (a, b) = (vector(bytes, 0), vector(bytes, 1));
            lowered(format!("{mnemonic} {a}, {a}, {b}"), &[a, b], &[a], &[])
        }
        // Each lane shifted by the count in the low 64 bits of the second operand.
        "avx2.psll.w" | "avx2.psll.d" | "avx2.psll.q" | "avx2.psra.w" | "avx2.psra.d"
        | "avx2.psrl.w" | "avx2.psrl.d" | "avx2.psrl.q" => {
            let mnemonic = format!("vp{}", op["avx2.p".len()..].replace('.', ""));
            lowered(format!("{mnemonic} ymm0, ymm0, xmm1"), &["ymm0", x1], &["ymm0"], &[])
        }
        // To integers, rounded as MXCSR says (`cvtt` toward zero), of four doubles into a
        // vector of 128 bits or of eight singles.
        "avx.cvt.pd2dq.256" | "avx.cvtt.pd2dq.256" => {
            let mnemonic = format!("v{}", op[4..op.len() - 4].replace('.', ""));
            lowered(format!("{mnemonic} xmm0, ymm0"), &["ymm0"], &[x0], &[])
        }
        "avx.cvt.ps2dq.256" | "avx.cvtt.ps2dq.256" | "avx.rcp.ps.256" | "avx.rsqrt.ps.256" => {
            let mnemonic = format!("v{}", op[4..op.len() - 4].replace('.', ""));
            lowered(format!("{mnemonic} ymm0, ymm0"), &["ymm0"], &["ymm0"], &[])
        }
        // Zeroes every vector register, which holds nothing of the caller's across a call.
        "avx.vzeroall" => {
            const ALL: [&str; 16] = [
                "xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7", "xmm8", "xmm9",
                "xmm10", "xmm11", "xmm12", "xmm13", "xmm14", "xmm15",
            ];
            lowered("vzeroall", &[], &[], &ALL)
        }
        // SHA-512, SM3 and SM4 (AVX's encodings), the first operand also the result.
        "vsha512msg1" => lowered("vsha512msg1 ymm0, xmm1", &["ymm0", x1], &["ymm0"], &[]),
        "vsha512msg2" => lowered("vsha512msg2 ymm0, ymm1", &["ymm0", "ymm1"], &["ymm0"], &[]),
        "vsha512rnds2" => {
            lowered("vsha512rnds2 ymm0, ymm1, xmm2", &["ymm0", "ymm1", "xmm2"], &["ymm0"], &[])
        }
        "vsm3msg1" | "vsm3msg2" => {
            lowered(format!("{op} xmm0, xmm1, xmm2"), &[x0, x1, "xmm2"], &[x0], &[])
        }
        "vsm4key4128" | "vsm4rnds4128" | "vsm4key4256" | "vsm4rnds4256" => {
            let bytes = if op.ends_with("256") { 32 } else { 16 };
            let (a, b) = (vector(bytes, 0), vector(bytes, 1));
            lowered(format!("{} {a}, {a}, {b}", &op[..op.len() - 3]), &[a, b], &[a], &[])
        }
        // Barriers, and the instructions on the machine's state of no operand.
        "sse.sfence" | "sse2.lfence" | "sse2.mfence" => {
            lowered(op.split('.').nth(1)?, &[], &[], &[])
        }
        "tilerelease" | "xend" => lowered(op, &[], &[], &[]),
        // Whether a transaction runs: the zero flag clear.
        "xtest" => lowered(format!("xtest\n{}", flag("ne")), &[], &["eax"], &[]),
        // Starts a transaction: all ones, or where it aborted to (the next instruction) the
        // status of the abort, which the processor leaves in `eax`.
        "xbegin" => lowered("mov eax, -1\nxbegin 2f\n2:", &[], &["eax"], &[]),
        // The time stamp counter, of `edx:eax`, and with `rdtscp` the processor's number.
        "rdtsc" => lowered("rdtsc\nshl rdx, 32\nor rax, rdx", &[], &["rax"], &["rdx"]),
        "rdtscp" => lowered("rdtscp\nshl rdx, 32\nor rax, rdx", &[], &["rax", "ecx"], &["rdx"]),
        // A random number and whether there was one: 1 with the carry flag, otherwise the
        // number, which the processor zeroes then, as LLVM selects it (`cmovb`).
        "rdrand.16" | "rdrand.32" | "rdrand.64" | "rdseed.16" | "rdseed.32" | "rdseed.64" => {
            let (mnemonic, bits) = op.split_once('.')?;
            let (value, low) = match bits {
                "16" => ("ax", "movzx ecx, ax"),
                "32" => ("eax", "mov ecx, eax"),
                _ => ("rax", "mov ecx, eax"),
            };
            let text = format!("{mnemonic} {value}\n{low}\nmov edx, 1\ncmovb ecx, edx");
            lowered(text, &[], &[value, "ecx"], &["edx"])
        }
        // Of the memory at a pointer.
        "sse2.clflush" | "clflushopt" => {
            lowered(format!("{} byte ptr [r8]", op.trim_start_matches("sse2.")), &["r8"], &[], &[])
        }
        "sse.ldmxcsr" | "sse.stmxcsr" => {
            lowered(format!("{} dword ptr [r8]", &op[4..]), &["r8"], &[], &[])
        }
        "fxsave" | "fxsave64" | "fxrstor" | "fxrstor64" | "ldtilecfg" | "sttilecfg" => {
            lowered(format!("{op} [r8]"), &["r8"], &[], &[])
        }
        "prefetchrs" => lowered("prefetchrst2 byte ptr [r8]", &["r8"], &[], &[]),
        "movrsqi" | "movrshi" | "movrssi" | "movrsdi" => {
            let (register, size) = match op {
                "movrsqi" => ("al", "byte"),
                "movrshi" => ("ax", "word"),
                "movrssi" => ("eax", "dword"),
                _ => ("rax", "qword"),
            };
            lowered(format!("movrs {register}, {size} ptr [r8]"), &["r8"], &[register], &[])
        }
        // The state components of the mask in `edx:eax`, saved to or restored from memory.
        "xsave" | "xsave64" | "xsaveopt" | "xsaveopt64" | "xsavec" | "xsavec64" | "xsaves"
        | "xsaves64" | "xrstor" | "xrstor64" | "xrstors" | "xrstors64" => {
            lowered(format!("{op} [r8]"), &["r8", "edx", "eax"], &[], &[])
        }
        "xsetbv" => lowered("xsetbv", &["ecx", "edx", "eax"], &[], &[]),
        // The bytes of the first operand where the second has its top bit set, written to
        // the memory `rdi` points at.
        "sse2.maskmov.dqu" => lowered("maskmovdqu xmm0, xmm1", &[x0, x1, "rdi"], &[], &[]),
        // The sum (difference) of the two and the carry (borrow) in, and the carry out: LLVM
        // makes the carry flag of a carry in that is not 0 by adding 255 to it.
        "addcarry.32" | "addcarry.64" | "subborrow.32" | "subborrow.64" => {
            let (a, b) = if op.ends_with("64") { ("rdx", "r8") } else { ("edx", "r8d") };
            let mnemonic = if op.starts_with("add") { "adc" } else { "sbb" };
            let text = format!("add cl, -1\n{mnemonic} {a}, {b}\nsetb al");
            lowered(text, &["cl", a, b], &["al", a], &[])
        }
        // BMI's: the bits of the first operand the second says.
        "bmi.bextr.32" | "bmi.bzhi.32" | "bmi.pdep.32" | "bmi.pext.32" => {
            let mnemonic = &op[4..op.len() - 3];
            lowered(format!("{mnemonic} eax, ecx, edx"), &["ecx", "edx"], &["eax"], &[])
        }
        "bmi.bextr.64" | "bmi.bzhi.64" | "bmi.pdep.64" | "bmi.pext.64" => {
            let mnemonic = &op[4..op.len() - 3];
            lowered(format!("{mnemonic} rax, rcx, rdx"), &["rcx", "rdx"], &["rax"], &[])
        }
        // Key Locker's AES of one block with the key of a handle, and whether the handle was
        // refused (the zero flag), in which case the block is zeroed.
        "aesenc128kl" | "aesdec128kl" | "aesenc256kl" | "aesdec256kl" => {
            let text = format!("{op} xmm0, [r8]\n{}", flag_byte("e"));
            lowered(text, &[x0, "r8"], &["al", x0], &[])
        }
        // Of eight blocks, `xmm0` to `xmm7`.
        "aesencwide128kl" | "aesdecwide128kl" | "aesencwide256kl" | "aesdecwide256kl" => {
            const BLOCKS: [&str; 8] =
                ["xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7"];
            let text = format!("{op} [r8]\n{}", flag_byte("e"));
            let mut args = vec!["r8"];
            args.extend(BLOCKS);
            let mut results = vec!["al"];
            results.extend(BLOCKS);
            lowered(text, &args, &results, &[])
        }
        // The handle of a key wrapped with the wrapping key, the control word's value, and
        // the registers the instruction zeroes, which LLVM takes as results in the order the
        // instruction defines them: `xmm3` is not one of them for a key of 128 bits.
        "encodekey128" => lowered(
            "encodekey128 eax, ecx",
            &["ecx", x0],
            &["eax", "xmm0", "xmm1", "xmm2", "xmm4", "xmm5", "xmm6"],
            &[],
        ),
        "encodekey256" => lowered(
            "encodekey256 eax, ecx",
            &["ecx", x0, x1],
            &["eax", "xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6"],
            &[],
        ),
        // The wrapping key, of the integrity key in `xmm0`, the two halves of the key and the
        // control word in `eax`.
        "loadiwkey" => lowered("loadiwkey xmm1, xmm2", &[x0, x1, "xmm2", "eax"], &[], &[]),
        _ => None,
    }
}

/// The flag an instruction set as a byte that is 0 or 1.
fn flag_byte(condition: &str) -> String {
    format!("set{condition} al")
}

/// `{sse.,sse2.}{comi,ucomi}{eq,neq,lt,le,gt,ge}.{ss,sd}` as its condition and the
/// instruction that compares.
fn comparison(op: &str) -> Option<(&str, String)> {
    let (family, rest) = op.split_once('.')?;
    if !matches!(family, "sse" | "sse2") {
        return None;
    }
    let (compare, lanes) = rest.split_once('.')?;
    if !matches!((family, lanes), ("sse", "ss") | ("sse2", "sd")) {
        return None;
    }
    let (mnemonic, condition) = match compare.strip_prefix("ucomi") {
        Some(condition) => ("ucomis", condition),
        None => ("comis", compare.strip_prefix("comi")?),
    };
    if !matches!(condition, "eq" | "neq" | "lt" | "le" | "gt" | "ge") {
        return None;
    }
    Some((condition, format!("{mnemonic}{}", &lanes[1..])))
}

/// `Lowered::text` for a list of names, for a test: each name and what it lowers to, or that
/// it does not.
#[cfg(test)]
fn describe(names: &[&str]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for name in names {
        match lower(name) {
            Some(it) => {
                _ = writeln!(
                    out,
                    "{name}: {} {:?} -> {:?}",
                    it.text.replace('\n', "; "),
                    it.args,
                    it.results
                )
            }
            None => _ = writeln!(out, "{name}: none"),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_name_is_the_instruction_llvm_selects_for_it() {
        let described = describe(&[
            "x86.sse.max.ss",
            "x86.sse2.psra.w",
            "x86.ssse3.pmul.hr.sw.128",
            "x86.ssse3.psign.d.128",
            "x86.aesni.aesdeclast",
            "x86.vgf2p8mulb.128",
            "x86.sse.rsqrt.ss",
            "x86.sse2.cvttpd2dq",
            "x86.sse.cvttss2si64",
            "x86.sse2.cvtsd2si",
            "x86.sse.comilt.ss",
            "x86.sse2.ucomineq.sd",
            "x86.sse41.ptestnzc",
            "x86.avx.vtestc.ps.256",
            "x86.avx.vtestnzc.pd",
            "x86.sse41.round.sd",
            "x86.avx.vpermilvar.ps.256",
            "x86.avx2.pmul.hr.sw",
            "x86.aesni.aesenc.512",
            "x86.avx2.psra.d",
            "x86.avx.cvtt.pd2dq.256",
            "x86.avx.rsqrt.ps.256",
            "x86.vsm4rnds4256",
            "x86.rdseed.16",
            "x86.sse.stmxcsr",
            "x86.movrshi",
            "x86.subborrow.64",
            "x86.bmi.pdep.32",
            "x86.sse.comilt.sd",
            "x86.avx512.max.ps.512",
        ]);
        let expected = "\
x86.sse.max.ss: maxss xmm0, xmm1 [Some(\"xmm0\"), Some(\"xmm1\")] -> [\"xmm0\"]
x86.sse2.psra.w: psraw xmm0, xmm1 [Some(\"xmm0\"), Some(\"xmm1\")] -> [\"xmm0\"]
x86.ssse3.pmul.hr.sw.128: pmulhrsw xmm0, xmm1 [Some(\"xmm0\"), Some(\"xmm1\")] -> [\"xmm0\"]
x86.ssse3.psign.d.128: psignd xmm0, xmm1 [Some(\"xmm0\"), Some(\"xmm1\")] -> [\"xmm0\"]
x86.aesni.aesdeclast: aesdeclast xmm0, xmm1 [Some(\"xmm0\"), Some(\"xmm1\")] -> [\"xmm0\"]
x86.vgf2p8mulb.128: gf2p8mulb xmm0, xmm1 [Some(\"xmm0\"), Some(\"xmm1\")] -> [\"xmm0\"]
x86.sse.rsqrt.ss: rsqrtss xmm0, xmm0 [Some(\"xmm0\")] -> [\"xmm0\"]
x86.sse2.cvttpd2dq: cvttpd2dq xmm0, xmm0 [Some(\"xmm0\")] -> [\"xmm0\"]
x86.sse.cvttss2si64: cvttss2si rax, xmm0 [Some(\"xmm0\")] -> [\"rax\"]
x86.sse2.cvtsd2si: cvtsd2si eax, xmm0 [Some(\"xmm0\")] -> [\"eax\"]
x86.sse.comilt.ss: comiss xmm1, xmm0; seta al; movzx eax, al [Some(\"xmm0\"), Some(\"xmm1\")] -> [\"eax\"]
x86.sse2.ucomineq.sd: ucomisd xmm0, xmm1; setp al; setne cl; or al, cl; movzx eax, al [Some(\"xmm0\"), Some(\"xmm1\")] -> [\"eax\"]
x86.sse41.ptestnzc: ptest xmm0, xmm1; seta al; movzx eax, al [Some(\"xmm0\"), Some(\"xmm1\")] -> [\"eax\"]
x86.avx.vtestc.ps.256: vtestps ymm0, ymm1; setb al; movzx eax, al [Some(\"ymm0\"), Some(\"ymm1\")] -> [\"eax\"]
x86.avx.vtestnzc.pd: vtestpd xmm0, xmm1; seta al; movzx eax, al [Some(\"xmm0\"), Some(\"xmm1\")] -> [\"eax\"]
x86.sse41.round.sd: roundsd xmm0, xmm1, {imm} [Some(\"xmm0\"), Some(\"xmm1\"), None] -> [\"xmm0\"]
x86.avx.vpermilvar.ps.256: vpermilps ymm0, ymm0, ymm1 [Some(\"ymm0\"), Some(\"ymm1\")] -> [\"ymm0\"]
x86.avx2.pmul.hr.sw: vpmulhrsw ymm0, ymm0, ymm1 [Some(\"ymm0\"), Some(\"ymm1\")] -> [\"ymm0\"]
x86.aesni.aesenc.512: vaesenc zmm0, zmm0, zmm1 [Some(\"zmm0\"), Some(\"zmm1\")] -> [\"zmm0\"]
x86.avx2.psra.d: vpsrad ymm0, ymm0, xmm1 [Some(\"ymm0\"), Some(\"xmm1\")] -> [\"ymm0\"]
x86.avx.cvtt.pd2dq.256: vcvttpd2dq xmm0, ymm0 [Some(\"ymm0\")] -> [\"xmm0\"]
x86.avx.rsqrt.ps.256: vrsqrtps ymm0, ymm0 [Some(\"ymm0\")] -> [\"ymm0\"]
x86.vsm4rnds4256: vsm4rnds4 ymm0, ymm0, ymm1 [Some(\"ymm0\"), Some(\"ymm1\")] -> [\"ymm0\"]
x86.rdseed.16: rdseed ax; movzx ecx, ax; mov edx, 1; cmovb ecx, edx [] -> [\"ax\", \"ecx\"]
x86.sse.stmxcsr: stmxcsr dword ptr [r8] [Some(\"r8\")] -> []
x86.movrshi: movrs ax, word ptr [r8] [Some(\"r8\")] -> [\"ax\"]
x86.subborrow.64: add cl, -1; sbb rdx, r8; setb al [Some(\"cl\"), Some(\"rdx\"), Some(\"r8\")] -> [\"al\", \"rdx\"]
x86.bmi.pdep.32: pdep eax, ecx, edx [Some(\"ecx\"), Some(\"edx\")] -> [\"eax\"]
x86.sse.comilt.sd: none
x86.avx512.max.ps.512: none
";
        assert_eq!(described, expected);
    }
}
