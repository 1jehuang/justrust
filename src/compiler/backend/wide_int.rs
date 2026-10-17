//! Native checked two-limb integer operations. All offsets are within wide_scratch.
use super::*;

impl Emitter<'_> {
    // Register operands use architectural register numbers. Memory is always rbp+disp32.
    fn wi_load(&mut self, reg: u8, offset: i32) {
        self.bytes(&[0x48 | ((reg >> 3) << 2), 0x8b, 0x85 | ((reg & 7) << 3)]);
        self.i32(self.wide_scratch + offset);
    }
    fn wi_store(&mut self, offset: i32, reg: u8) {
        self.bytes(&[0x48 | ((reg >> 3) << 2), 0x89, 0x85 | ((reg & 7) << 3)]);
        self.i32(self.wide_scratch + offset);
    }
    fn wi_rr(&mut self, opcode: u8, dst: u8, src: u8) {
        self.bytes(&[
            0x48 | ((src >> 3) << 2) | (dst >> 3),
            opcode,
            0xc0 | ((src & 7) << 3) | (dst & 7),
        ]);
    }
    fn wi_zero(&mut self, offset: i32) {
        self.bytes(&[0x31, 0xc0]);
        self.wi_store(offset, 0);
    }
    fn wi_neg(&mut self, offset: i32) {
        self.wi_load(0, offset);
        self.wi_load(2, offset + 8);
        self.bytes(&[0x48, 0xf7, 0xd8, 0x48, 0x83, 0xd2, 0x00, 0x48, 0xf7, 0xda]);
        self.wi_store(offset, 0);
        self.wi_store(offset + 8, 2);
    }
    fn wi_abs(&mut self, offset: i32) -> Result<(), String> {
        self.wi_load(0, offset + 8);
        self.wi_rr(0x85, 0, 0);
        let nonnegative = self.jump(&[0x0f, 0x89]);
        self.wi_neg(offset);
        self.patch(nonnegative, self.position())
    }
    fn wi_result(&mut self) {
        self.address(self.wide_scratch + 32);
    }
    pub(super) fn wide_unary(&mut self, op: UnaryOp, ty: ir::IntTy) -> Result<(), String> {
        self.wi_load(0, 0);
        self.wi_load(2, 8);
        match op {
            UnaryOp::Not => self.bytes(&[0x48, 0xf7, 0xd0, 0x48, 0xf7, 0xd2]),
            UnaryOp::Neg => {
                if ty.is_signed() {
                    // Negate both limbs. Only MIN is forbidden, tested below.
                    self.bytes(&[0x48, 0xf7, 0xd8, 0x48, 0x83, 0xd2, 0x00, 0x48, 0xf7, 0xda]);
                    // NEG high alone can overflow for valid negative values with nonzero low.
                    // The sole forbidden input is MIN, so test the original pair instead.
                    self.wi_store(32, 0);
                    self.wi_store(40, 2);
                    self.wi_load(0, 0);
                    self.wi_rr(0x85, 0, 0);
                    let low_nonzero = self.jump(&[0x0f, 0x85]);
                    self.word(1u64 << 63);
                    self.wi_load(2, 8);
                    self.wi_rr(0x39, 2, 0);
                    self.trap_if(0x84);
                    self.patch(low_nonzero, self.position())?;
                    self.wi_result();
                    return Ok(());
                }
                self.wi_rr(0x09, 0, 2);
                self.trap_if(0x85);
                self.bytes(&[0x31, 0xc0, 0x31, 0xd2]);
            }
        }
        self.wi_store(32, 0);
        self.wi_store(40, 2);
        self.wi_result();
        Ok(())
    }

    pub(super) fn wide_binary(
        &mut self,
        op: BinaryOp,
        ty: ir::IntTy,
        _rhs: ir::IntTy,
    ) -> Result<(), String> {
        use BinaryOp::*;
        // Canonical sign/zero extension makes the exact RHS type unnecessary:
        // negative counts have a nonzero high limb, and all positive counts
        // are checked in full before x86's masked shift-count instructions.
        if ty.bits() < 128 {
            if !matches!(op, Shl | Shr) {
                return Err("wide RHS requires a shift for a narrow LHS".into());
            }
            self.wi_load(1, 24);
            self.wi_rr(0x85, 1, 1);
            self.trap_if(0x85);
            self.wi_load(1, 16);
            self.bytes(&[0x48, 0x83, 0xf9, (ty.bits() - 1) as u8]);
            self.trap_if(0x87);
            self.wi_load(0, 0);
            self.bytes(&[
                0x48,
                0xd3,
                if op == Shl {
                    0xe0
                } else if ty.is_signed() {
                    0xf8
                } else {
                    0xe8
                },
            ]);
            self.normalize_integer(ty);
            return Ok(());
        }
        match op {
            Mul => return self.wi_mul(ty),
            Div | Rem => return self.wi_div(op, ty),
            Shl | Shr => return self.wi_shift(op, ty),
            Eq | Ne | Lt | Le | Gt | Ge => return self.wi_compare(op, ty),
            Add | Sub | BitAnd | BitOr | BitXor => {}
            And | Or => return Err("logical operator on wide integer".into()),
        }
        self.wi_load(0, 0);
        self.wi_load(2, 8);
        self.wi_load(1, 16);
        self.wi_load(3, 24);
        match op {
            Add | Sub => {
                self.wi_rr(if op == Add { 0x01 } else { 0x29 }, 0, 1);
                self.wi_rr(if op == Add { 0x11 } else { 0x19 }, 2, 3);
                self.trap_if(if ty.is_signed() { 0x80 } else { 0x82 });
            }
            _ => {
                let opcode = match op {
                    BitAnd => 0x21,
                    BitOr => 0x09,
                    _ => 0x31,
                };
                self.wi_rr(opcode, 0, 1);
                self.wi_rr(opcode, 2, 3);
            }
        }
        self.wi_store(32, 0);
        self.wi_store(40, 2);
        self.wi_result();
        Ok(())
    }

    fn wi_compare(&mut self, op: BinaryOp, ty: ir::IntTy) -> Result<(), String> {
        use BinaryOp::*;
        self.wi_load(0, 8);
        self.wi_load(2, 24);
        self.wi_rr(0x39, 0, 2);
        let different = self.jump(&[0x0f, 0x85]);
        self.wi_load(0, 0);
        self.wi_load(2, 16);
        self.wi_rr(0x39, 0, 2);
        let low_cc = match op {
            Eq => 0x94,
            Ne => 0x95,
            Lt => 0x92,
            Le => 0x96,
            Gt => 0x97,
            Ge => 0x93,
            _ => unreachable!(),
        };
        self.bytes(&[0x0f, low_cc, 0xc0]);
        let done = self.jump(&[0xe9]);
        self.patch(different, self.position())?;
        let high_cc = if ty.is_signed() {
            match op {
                Lt => 0x9c,
                Le => 0x9e,
                Gt => 0x9f,
                Ge => 0x9d,
                _ => low_cc,
            }
        } else {
            low_cc
        };
        self.bytes(&[0x0f, high_cc, 0xc0]);
        self.patch(done, self.position())?;
        self.bytes(&[0x0f, 0xb6, 0xc0]);
        Ok(())
    }

    fn wi_shift(&mut self, op: BinaryOp, ty: ir::IntTy) -> Result<(), String> {
        self.wi_load(1, 24);
        self.wi_rr(0x85, 1, 1);
        self.trap_if(0x85);
        self.wi_load(1, 16);
        self.bytes(&[0x48, 0x83, 0xf9, 0x7f]);
        self.trap_if(0x87);
        self.wi_load(0, 0);
        self.wi_load(2, 8);
        self.bytes(&[0x48, 0x83, 0xf9, 0x40]);
        let small = self.jump(&[0x0f, 0x82]);
        if op == BinaryOp::Shl {
            self.wi_rr(0x89, 2, 0);
            self.bytes(&[0x31, 0xc0, 0x48, 0xd3, 0xe2]);
        } else {
            self.wi_rr(0x89, 0, 2);
            self.bytes(&[0x48, 0xd3, if ty.is_signed() { 0xf8 } else { 0xe8 }]);
            if ty.is_signed() {
                self.bytes(&[0x48, 0xc1, 0xfa, 0x3f]);
            } else {
                self.bytes(&[0x31, 0xd2]);
            }
        }
        let done = self.jump(&[0xe9]);
        self.patch(small, self.position())?;
        if op == BinaryOp::Shl {
            self.bytes(&[0x48, 0x0f, 0xa5, 0xc2, 0x48, 0xd3, 0xe0]);
        } else {
            self.bytes(&[
                0x48,
                0x0f,
                0xad,
                0xd0,
                0x48,
                0xd3,
                if ty.is_signed() { 0xfa } else { 0xea },
            ]);
        }
        self.patch(done, self.position())?;
        self.wi_store(32, 0);
        self.wi_store(40, 2);
        self.wi_result();
        Ok(())
    }

    fn wi_mul(&mut self, ty: ir::IntTy) -> Result<(), String> {
        if ty.is_signed() {
            self.wi_load(0, 8);
            self.wi_load(2, 24);
            self.wi_rr(0x31, 0, 2);
            self.wi_store(240, 0);
            self.wi_abs(0)?;
            self.wi_abs(16)?;
        }
        for (dst, src) in [(64, 0), (72, 8)] {
            self.wi_load(0, src);
            self.wi_store(dst, 0);
        }
        for offset in [80, 88, 96, 104, 112, 120] {
            self.wi_zero(offset);
        }
        self.bytes(&[0xb9, 0x80, 0, 0, 0]);
        let start = self.position();
        self.wi_load(0, 16);
        self.bytes(&[0xa8, 1]);
        let skip = self.jump(&[0x0f, 0x84]);
        for limb in 0..4 {
            self.wi_load(0, 96 + limb * 8);
            self.wi_load(2, 64 + limb * 8);
            self.wi_rr(if limb == 0 { 0x01 } else { 0x11 }, 0, 2);
            self.wi_store(96 + limb * 8, 0);
        }
        self.patch(skip, self.position())?;
        for limb in 0..4 {
            self.wi_load(0, 64 + limb * 8);
            self.bytes(&[0x48, 0xd1, if limb == 0 { 0xe0 } else { 0xd0 }]);
            self.wi_store(64 + limb * 8, 0);
        }
        self.wi_load(0, 16);
        self.wi_load(2, 24);
        self.bytes(&[0x48, 0x0f, 0xac, 0xd0, 1, 0x48, 0xd1, 0xea]);
        self.wi_store(16, 0);
        self.wi_store(24, 2);
        self.bytes(&[0xff, 0xc9]);
        let again = self.jump(&[0x0f, 0x85]);
        self.patch(again, start)?;
        self.wi_load(0, 112);
        self.wi_load(2, 120);
        self.wi_rr(0x09, 0, 2);
        self.trap_if(0x85);
        self.wi_load(0, 96);
        self.wi_store(32, 0);
        self.wi_load(2, 104);
        self.wi_store(40, 2);
        if ty.is_signed() {
            self.wi_load(0, 240);
            self.wi_rr(0x85, 0, 0);
            let positive = self.jump(&[0x0f, 0x89]);
            self.word(1u64 << 63);
            self.wi_rr(0x39, 2, 0);
            self.trap_if(0x87);
            let below = self.jump(&[0x0f, 0x82]);
            self.wi_load(0, 32);
            self.wi_rr(0x85, 0, 0);
            self.trap_if(0x85);
            self.patch(below, self.position())?;
            self.wi_neg(32);
            let done = self.jump(&[0xe9]);
            self.patch(positive, self.position())?;
            self.wi_rr(0x85, 2, 2);
            self.trap_if(0x88);
            self.patch(done, self.position())?;
        }
        self.wi_result();
        Ok(())
    }

    fn wi_div(&mut self, op: BinaryOp, ty: ir::IntTy) -> Result<(), String> {
        self.wi_load(0, 16);
        self.wi_load(2, 24);
        self.wi_rr(0x09, 0, 2);
        self.trap_if(0x84);
        if ty.is_signed() {
            self.wi_load(0, 8);
            self.wi_store(232, 0); // remainder sign
            self.wi_load(2, 24);
            self.wi_rr(0x31, 0, 2);
            self.wi_store(240, 0); // quotient sign
            // MIN / -1 and MIN % -1 both overflow.
            self.wi_load(0, 0);
            self.wi_rr(0x85, 0, 0);
            let not_min_low = self.jump(&[0x0f, 0x85]);
            self.word(1u64 << 63);
            self.wi_load(2, 8);
            self.wi_rr(0x39, 2, 0);
            let not_min_high = self.jump(&[0x0f, 0x85]);
            self.wi_load(0, 16);
            self.bytes(&[0x48, 0x83, 0xf8, 0xff]);
            let not_minus_low = self.jump(&[0x0f, 0x85]);
            self.wi_load(0, 24);
            self.bytes(&[0x48, 0x83, 0xf8, 0xff]);
            self.trap_if(0x84);
            let end = self.position();
            for at in [not_min_low, not_min_high, not_minus_low] {
                self.patch(at, end)?;
            }
            self.wi_abs(0)?;
            self.wi_abs(16)?;
        }
        // rdx:rax remainder, rdi:rsi dividend/quotient, r9:r8 divisor.
        self.wi_load(6, 0);
        self.wi_load(7, 8);
        self.wi_load(8, 16);
        self.wi_load(9, 24);
        self.bytes(&[0x31, 0xc0, 0x31, 0xd2, 0xb9, 0x80, 0, 0, 0]);
        let start = self.position();
        self.bytes(&[
            0x48, 0xd1, 0xe6, 0x48, 0xd1, 0xd7, 0x48, 0xd1, 0xd0, 0x48, 0xd1, 0xd2,
        ]);
        // Carry is the 129th remainder bit and mandates subtraction.
        let carry = self.jump(&[0x0f, 0x82]);
        self.wi_rr(0x39, 2, 9);
        let high_above = self.jump(&[0x0f, 0x87]);
        let high_below = self.jump(&[0x0f, 0x82]);
        self.wi_rr(0x39, 0, 8);
        let low_below = self.jump(&[0x0f, 0x82]);
        let subtract = self.position();
        self.patch(carry, subtract)?;
        self.patch(high_above, subtract)?;
        self.wi_rr(0x29, 0, 8);
        self.wi_rr(0x19, 2, 9);
        self.bytes(&[0x48, 0x83, 0xce, 1]);
        let next = self.position();
        self.patch(high_below, next)?;
        self.patch(low_below, next)?;
        self.bytes(&[0xff, 0xc9]);
        let again = self.jump(&[0x0f, 0x85]);
        self.patch(again, start)?;
        if op == BinaryOp::Div {
            self.wi_store(32, 6);
            self.wi_store(40, 7);
        } else {
            self.wi_store(32, 0);
            self.wi_store(40, 2);
        }
        if ty.is_signed() {
            self.wi_load(0, if op == BinaryOp::Div { 240 } else { 232 });
            self.wi_rr(0x85, 0, 0);
            let positive = self.jump(&[0x0f, 0x89]);
            self.wi_neg(32);
            self.patch(positive, self.position())?;
        }
        self.wi_result();
        Ok(())
    }
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::*;
    use crate::compiler::frontend;
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    use std::sync::atomic::{AtomicU64, Ordering};

    // Match the parent backend test runner: retry only failure to spawn with
    // Linux ETXTBSY, never a started executable or any other execution error.
    fn spawn_image<T>(mut spawn: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
        for attempt in 0..100 {
            match spawn() {
                Err(error) if error.raw_os_error() == Some(26) && attempt < 99 => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                result => return result,
            }
        }
        unreachable!("bounded spawn loop always returns on its final attempt")
    }

    #[derive(Clone, Copy, Debug)]
    struct Case {
        op: BinaryOp,
        ty: ir::IntTy,
        a: u128,
        b: u128,
        expected: u128,
    }

    fn run(cases: &[Case], unary: &[(UnaryOp, ir::IntTy, u128, u128)], check_results: bool) -> i32 {
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
        let resolve = |_: &ir::CallTarget| Err("no calls".into());
        let mut e = Emitter {
            object: Object {
                code: Vec::new(),
                calls: Vec::new(),
                strings: Vec::new(),
            },
            concrete: &concrete,
            resolve: &resolve,
            locals: Vec::new(),
            aggregate_scratch: 0,
            wide_scratch: -256,
            returns: Vec::new(),
            traps: Vec::new(),
            edges: Vec::new(),
        };
        e.bytes(&[0x55, 0x48, 0x89, 0xe5, 0x48, 0x81, 0xec]);
        e.i32(256);
        fn put(e: &mut Emitter<'_>, slot: i32, value: u128) {
            e.word(value as u64);
            e.wi_store(slot, 0);
            e.word((value >> 64) as u64);
            e.wi_store(slot + 8, 0);
        }
        fn check(e: &mut Emitter<'_>, value: u128, indirect: bool) {
            if indirect {
                // Check pointer convention as well as both result limbs.
                e.wi_rr(0x89, 2, 0);
                e.address(e.wide_scratch + 32);
                e.wi_rr(0x39, 0, 2);
                e.trap_if(0x85);
                e.wi_load(2, 40);
                e.word((value >> 64) as u64);
                e.wi_rr(0x39, 2, 0);
                e.trap_if(0x85);
                e.wi_load(0, 32);
            }
            e.wi_rr(0x89, 2, 0);
            e.word(value as u64);
            e.wi_rr(0x39, 2, 0);
            e.trap_if(0x85);
        }
        for case in cases {
            put(&mut e, 0, case.a);
            put(&mut e, 16, case.b);
            e.wide_binary(case.op, case.ty, ir::IntTy::U128).unwrap();
            let comparison = matches!(
                case.op,
                BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
            );
            if check_results {
                check(&mut e, case.expected, !comparison && case.ty.bits() == 128);
            }
        }
        for &(op, ty, value, expected) in unary {
            put(&mut e, 0, value);
            e.wide_unary(op, ty).unwrap();
            if check_results {
                check(&mut e, expected, true);
            }
        }
        e.bytes(&[0x31, 0xff, 0xb8, 60, 0, 0, 0, 0x0f, 0x05]);
        e.trap_block().unwrap();
        let elf = link(&[("main".into(), e.object)]).unwrap();
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(std::env::temp_dir);
        static NEXT_IMAGE: AtomicU64 = AtomicU64::new(0);
        let dir = root.join(format!(
            "jr-wide-oracle-{}-{}-{}",
            std::process::id(),
            NEXT_IMAGE.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let path = dir.join("program");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&path)
            .unwrap();
        file.write_all(&elf).unwrap();
        drop(file);
        // Closing our writer does not close a descriptor transiently inherited
        // by another concurrently spawning test. Its next exec closes CLOEXEC
        // descriptors. Retry only that pre-execution OS error, as the parent
        // native backend harness does, without rerunning any executed image.
        let mut command = std::process::Command::new(&path);
        let status = spawn_image(|| command.spawn()).unwrap().wait().unwrap();
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
        status.code().unwrap_or(-1)
    }

    #[test]
    fn wide_native_oracle_spawn_policy() {
        let mut attempts = 0;
        let result = spawn_image(|| {
            attempts += 1;
            if attempts < 3 {
                Err(std::io::Error::from_raw_os_error(26))
            } else {
                Ok(7)
            }
        })
        .unwrap();
        assert_eq!(result, 7);
        assert_eq!(attempts, 3);
        for errno in [2, 8, 13] {
            let mut attempts = 0;
            let result: std::io::Result<()> = spawn_image(|| {
                attempts += 1;
                Err(std::io::Error::from_raw_os_error(errno))
            });
            assert_eq!(result.unwrap_err().raw_os_error(), Some(errno));
            assert_eq!(attempts, 1);
        }
        let mut attempts = 0;
        let result: std::io::Result<()> = spawn_image(|| {
            attempts += 1;
            Err(std::io::Error::from_raw_os_error(26))
        });
        assert_eq!(result.unwrap_err().raw_os_error(), Some(26));
        assert_eq!(attempts, 100);
    }

    #[test]
    fn wide_native_oracle_parallel_images() {
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..8 {
                        assert_eq!(run(&[], &[], true), 0);
                    }
                });
            }
        });
    }

    #[test]
    fn wide_native_oracle_success() {
        use BinaryOp::*;
        use ir::IntTy::*;
        let mut cases = Vec::new();
        let mut values = vec![
            0,
            1,
            2,
            3,
            u64::MAX as u128,
            1u128 << 64,
            (1u128 << 64) + 1,
            (1u128 << 127) - 1,
            1u128 << 127,
            u128::MAX - 1,
            u128::MAX,
        ];
        let mut random = 0x67ac_08d2_714f_caa1_4971_2337_fde0_9811u128;
        for _ in 0..32 {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 43;
            values.push(random);
        }
        for &a in &values {
            for &b in &values {
                for ty in [I128, U128] {
                    let signed = ty.is_signed();
                    for op in [
                        Add, Sub, Mul, Div, Rem, Eq, Ne, Lt, Le, Gt, Ge, BitAnd, BitOr, BitXor,
                    ] {
                        let expected = match op {
                            Add if signed => (a as i128).checked_add(b as i128).map(|v| v as u128),
                            Sub if signed => (a as i128).checked_sub(b as i128).map(|v| v as u128),
                            Mul if signed => (a as i128).checked_mul(b as i128).map(|v| v as u128),
                            Div if signed => (a as i128).checked_div(b as i128).map(|v| v as u128),
                            Rem if signed => (a as i128).checked_rem(b as i128).map(|v| v as u128),
                            Add => a.checked_add(b),
                            Sub => a.checked_sub(b),
                            Mul => a.checked_mul(b),
                            Div => a.checked_div(b),
                            Rem => a.checked_rem(b),
                            Eq => Some(u128::from(a == b)),
                            Ne => Some(u128::from(a != b)),
                            Lt => Some(u128::from(if signed {
                                (a as i128) < (b as i128)
                            } else {
                                a < b
                            })),
                            Le => Some(u128::from(if signed {
                                (a as i128) <= (b as i128)
                            } else {
                                a <= b
                            })),
                            Gt => Some(u128::from(if signed {
                                (a as i128) > (b as i128)
                            } else {
                                a > b
                            })),
                            Ge => Some(u128::from(if signed {
                                (a as i128) >= (b as i128)
                            } else {
                                a >= b
                            })),
                            BitAnd => Some(a & b),
                            BitOr => Some(a | b),
                            BitXor => Some(a ^ b),
                            _ => unreachable!(),
                        };
                        if let Some(expected) = expected {
                            cases.push(Case {
                                op,
                                ty,
                                a,
                                b,
                                expected,
                            });
                        }
                    }
                }
            }
            for b in [0, 1, 2, 31, 63, 64, 65, 126, 127] {
                for ty in [I128, U128] {
                    cases.push(Case {
                        op: Shl,
                        ty,
                        a,
                        b,
                        expected: a << b,
                    });
                    cases.push(Case {
                        op: Shr,
                        ty,
                        a,
                        b,
                        expected: if ty.is_signed() {
                            ((a as i128) >> b) as u128
                        } else {
                            a >> b
                        },
                    });
                }
            }
        }
        // Batch to bound generated image and make failures localizable.
        for (batch, chunk) in cases.chunks(300).enumerate() {
            if run(chunk, &[], true) != 0 {
                for case in chunk {
                    assert_eq!(run(&[*case], &[], true), 0, "batch {batch}: {case:?}");
                }
                panic!("batch {batch} failed only when combined");
            }
        }
        let mut unary = Vec::new();
        for &a in &values {
            unary.push((UnaryOp::Not, U128, a, !a));
            unary.push((UnaryOp::Not, I128, a, !a));
            if let Some(v) = (a as i128).checked_neg() {
                unary.push((UnaryOp::Neg, I128, a, v as u128));
            }
        }
        unary.push((UnaryOp::Neg, U128, 0, 0));
        assert_eq!(run(&[], &unary, true), 0);
        let narrow = [
            Case {
                op: Shl,
                ty: U64,
                a: 7,
                b: 1,
                expected: 14,
            },
            Case {
                op: Shr,
                ty: I8,
                a: (-128i128) as u128,
                b: 7,
                expected: u128::MAX,
            },
            Case {
                op: Shl,
                ty: U8,
                a: 255,
                b: 7,
                expected: 128,
            },
        ];
        assert_eq!(run(&narrow, &[], true), 0);
    }

    #[test]
    fn wide_native_oracle_traps() {
        use BinaryOp::*;
        use ir::IntTy::*;
        let mut cases = vec![
            (Add, U128, u128::MAX, 1),
            (Sub, U128, 0, 1),
            (Add, I128, i128::MAX as u128, 1),
            (Sub, I128, i128::MIN as u128, 1),
            (Mul, U128, 1u128 << 64, 1u128 << 64),
            (Mul, U128, u128::MAX, 2),
            (Mul, I128, i128::MIN as u128, u128::MAX),
            (Mul, I128, i128::MAX as u128, 2),
            (Mul, I128, 1u128 << 126, 4),
        ];
        for op in [Div, Rem] {
            cases.push((op, U128, 19, 0));
            cases.push((op, I128, 19, 0));
            cases.push((op, I128, i128::MIN as u128, u128::MAX));
        }
        for op in [Shl, Shr] {
            for b in [128, 129, 1u128 << 64, u128::MAX] {
                cases.push((op, U128, 1, b));
            }
            for (ty, b) in [(U8, 8), (I16, 16), (U32, 32), (U64, 64)] {
                cases.push((op, ty, 1, b));
            }
        }
        for (op, ty, a, b) in cases {
            let case = Case {
                op,
                ty,
                a,
                b,
                expected: 0,
            };
            // No result checks: only the operation's own guards may produce exit 101.
            assert_eq!(run(&[case], &[], false), 101, "{case:?}");
        }
        assert_eq!(
            run(
                &[],
                &[(UnaryOp::Neg, I128, i128::MIN as u128, i128::MIN as u128)],
                false
            ),
            101
        );
    }
}
