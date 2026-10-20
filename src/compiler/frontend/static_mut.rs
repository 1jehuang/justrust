//! `static mut NAME: T = INIT;` items.
//!
//! Each item becomes a synthetic accessor `fn NAME() -> *mut T` whose body is
//! `std::mem::static_slot(ID, INIT)`. The prelude slot owns a fixed,
//! item-specific address window that is mapped and initialized on first use
//! and shared by every later access, so all accesses observe one mutable
//! location for the whole process, as a Rust static does. Name resolution
//! rewrites every use of NAME to `*NAME()`, which is a raw-pointer place and
//! therefore requires an unsafe context (E0133), as in stock Rust.
use super::*;

impl Parser<'_, '_> {
    pub(super) fn static_mut_item(&mut self, module: ModuleId) -> Result<Function, ParseError> {
        if self.item_expansion.is_some() {
            return Err(self.unsupported("generated static mut items are unsupported"));
        }
        self.expect("static")?;
        self.expect("mut")?;
        let name = self.ident()?;
        self.expect(":")?;
        let ty = self.ty()?;
        self.expect("=")?;
        let initializer = self.nested_expr()?;
        self.expect(";")?;
        let origin = self.names.span(name).map_err(native_adapter::name_error)?;
        // Stable per-item identity: FNV-1a of the module chain and name.
        let mut hash: u64 = 0xcbf29ce484222325;
        let mut feed = |text: &str| {
            for b in text.bytes().chain(std::iter::once(b':')) {
                hash ^= u64::from(b);
                hash = hash.wrapping_mul(0x100000001b3);
            }
        };
        let mut chain = Vec::new();
        let mut cursor = Some(module);
        while let Some(m) = cursor {
            if let Some(ident) = self.modules.name(m).map_err(module_parse_error)? {
                chain.push(
                    self.names
                        .spelling(ident)
                        .map_err(native_adapter::name_error)?
                        .to_owned(),
                );
            }
            cursor = self.modules.parent(m).map_err(module_parse_error)?;
        }
        for part in chain.iter().rev() {
            feed(part);
        }
        feed(
            self.names
                .spelling(name)
                .map_err(native_adapter::name_error)?,
        );
        let id = u128::from(hash & 0xf_ffff);
        let mut intern = |text: &str| {
            self.names
                .intern_root(text, origin)
                .map_err(native_adapter::name_error)
        };
        let segments = [intern("std")?, intern("mem")?, intern("static_slot")?];
        let callee = AstPath::new(self.names, PathAnchor::Relative, &segments)
            .map_err(native_adapter::name_error)?;
        let body = Block {
            statements: Vec::new(),
            tail: Some(Box::new(Expr::Call {
                name: callee,
                args: vec![
                    Expr::Integer(IntegerLiteral {
                        magnitude: id,
                        suffix: Some(ir::IntTy::Usize),
                    }),
                    initializer,
                ],
            })),
        };
        Ok(Function {
            embedded_source: None,
            lexical_scope: None,
            module,
            constness: Constness::Runtime,
            unsafety: false,
            static_mut: true,
            visibility: ir::Visibility::Public,
            bounds: Vec::new(),
            name: AstPath::new(self.names, PathAnchor::Relative, &[name])
                .map_err(native_adapter::name_error)?,
            regions: RegionDecls::default(),
            type_params: Vec::new(),
            params: Vec::new(),
            param_mut: Vec::new(),
            result: Type::RawPtr {
                kind: ir::BorrowKind::Mut,
                pointee: Box::new(ty),
            },
            impl_params: 0,
            body,
        })
    }
}
