//! Integer-only IEEE casts. Guard and sticky bits round directly to target precision.
use super::*;
impl Emitter<'_> {
    pub(super) fn wide_float_cast(&mut self, from: ir::NumTy, to: ir::NumTy) -> Result<(), String> {
        match (from, to) {
            (ir::NumTy::Int(i), ir::NumTy::Float(f)) if i.bits() == 128 => {
                self.wf_int_to_float(i.is_signed(), f)
            }
            (ir::NumTy::Float(f), ir::NumTy::Int(i)) if i.bits() == 128 => {
                self.wf_float_to_int(f, i.is_signed())
            }
            _ => Err("wide float cast requires an IEEE float and a 128-bit integer".into()),
        }
    }
    fn wf_load(&mut self, opcode: &[u8], offset: i32) {
        self.bytes(opcode);
        self.i32(self.wide_scratch + offset);
    }
    fn wf_here(&mut self, branch: usize) -> Result<(), String> {
        self.patch(branch, self.position())
    }
    fn wf_int_to_float(&mut self, signed: bool, float: ir::FloatTy) -> Result<(), String> {
        let (p, bias, sign) = match float {
            ir::FloatTy::F32 => (24_u8, 127, 31_u8),
            ir::FloatTy::F64 => (53, 1023, 63),
        };
        self.wf_load(&[0x48, 0x8b, 0x85], 0);
        self.wf_load(&[0x48, 0x8b, 0x95], 8);
        self.bytes(&[0x45, 0x31, 0xc0]); // r8 sign, rdx:rax unsigned magnitude
        if signed {
            self.bytes(&[0x48, 0x85, 0xd2]);
            let positive = self.jump(&[0x0f, 0x89]);
            self.bytes(&[
                0x41, 0xb8, 1, 0, 0, 0, 0x48, 0xf7, 0xd8, 0x48, 0x83, 0xd2, 0, 0x48, 0xf7, 0xda,
            ]);
            self.wf_here(positive)?;
        }
        self.bytes(&[0x45, 0x31, 0xc9, 0x45, 0x31, 0xd2, 0x45, 0x31, 0xdb]); // r9 shifts, r10 sticky, r11 guard
        let normalize = self.position();
        self.bytes(&[0x48, 0x85, 0xd2]);
        let high = self.jump(&[0x0f, 0x85]);
        self.bytes(&[0x48, 0x89, 0xc1, 0x48, 0xc1, 0xe9, p, 0x48, 0x85, 0xc9]);
        let normalized = self.jump(&[0x0f, 0x84]);
        self.wf_here(high)?;
        self.bytes(&[0x4d, 0x09, 0xda, 0x49, 0x89, 0xc3, 0x41, 0x83, 0xe3, 1]);
        self.bytes(&[
            0x48, 0x0f, 0xac, 0xd0, 1, 0x48, 0xd1, 0xea, 0x49, 0xff, 0xc1,
        ]);
        let again = self.jump(&[0xe9]);
        self.patch(again, normalize)?;
        self.wf_here(normalized)?;
        self.bytes(&[0x48, 0x85, 0xc0]);
        let zero = self.jump(&[0x0f, 0x84]);
        self.bytes(&[0x48, 0x0f, 0xbd, 0xc8, 0x49, 0x01, 0xc9]); // exponent = retained MSB + shifts
        self.bytes(&[0x4d, 0x85, 0xdb]);
        let no_guard = self.jump(&[0x0f, 0x84]);
        self.bytes(&[0x48, 0x89, 0xc1, 0x83, 0xe1, 1, 0x4c, 0x09, 0xd1]);
        let even = self.jump(&[0x0f, 0x84]);
        self.bytes(&[
            0x48, 0xff, 0xc0, 0x48, 0x89, 0xc1, 0x48, 0xc1, 0xe9, p, 0x48, 0x85, 0xc9,
        ]);
        let carry = self.jump(&[0x0f, 0x84]);
        self.bytes(&[0x48, 0xd1, 0xe8, 0x49, 0xff, 0xc1]);
        self.wf_here(carry)?;
        self.wf_here(no_guard)?;
        self.wf_here(even)?;
        self.bytes(&[0x48, 0x0f, 0xbd, 0xc8, 0x48, 0xf7, 0xd9, 0x48, 0x81, 0xc1]);
        self.i32(i32::from(p - 1));
        self.bytes(&[
            0x48,
            0xd3,
            0xe0,
            0x48,
            0x0f,
            0xba,
            0xf0,
            p - 1,
            0x49,
            0x81,
            0xc1,
        ]);
        self.i32(bias);
        self.bytes(&[
            0x49,
            0xc1,
            0xe1,
            p - 1,
            0x4c,
            0x09,
            0xc8,
            0x49,
            0xc1,
            0xe0,
            sign,
            0x4c,
            0x09,
            0xc0,
        ]);
        self.wf_here(zero)?;
        Ok(())
    }
    fn wf_float_to_int(&mut self, float: ir::FloatTy, signed: bool) -> Result<(), String> {
        let (fraction, bias, mask, sign) = match float {
            ir::FloatTy::F32 => (23_u8, 127, 255, 31_u8),
            ir::FloatTy::F64 => (52, 1023, 2047, 63),
        };
        self.wf_load(&[0x48, 0x8b, 0x85], 0);
        self.bytes(&[
            0x49, 0x89, 0xc0, 0x49, 0xc1, 0xe8, sign, 0x49, 0x89, 0xc1, 0x49, 0xc1, 0xe9, fraction,
            0x41, 0x81, 0xe1,
        ]);
        self.i32(mask);
        self.bytes(&[
            0x48,
            0x89,
            0xc2,
            0x48,
            0xc1,
            0xe2,
            64 - fraction,
            0x48,
            0xc1,
            0xea,
            64 - fraction,
            0x41,
            0x81,
            0xf9,
        ]);
        self.i32(mask);
        let finite = self.jump(&[0x0f, 0x85]);
        self.bytes(&[0x48, 0x85, 0xd2]);
        let nan = self.jump(&[0x0f, 0x85]);
        let infinity = self.jump(&[0xe9]);
        self.wf_here(finite)?;
        let mut zeros = vec![nan];
        if !signed {
            self.bytes(&[0x4d, 0x85, 0xc0]);
            zeros.push(self.jump(&[0x0f, 0x85]));
        }
        self.bytes(&[0x49, 0x81, 0xe9]);
        self.i32(bias);
        zeros.push(self.jump(&[0x0f, 0x88]));
        self.bytes(&[0x49, 0x81, 0xf9]);
        self.i32(if signed { 127 } else { 128 });
        let overflow = self.jump(&[0x0f, 0x83]);
        self.bytes(&[
            0x48, 0x89, 0xd0, 0x48, 0x0f, 0xba, 0xe8, fraction, 0x31, 0xd2, 0x4c, 0x89, 0xc9, 0x48,
            0x81, 0xe9,
        ]);
        self.i32(i32::from(fraction));
        let left = self.jump(&[0x0f, 0x89]);
        self.bytes(&[0x48, 0xf7, 0xd9, 0x48, 0xd3, 0xe8]);
        let right_done = self.jump(&[0xe9]);
        self.wf_here(left)?;
        self.bytes(&[0x48, 0x85, 0xc9]);
        let no_shift = self.jump(&[0x0f, 0x84]);
        let shift = self.position();
        self.bytes(&[0x48, 0xd1, 0xe0, 0x48, 0xd1, 0xd2, 0x48, 0xff, 0xc9]);
        let again = self.jump(&[0x0f, 0x85]);
        self.patch(again, shift)?;
        self.wf_here(no_shift)?;
        self.wf_here(right_done)?;
        if signed {
            self.bytes(&[0x4d, 0x85, 0xc0]);
            let positive = self.jump(&[0x0f, 0x84]);
            self.bytes(&[0x48, 0xf7, 0xd8, 0x48, 0x83, 0xd2, 0, 0x48, 0xf7, 0xda]);
            self.wf_here(positive)?;
        }
        let normal = self.jump(&[0xe9]);
        self.wf_here(infinity)?;
        self.wf_here(overflow)?;
        self.bytes(&[0x4d, 0x85, 0xc0]);
        let negative = self.jump(&[0x0f, 0x85]);
        self.word(u64::MAX);
        self.bytes(&[0x48, 0x89, 0xc2]);
        if signed {
            self.bytes(&[0x48, 0xd1, 0xea]);
        }
        let maximum = self.jump(&[0xe9]);
        self.wf_here(negative)?;
        let minimum = if signed {
            self.word(1_u64 << 63);
            self.bytes(&[0x48, 0x89, 0xc2, 0x31, 0xc0]);
            Some(self.jump(&[0xe9]))
        } else {
            None
        };
        for branch in zeros {
            self.wf_here(branch)?;
        }
        self.bytes(&[0x31, 0xc0, 0x31, 0xd2]);
        self.wf_here(normal)?;
        self.wf_here(maximum)?;
        if let Some(branch) = minimum {
            self.wf_here(branch)?;
        }
        self.wf_load(&[0x48, 0x89, 0x85], 32);
        self.wf_load(&[0x48, 0x89, 0x95], 40);
        self.address(self.wide_scratch + 32);
        Ok(())
    }
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::*;
    use crate::compiler::frontend;
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    // The oracle is the bootstrap Rust compiler's casts, not a second copy of
    // the emitter algorithm. Test actual emitted instructions, not a model.
    #[test]
    fn wide_float_native_cast_oracles() {
        let program = frontend::parse("fn main() {}").unwrap();
        let signatures = frontend::signature_table(&program).unwrap();
        let checked = frontend::type_function_with_signatures(&program, 0, &signatures).unwrap();
        let plan = signatures.plan().unwrap();
        let facts = plan
            .instance_facts(&ir::CallTarget {
                declaration: plan.callable_decl(0).unwrap(),
                type_args: [].into(),
            })
            .unwrap();
        let concrete = seal_concrete_cfg(
            cfg::lower_with_facts(&checked, &facts, facts.budget()).unwrap(),
            facts,
        )
        .unwrap();
        let resolve = |_: &ir::CallTarget| -> Result<String, String> { Err("no calls".into()) };
        let mut e = Emitter {
            object: Object {
                code: Vec::new(),
                calls: Vec::new(),
                strings: Vec::new(),
                vtables: Vec::new(),
                exports: Vec::new(),
                receipt: None,
            },
            concrete: &concrete,
            resolve: &resolve,
            drop_flags: Vec::new(),
            path_flags: Vec::new(),
            locals: Vec::new(),
            aggregate_scratch: -256,
            wide_scratch: -256,
            vtable_scratch: 0,
            returns: Vec::new(),
            traps: Vec::new(),
            edges: Vec::new(),
        };
        for (from, to) in [
            (
                ir::NumTy::Int(ir::IntTy::I64),
                ir::NumTy::Float(ir::FloatTy::F64),
            ),
            (
                ir::NumTy::Int(ir::IntTy::I128),
                ir::NumTy::Int(ir::IntTy::U128),
            ),
            (
                ir::NumTy::Float(ir::FloatTy::F32),
                ir::NumTy::Float(ir::FloatTy::F64),
            ),
        ] {
            assert!(e.wide_float_cast(from, to).is_err());
            assert!(e.object.code.is_empty(), "invalid cast emitted code");
        }
        e.bytes(&[0x55, 0x48, 0x89, 0xe5, 0x48, 0x81, 0xec]);
        e.i32(256);
        let mut failures = Vec::new();
        let mut case_id = 0_u32;
        let mut check = |e: &mut Emitter<'_>, from, to, input: u128, expected: u128| {
            case_id += 1;
            e.word(input as u64);
            e.wf_load(&[0x48, 0x89, 0x85], 0);
            e.word((input >> 64) as u64);
            e.wf_load(&[0x48, 0x89, 0x85], 8);
            e.wide_float_cast(from, to).unwrap();
            if matches!(to, ir::NumTy::Int(_)) {
                e.bytes(&[0x48, 0x8b, 0x50, 8, 0x48, 0x8b, 0x00]);
            }
            e.bytes(&[0x49, 0x89, 0xc3]); // save low
            e.word(expected as u64);
            e.bytes(&[0x49, 0x39, 0xc3]);
            failures.push((e.jump(&[0x0f, 0x85]), case_id));
            if matches!(to, ir::NumTy::Int(_)) {
                e.word((expected >> 64) as u64);
                e.bytes(&[0x48, 0x39, 0xc2]);
                failures.push((e.jump(&[0x0f, 0x85]), case_id));
            }
        };
        let mut integers = vec![0, 1, 2, 3, u128::MAX, i128::MAX as u128, i128::MIN as u128];
        for bit in 1..128 {
            let n = 1_u128 << bit;
            integers.extend([n - 1, n, n + 1]);
        }
        // Midpoints, sticky bits and a direct-f32 double-rounding witness.
        integers.extend([
            (1_u128 << 100) + (1_u128 << 76) + 1,
            (1_u128 << 100) + (1_u128 << 76),
            (1_u128 << 100) + (3_u128 << 76),
        ]);
        let mut random = 0xd728_82ca_fedc_1234_u128;
        for _ in 0..128 {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 43;
            integers.push(random);
        }
        for value in integers {
            for signed in [false, true] {
                let from = ir::NumTy::Int(if signed {
                    ir::IntTy::I128
                } else {
                    ir::IntTy::U128
                });
                let f32_bits = if signed {
                    (value as i128 as f32).to_bits()
                } else {
                    (value as f32).to_bits()
                };
                let f64_bits = if signed {
                    (value as i128 as f64).to_bits()
                } else {
                    (value as f64).to_bits()
                };
                check(
                    &mut e,
                    from,
                    ir::NumTy::Float(ir::FloatTy::F32),
                    value,
                    u128::from(f32_bits),
                );
                check(
                    &mut e,
                    from,
                    ir::NumTy::Float(ir::FloatTy::F64),
                    value,
                    u128::from(f64_bits),
                );
            }
        }
        for float in [ir::FloatTy::F32, ir::FloatTy::F64] {
            let fraction = if float == ir::FloatTy::F32 { 23 } else { 52 };
            let max_exp = if fraction == 23 { 255 } else { 2047 };
            let sign_bit = if fraction == 23 { 31 } else { 63 };
            let mut inputs = Vec::new();
            for exponent in 0..=max_exp {
                if exponent > 260 && exponent < 1000 && exponent != max_exp {
                    continue;
                }
                for mantissa in [0, 1, (1_u64 << fraction) - 1, 1_u64 << (fraction - 1)] {
                    for sign in [0, 1_u64 << sign_bit] {
                        inputs.push(sign | ((exponent as u64) << fraction) | mantissa);
                    }
                }
            }
            for bits in inputs {
                let (unsigned, signed) = if fraction == 23 {
                    let f = f32::from_bits(bits as u32);
                    (f as u128, f as i128)
                } else {
                    let f = f64::from_bits(bits);
                    (f as u128, f as i128)
                };
                check(
                    &mut e,
                    ir::NumTy::Float(float),
                    ir::NumTy::Int(ir::IntTy::U128),
                    u128::from(bits),
                    unsigned,
                );
                check(
                    &mut e,
                    ir::NumTy::Float(float),
                    ir::NumTy::Int(ir::IntTy::I128),
                    u128::from(bits),
                    signed as u128,
                );
            }
        }
        // Return a case index on failure, zero on success, through stdout.
        e.bytes(&[0x31, 0xc0]);
        let success = e.jump(&[0xe9]);
        let mut finish = Vec::new();
        for (branch, index) in failures {
            e.wf_here(branch).unwrap();
            e.word(u64::from(index));
            finish.push(e.jump(&[0xe9]));
        }
        e.wf_here(success).unwrap();
        for branch in finish {
            e.wf_here(branch).unwrap();
        }
        e.wf_load(&[0x48, 0x89, 0x85], 48);
        e.address(-208);
        e.bytes(&[
            0x48, 0x89, 0xc6, 0xbf, 1, 0, 0, 0, 0xba, 8, 0, 0, 0, 0xb8, 1, 0, 0, 0, 0x0f, 0x05,
        ]);
        e.bytes(&[0x31, 0xff, 0xb8, 60, 0, 0, 0, 0x0f, 0x05]);
        let elf = link(&[("main".into(), e.object)]).unwrap();
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = root.join(format!("jr-wide-float-oracle-{}", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&path)
            .unwrap();
        file.write_all(&elf).unwrap();
        drop(file);
        // A concurrent fork can briefly inherit another test's writable image.
        // Retry only ETXTBSY before spawn succeeds, never an executed child.
        let output = (0..100)
            .find_map(|attempt| match std::process::Command::new(&path).output() {
                Err(error) if error.raw_os_error() == Some(26) && attempt < 99 => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    None
                }
                result => Some(result),
            })
            .expect("bounded spawn loop returns on its final attempt")
            .unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(
            output.status.success(),
            "native oracle status {:?}",
            output.status
        );
        assert_eq!(output.stdout.len(), 8);
        let failed = u64::from_le_bytes(output.stdout.try_into().unwrap());
        assert_eq!(
            failed, 0,
            "wide float oracle failed case {failed} of {case_id}"
        );
    }
}
