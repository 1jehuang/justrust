//! Immutable static item storage: one interned data blob per declaration.
use super::*;

/// The canonical declaration key and its exact immutable bytes. Linkers
/// intern by `key` and reject two objects that disagree on the bytes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StaticBlob {
    pub key: Vec<u8>,
    pub bytes: Vec<u8>,
}

impl Emitter<'_> {
    pub(super) fn static_ref(&mut self, data: &ir::StaticData) -> Result<(), String> {
        let budget = self.concrete.facts().budget().clone();
        data.validate(&budget)?;
        let bytes = match data.value_ty() {
            ir::Ty::Array { .. } => self.promoted_array_bytes(data.value_ty(), data.values())?,
            _ => match data.values() {
                [scalar] => self.promoted_scalar_bytes(*scalar)?,
                _ => return Err("static scalar payload changed".into()),
            },
        };
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
        self.bytes(&[0x48, 0xbe]); // mov rsi, imm64
        let at = self.position();
        self.bytes(&[0; 8]);
        self.object.statics.push((at, StaticBlob { key, bytes }));
        self.bytes(&[0x48, 0x89, 0xf0]); // mov rax, rsi: thin static pointer
        Ok(())
    }
}
