//! Immutable static storage, with object-local relocations into its payload.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StaticBlob {
    pub key: Vec<u8>,
    pub bytes: Vec<u8>,
    pub align: usize,
    /// Each pointer word at `from` points to `target` in this same blob.
    pub relocations: Vec<(usize, usize)>,
}

impl StaticBlob {
    pub(in crate::compiler) fn validate_relocations(&self) -> Result<(), String> {
        for (from, target) in &self.relocations {
            if from.checked_add(8).is_none_or(|end| end > self.bytes.len())
                || *target >= self.bytes.len()
            {
                return Err("static data relocation exceeds its allocation".into());
            }
            if self.bytes[*from..*from + 8] != [0; 8] {
                return Err("static data relocation slot is not zero".into());
            }
        }
        Ok(())
    }
    pub(super) fn relocate(&self, base: u64, image: &mut [u8]) -> Result<(), String> {
        self.validate_relocations()?;
        if image.len() != self.bytes.len() {
            return Err("static data image extent changed".into());
        }
        for (from, target) in &self.relocations {
            let address = base
                .checked_add(u64::try_from(*target).map_err(|_| "static target offset overflow")?)
                .ok_or("static data address overflow")?;
            image[*from..*from + 8].copy_from_slice(&address.to_le_bytes());
        }
        Ok(())
    }
}

impl Emitter<'_> {
    pub(super) fn static_ref(&mut self, data: &ir::StaticData) -> Result<(), String> {
        let budget = self.concrete.facts().budget().clone();
        data.validate(&budget)?;
        let mut image = StaticImage {
            bytes: Vec::new(),
            relocations: Vec::new(),
            align: 16,
            targets: HashMap::new(),
            budget: &budget,
        };
        let root = image.allocate(data.value_ty())?;
        if root != 0 {
            return Err("static root storage moved".into());
        }
        let mut next = 0;
        image.write(data.value_ty(), data.values(), &mut next, 0, false, 0)?;
        if next != data.values().len() {
            return Err("static data payload differs from its declared type".into());
        }
        let key = data.link_key();
        budget.retain_bytes(
            std::mem::size_of::<(usize, StaticBlob)>()
                .checked_add(key.len())
                .ok_or("static relocation storage overflow")?,
        )?;
        self.object
            .statics
            .try_reserve_exact(1)
            .map_err(|_| "static relocation allocation failed")?;
        self.bytes(&[0x48, 0xbe]);
        let at = self.position();
        self.bytes(&[0; 8]);
        self.object.statics.push((
            at,
            StaticBlob {
                key,
                bytes: image.bytes,
                align: image.align,
                relocations: image.relocations,
            },
        ));
        self.bytes(&[0x48, 0x89, 0xf0]);
        Ok(())
    }
}

/// Literal array promotions share the static field/array layout writer.
/// Their admitted leaves contain no pointers and therefore no relocations.
pub(super) fn promoted_scalar_image(
    ty: &ir::Ty,
    values: &[ir::PromotedScalar],
    budget: &ir::PlanBudget,
) -> Result<Vec<u8>, String> {
    budget.retain_bytes(
        values
            .len()
            .checked_mul(std::mem::size_of::<ir::StaticValue>())
            .ok_or("promoted scalar storage overflow")?,
    )?;
    let mut leaves = Vec::new();
    leaves
        .try_reserve_exact(values.len())
        .map_err(|_| "promoted scalar allocation failed")?;
    for value in values {
        leaves.push(ir::StaticValue::Scalar(*value));
    }
    let mut image = StaticImage {
        bytes: Vec::new(),
        relocations: Vec::new(),
        align: 16,
        targets: HashMap::new(),
        budget,
    };
    image.allocate(ty)?;
    let mut next = 0;
    image.write(ty, &leaves, &mut next, 0, false, 0)?;
    if next != leaves.len() || !image.relocations.is_empty() {
        return Err("promoted array payload differs from its type".into());
    }
    Ok(image.bytes)
}

