//! Addresses LLVM is left to compute as integers once it has built one function into another.
//!
//! The IR a function is lowered from has no pointers, only integers, and [`super::lower`]
//! tells which are addresses from what the function does with them. A function that is
//! given an address and only adds to it, or keeps it, has an integer: `NonNull::add`, an
//! iterator's `next`. Built into a caller that has the address as a pointer, that is
//! `inttoptr(ptrtoint p + n)`, or a loop's `phi` of such integers, which LLVM does not take
//! to be based on `p`: what is stored through `p` is loaded again, and a slot whose address
//! it is stays in memory (nbody's `delta`, summed through `iter()`). The lowering already
//! takes a pointer plus any integer to be based on the pointer; this does so for what
//! building in made of it, with `getelementptr i8, p, n` and `phi`s of pointers.

use inkwell::{
    llvm_sys::{
        LLVMOpcode,
        core::{
            LLVMAddIncoming, LLVMBuildGEP2, LLVMBuildPhi, LLVMBuildPtrToInt, LLVMBuildSelect,
            LLVMConstGEP2, LLVMConstNull, LLVMCountIncoming, LLVMCreateBuilderInContext,
            LLVMDisposeBuilder, LLVMGetConstOpcode, LLVMGetFirstBasicBlock, LLVMGetFirstFunction,
            LLVMGetFirstInstruction, LLVMGetIncomingBlock, LLVMGetIncomingValue,
            LLVMGetInstructionOpcode, LLVMGetInstructionParent, LLVMGetIsDisjoint,
            LLVMGetModuleContext, LLVMGetNextBasicBlock, LLVMGetNextFunction,
            LLVMGetNextInstruction, LLVMGetOperand, LLVMInstructionEraseFromParent,
            LLVMInt8TypeInContext, LLVMIsAConstantExpr, LLVMIsAConstantInt, LLVMIsAInstruction,
            LLVMIsNull, LLVMPositionBuilderBefore, LLVMReplaceAllUsesWith, LLVMTypeOf,
        },
        prelude::{LLVMBasicBlockRef, LLVMBuilderRef, LLVMTypeRef, LLVMValueRef},
    },
    module::Module,
};
use rustc_hash::{FxHashMap, FxHashSet};

/// The most integers one address is made of that are rewritten as pointers.
const LARGEST: usize = 64;

/// Rewrites each `inttoptr` in `module` of an integer made of `ptrtoint`s, null and what is
/// added to them, through `phi`s and `select`s, as pointers. Says whether it rewrote any.
pub(super) fn rebase(module: &Module<'_>) -> bool {
    let mut rewrote = false;
    // SAFETY: every value walked is of `module`, which outlives the walk; an instruction is
    // erased only after the walk has moved past it and its uses have been replaced.
    unsafe {
        let context = LLVMGetModuleContext(module.as_mut_ptr());
        let builder = LLVMCreateBuilderInContext(context);
        let byte = LLVMInt8TypeInContext(context);
        let mut function = LLVMGetFirstFunction(module.as_mut_ptr());
        while !function.is_null() {
            let mut block = LLVMGetFirstBasicBlock(function);
            while !block.is_null() {
                let mut inst = LLVMGetFirstInstruction(block);
                while !inst.is_null() {
                    let next = LLVMGetNextInstruction(inst);
                    if LLVMGetInstructionOpcode(inst) == LLVMOpcode::LLVMIntToPtr
                        && Web::rewrite(builder, byte, inst)
                    {
                        rewrote = true;
                    }
                    inst = next;
                }
                block = LLVMGetNextBasicBlock(block);
            }
            function = LLVMGetNextFunction(function);
        }
        LLVMDisposeBuilder(builder);
    }
    rewrote
}

/// What an integer of the web is as a pointer.
#[derive(Clone, Copy)]
enum Node {
    /// `ptrtoint` of this pointer.
    Address(LLVMValueRef),
    /// Zero, which is the null pointer.
    Null,
    Phi,
    /// `select` of a condition between two of the web.
    Select,
    /// The integer is the one of the web among the operands plus the other.
    Sum {
        based: LLVMValueRef,
        offset: LLVMValueRef,
    },
}

/// The integers an address is made of, each with what it is made of in turn.
struct Web {
    nodes: FxHashMap<LLVMValueRef, Node>,
    /// In an order that has an operand before what is made of it, `phi`s aside.
    order: Vec<LLVMValueRef>,
    addresses: usize,
    /// The type of every integer of the web: the one `inttoptr` takes.
    int: LLVMTypeRef,
}