struct StaticImage<'a> {
    bytes: Vec<u8>,
    relocations: Vec<(usize, usize)>,
    align: usize,
    targets: HashMap<(ir::Ty, ir::StaticValue), usize>,
    budget: &'a ir::PlanBudget,
}
impl StaticImage<'_> {
    fn reserve(&mut self, size: usize, align: usize) -> Result<usize, String> {
        if !align.is_power_of_two() || align > MAX_REPR_ALIGN as usize {
            return Err("invalid static target alignment".into());
        }
        self.align = self.align.max(align);
        let at = self
            .bytes
            .len()
            .checked_add(align - 1)
            .map(|n| n & !(align - 1))
            .ok_or("static target offset overflow")?;
        let end = at
            .checked_add(size.max(1))
            .ok_or("static target extent overflow")?;
        let added = end - self.bytes.len();
        self.budget.charge_work(added)?;
        self.budget.retain_bytes(added)?;
        self.bytes
            .try_reserve_exact(added)
            .map_err(|_| "static target allocation failed")?;
        self.bytes.resize(end, 0);
        Ok(at)
    }
    fn allocate(&mut self, ty: &ir::Ty) -> Result<usize, String> {
        let align = usize::try_from(frame_value_alignment(ty, self.budget)?)
            .map_err(|_| "negative static alignment")?
            .max(16);
        let size = usize::try_from(value_size(ty, self.budget)?)
            .map_err(|_| "negative static extent")?
            .max(align);
        self.reserve(size, align)
    }
    fn pointer(&mut self, at: usize, target: usize, len: usize) -> Result<(), String> {
        let end = at
            .checked_add(16)
            .ok_or("static descriptor offset overflow")?;
        let descriptor = self
            .bytes
            .get_mut(at..end)
            .ok_or("static descriptor exceeds storage")?;
        descriptor[..8].fill(0);
        descriptor[8..].copy_from_slice(
            &u64::try_from(len)
                .map_err(|_| "static descriptor length overflow")?
                .to_le_bytes(),
        );
        self.budget
            .retain_bytes(std::mem::size_of::<(usize, usize)>())?;
        self.relocations
            .try_reserve_exact(1)
            .map_err(|_| "static data relocation allocation failed")?;
        self.relocations.push((at, target));
        Ok(())
    }
    fn write(
        &mut self,
        ty: &ir::Ty,
        values: &[ir::StaticValue],
        next: &mut usize,
        at: usize,
        exact: bool,
        depth: usize,
    ) -> Result<(), String> {
        self.budget.check_depth(depth)?;
        self.budget.charge_work(1)?;
        match ty {
            ir::Ty::Unit => Ok(()),
            ir::Ty::Int(_) | ir::Ty::Bool | ir::Ty::Char => {
                let Some(ir::StaticValue::Scalar(scalar)) = values.get(*next) else {
                    return Err("static scalar payload is absent".into());
                };
                *next += 1;
                scalar.validate()?;
                if scalar.value_ty() != *ty {
                    return Err("static data scalar differs from its declared type".into());
                }
                let (word, high): (u64, u64) = match *scalar {
                    ir::PromotedScalar::Int(value) if value.ty.bits() == 128 => {
                        (value.bits as u64, (value.bits >> 64) as u64)
                    }
                    ir::PromotedScalar::Int(value) => {
                        let bits = value.ty.bits();
                        let raw = value.bits as u64;
                        let word = if bits == 64 {
                            raw
                        } else if value.ty.is_signed() {
                            (((raw << (64 - bits)) as i64) >> (64 - bits)) as u64
                        } else {
                            raw & ((1u64 << bits) - 1)
                        };
                        (word, 0)
                    }
                    ir::PromotedScalar::Bool(value) => (u64::from(value), 0),
                    ir::PromotedScalar::Char(value) => (u64::from(value.value), 0),
                };
                let width = if wide_integer(ty) {
                    16
                } else if exact {
                    scalar_memory_bytes(ty) as usize
                } else {
                    8
                };
                let mut image = [0u8; 16];
                image[..8].copy_from_slice(&word.to_le_bytes());
                image[8..].copy_from_slice(&high.to_le_bytes());
                let end = at.checked_add(width).ok_or("static offset overflow")?;
                self.bytes
                    .get_mut(at..end)
                    .ok_or("static leaf exceeds storage")?
                    .copy_from_slice(&image[..width]);
                Ok(())
            }
            ir::Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent,
            } => {
                let value = values
                    .get(*next)
                    .ok_or("static reference payload is absent")?;
                *next += 1;
                charge_target(value, self.budget, depth + 1)?;
                let key = (ty.clone(), value.clone());
                let len = match value {
                    ir::StaticValue::Text(text) => text.len(),
                    ir::StaticValue::Slice { len, .. } => *len,
                    _ => return Err("static reference payload is not a reference".into()),
                };
                if let Some(target) = self.targets.get(&key).copied() {
                    return self.pointer(at, target, len);
                }
                self.budget
                    .retain_bytes(std::mem::size_of::<((ir::Ty, ir::StaticValue), usize)>() * 2)?;
                self.targets
                    .try_reserve(1)
                    .map_err(|_| "static target interning allocation failed")?;
                match (referent.as_ref(), value) {
                    (ir::Ty::Str, ir::StaticValue::Text(text)) => {
                        let target = self.reserve(text.len(), 1)?;
                        self.bytes[target..target + text.len()].copy_from_slice(text.as_bytes());
                        self.targets.insert(key, target);
                        self.pointer(at, target, text.len())
                    }
                    (ir::Ty::Slice { element }, ir::StaticValue::Slice { len, values }) => {
                        let array = ir::Ty::Array {
                            element: element.clone(),
                            len: *len,
                        };
                        let target = self.allocate(&array)?;
                        let mut inner = 0;
                        self.write(&array, values, &mut inner, target, true, depth + 1)?;
                        if inner != values.len() {
                            return Err(
                                "static slice payload differs from its declared type".into()
                            );
                        }
                        self.targets.insert(key, target);
                        self.pointer(at, target, *len)
                    }
                    _ => Err("static reference payload differs from its declared type".into()),
                }
            }
            ir::Ty::Array { len, .. } => {
                for index in 0..*len {
                    let (offset, element) = array_offset(ty, index, self.budget)?;
                    let offset = usize::try_from(offset).map_err(|_| "negative array offset")?;
                    self.write(
                        &element,
                        values,
                        next,
                        at.checked_add(offset).ok_or("static offset overflow")?,
                        true,
                        depth + 1,
                    )?;
                }
                Ok(())
            }
            ir::Ty::Tuple(_) | ir::Ty::Struct(_) => {
                let compact =
                    crate::compiler::memory_layout::compact::layout(ty, self.budget, 0)?.is_some();
                let count = ty
                    .field_count()
                    .ok_or("static aggregate has no canonical fields")?;
                for index in 0..count {
                    let (offset, field) = field_offset(ty, index, self.budget)?;
                    let offset = usize::try_from(offset).map_err(|_| "negative field offset")?;
                    self.write(
                        &field,
                        values,
                        next,
                        at.checked_add(offset).ok_or("static offset overflow")?,
                        compact,
                        depth + 1,
                    )?;
                }
                Ok(())
            }
            _ => Err("unsupported: static data has an unsupported leaf".into()),
        }
    }
}

/// Pay for the structural hash/equality walk before target interning. Only
/// immutable promoted targets are merged, never distinct static declarations.
fn charge_target(
    value: &ir::StaticValue,
    budget: &ir::PlanBudget,
    depth: usize,
) -> Result<(), String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    match value {
        ir::StaticValue::Scalar(_) => Ok(()),
        ir::StaticValue::Text(text) => budget.charge_work(text.len()),
        ir::StaticValue::Slice { values, .. } => {
            for value in values.iter() {
                charge_target(value, budget, depth + 1)?;
            }
            Ok(())
        }
    }
}