impl Web {
    /// Rewrites `inttoptr` as the pointer its integer is, if every integer it is made of
    /// is one of [`Node`]'s with at least one `ptrtoint`.
    unsafe fn rewrite(builder: LLVMBuilderRef, byte: LLVMTypeRef, inttoptr: LLVMValueRef) -> bool {
        unsafe {
            let pointer = LLVMTypeOf(inttoptr);
            let root = LLVMGetOperand(inttoptr, 0);
            let int = LLVMTypeOf(root);
            let mut web = Web { nodes: FxHashMap::default(), order: Vec::new(), addresses: 0, int };
            if web.add(root, pointer).is_none() || web.addresses == 0 {
                return false;
            }
            // A `ptrtoint` alone is LLVM's to fold.
            if matches!(web.nodes[&root], Node::Address(_)) || !web.ordered() {
                return false;
            }
            web.build(builder, byte, pointer, root, inttoptr);
            true
        }
    }

    /// Takes `value` and what it is made of into the web, if each is of it. `None` if one
    /// is not, or the web would be larger than [`LARGEST`].
    unsafe fn add(&mut self, value: LLVMValueRef, pointer: LLVMTypeRef) -> Option<()> {
        unsafe {
            if self.nodes.contains_key(&value) {
                return Some(());
            }
            if self.nodes.len() >= LARGEST || LLVMTypeOf(value) != self.int {
                return None;
            }
            let node = Self::node(value, pointer)?;
            self.nodes.insert(value, node);
            match node {
                Node::Address(_) => self.addresses += 1,
                Node::Null => {}
                Node::Phi => {
                    for at in 0..LLVMCountIncoming(value) {
                        self.add(LLVMGetIncomingValue(value, at), pointer)?;
                    }
                }
                Node::Select => {
                    self.add(LLVMGetOperand(value, 1), pointer)?;
                    self.add(LLVMGetOperand(value, 2), pointer)?;
                }
                Node::Sum { based, .. } => self.add(based, pointer)?,
            }
            if !matches!(node, Node::Phi) {
                self.order.push(value);
            }
            Some(())
        }
    }

    /// Whether each integer of [`Web::order`] comes after what it is made of. Not where an
    /// instruction is made of itself without a `phi`, which only unreachable code may be.
    unsafe fn ordered(&self) -> bool {
        unsafe {
            let mut built: FxHashSet<LLVMValueRef> = self
                .nodes
                .iter()
                .filter(|(_, node)| matches!(node, Node::Phi))
                .map(|(&value, _)| value)
                .collect();
            for &value in &self.order {
                let ready = match self.nodes[&value] {
                    Node::Address(_) | Node::Null | Node::Phi => true,
                    Node::Select => [LLVMGetOperand(value, 1), LLVMGetOperand(value, 2)]
                        .iter()
                        .all(|it| built.contains(it)),
                    Node::Sum { based, .. } => built.contains(&based),
                };
                if !ready {
                    return false;
                }
                built.insert(value);
            }
            true
        }
    }

    /// What `value` is in the web, if it may be of it.
    unsafe fn node(value: LLVMValueRef, pointer: LLVMTypeRef) -> Option<Node> {
        unsafe {
            if !LLVMIsAConstantInt(value).is_null() {
                return (LLVMIsNull(value) != 0).then_some(Node::Null);
            }
            let opcode = if !LLVMIsAInstruction(value).is_null() {
                LLVMGetInstructionOpcode(value)
            } else if !LLVMIsAConstantExpr(value).is_null() {
                LLVMGetConstOpcode(value)
            } else {
                return None;
            };
            Some(match opcode {
                LLVMOpcode::LLVMPtrToInt => {
                    let address = LLVMGetOperand(value, 0);
                    if LLVMTypeOf(address) != pointer {
                        return None;
                    }
                    Node::Address(address)
                }
                LLVMOpcode::LLVMPHI => Node::Phi,
                LLVMOpcode::LLVMSelect if !LLVMIsAInstruction(value).is_null() => Node::Select,
                LLVMOpcode::LLVMAdd => Self::sum(value)?,
                LLVMOpcode::LLVMOr
                    if !LLVMIsAInstruction(value).is_null() && LLVMGetIsDisjoint(value) != 0 =>
                {
                    Self::sum(value)?
                }
                _ => return None,
            })
        }
    }

    /// Which operand of the sum `value` is the address and which is added to it: the one
    /// that may be an address, if the other may not.
    unsafe fn sum(value: LLVMValueRef) -> Option<Node> {
        unsafe {
            let (a, b) = (LLVMGetOperand(value, 0), LLVMGetOperand(value, 1));
            match (Self::may_be_address(a), Self::may_be_address(b)) {
                (true, false) => Some(Node::Sum { based: a, offset: b }),
                (false, true) => Some(Node::Sum { based: b, offset: a }),
                _ => None,
            }
        }
    }

    /// Whether `value` is of a kind an address of the web is: a `ptrtoint`, a `phi` or a
    /// `select`, or a sum with one of those in it.
    unsafe fn may_be_address(value: LLVMValueRef) -> bool {
        unsafe {
            let opcode = if !LLVMIsAInstruction(value).is_null() {
                LLVMGetInstructionOpcode(value)
            } else if !LLVMIsAConstantExpr(value).is_null() {
                LLVMGetConstOpcode(value)
            } else {
                return false;
            };
            match opcode {
                LLVMOpcode::LLVMPtrToInt | LLVMOpcode::LLVMPHI | LLVMOpcode::LLVMSelect => true,
                LLVMOpcode::LLVMAdd | LLVMOpcode::LLVMOr => {
                    let (a, b) = (LLVMGetOperand(value, 0), LLVMGetOperand(value, 1));
                    [a, b].iter().any(|&it| {
                        let inner = if !LLVMIsAInstruction(it).is_null() {
                            LLVMGetInstructionOpcode(it)
                        } else if !LLVMIsAConstantExpr(it).is_null() {
                            LLVMGetConstOpcode(it)
                        } else {
                            return false;
                        };
                        matches!(
                            inner,
                            LLVMOpcode::LLVMPtrToInt | LLVMOpcode::LLVMPHI | LLVMOpcode::LLVMSelect
                        )
                    })
                }
                _ => false,
            }
        }
    }

    /// Builds the pointer of each integer of the web where the integer is, has every other
    /// use of an integer take `ptrtoint` of its pointer, and `inttoptr` the pointer of `root`.
    unsafe fn build(
        &self,
        builder: LLVMBuilderRef,
        byte: LLVMTypeRef,
        pointer: LLVMTypeRef,
        root: LLVMValueRef,
        inttoptr: LLVMValueRef,
    ) {
        unsafe {
            let mut pointers: FxHashMap<LLVMValueRef, LLVMValueRef> = FxHashMap::default();
            let phis: Vec<LLVMValueRef> = self
                .nodes
                .iter()
                .filter(|(_, node)| matches!(node, Node::Phi))
                .map(|(&value, _)| value)
                .collect();
            for &phi in &phis {
                LLVMPositionBuilderBefore(builder, phi);
                pointers.insert(phi, LLVMBuildPhi(builder, pointer, c"".as_ptr()));
            }
            for &value in &self.order {
                let built = match self.nodes[&value] {
                    Node::Address(address) => address,
                    Node::Null => LLVMConstNull(pointer),
                    Node::Phi => unreachable!("phis are built first"),
                    Node::Select => {
                        LLVMPositionBuilderBefore(builder, value);
                        let (then, otherwise) =
                            (LLVMGetOperand(value, 1), LLVMGetOperand(value, 2));
                        LLVMBuildSelect(
                            builder,
                            LLVMGetOperand(value, 0),
                            pointers[&then],
                            pointers[&otherwise],
                            c"".as_ptr(),
                        )
                    }
                    // A constant's sum is of constants, and is in no block to build before.
                    Node::Sum { based, mut offset } if LLVMIsAInstruction(value).is_null() => {
                        LLVMConstGEP2(byte, pointers[&based], &mut offset, 1)
                    }
                    Node::Sum { based, mut offset } => {
                        LLVMPositionBuilderBefore(builder, value);
                        LLVMBuildGEP2(builder, byte, pointers[&based], &mut offset, 1, c"".as_ptr())
                    }
                };
                pointers.insert(value, built);
            }
            for &phi in &phis {
                for at in 0..LLVMCountIncoming(phi) {
                    let mut incoming = pointers[&LLVMGetIncomingValue(phi, at)];
                    let mut from: LLVMBasicBlockRef = LLVMGetIncomingBlock(phi, at);
                    LLVMAddIncoming(pointers[&phi], &mut incoming, &mut from, 1);
                }
            }
            LLVMReplaceAllUsesWith(inttoptr, pointers[&root]);
            LLVMInstructionEraseFromParent(inttoptr);
            // What still takes an integer of the web takes the pointer's: the integers then
            // have no use left but each other, and go.
            for (&value, node) in &self.nodes {
                if LLVMIsAInstruction(value).is_null() || matches!(node, Node::Address(_)) {
                    continue;
                }
                let after = if matches!(node, Node::Phi) {
                    first_after_phis(LLVMGetInstructionParent(value))
                } else {
                    LLVMGetNextInstruction(value)
                };
                LLVMPositionBuilderBefore(builder, after);
                let int =
                    LLVMBuildPtrToInt(builder, pointers[&value], LLVMTypeOf(value), c"".as_ptr());
                LLVMReplaceAllUsesWith(value, int);
            }
        }
    }
}

/// The first instruction of `block` that is not a `phi` nor the landing pad.
unsafe fn first_after_phis(block: LLVMBasicBlockRef) -> LLVMValueRef {
    unsafe {
        let mut inst = LLVMGetFirstInstruction(block);
        while matches!(
            LLVMGetInstructionOpcode(inst),
            LLVMOpcode::LLVMPHI | LLVMOpcode::LLVMLandingPad
        ) {
            inst = LLVMGetNextInstruction(inst);
        }
        inst
    }
}
