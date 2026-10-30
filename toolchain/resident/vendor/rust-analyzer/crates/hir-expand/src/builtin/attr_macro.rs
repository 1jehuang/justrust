//! Builtin attributes.
use base_db::SourceDatabase;
use intern::sym;
use span::Span;

use crate::{ExpandResult, MacroCallId, MacroCallKind, name, tt};

use super::quote;

macro_rules! register_builtin {
    ($(($name:ident, $variant:ident) => $expand:ident),* $(,)? ) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum BuiltinAttrExpander {
            $($variant),*
        }

        impl BuiltinAttrExpander {
            /// Every variant, in declaration order.
            pub const ALL: &[Self] = &[$(Self::$variant),*];

            pub fn expander(&self) -> fn (&dyn SourceDatabase, MacroCallId, &tt::TopSubtree, Span) -> ExpandResult<tt::TopSubtree>  {
                match *self {
                    $( BuiltinAttrExpander::$variant => $expand, )*
                }
            }

            fn find_by_name(name: &name::Name) -> Option<Self> {
                match name {
                    $( id if id == &sym::$name => Some(BuiltinAttrExpander::$variant), )*
                     _ => None,
                }
            }
        }

    };
}

impl BuiltinAttrExpander {
    pub fn expand(
        &self,
        db: &dyn SourceDatabase,
        id: MacroCallId,
        tt: &tt::TopSubtree,
        span: Span,
    ) -> ExpandResult<tt::TopSubtree> {
        self.expander()(db, id, tt, span)
    }

    pub fn is_derive(self) -> bool {
        matches!(self, BuiltinAttrExpander::Derive | BuiltinAttrExpander::DeriveConst)
    }
    pub fn is_test(self) -> bool {
        matches!(self, BuiltinAttrExpander::Test)
    }
    pub fn is_bench(self) -> bool {
        matches!(self, BuiltinAttrExpander::Bench)
    }
    pub fn is_test_case(self) -> bool {
        matches!(self, BuiltinAttrExpander::TestCase)
    }
}

register_builtin! {
    (bench, Bench) => dummy_gate_test_expand,
    (cfg_accessible, CfgAccessible) => dummy_attr_expand,
    (cfg_eval, CfgEval) => dummy_attr_expand,
    (derive, Derive) => derive_expand,
    // derive const is equivalent to derive for our proposes.
    (derive_const, DeriveConst) => derive_expand,
    (global_allocator, GlobalAllocator) => global_allocator_expand,
    (test, Test) => dummy_gate_test_expand,
    (test_case, TestCase) => dummy_gate_test_expand,
    (define_opaque, DefineOpaque) => define_opaque_expand,
}

pub fn find_builtin_attr(ident: &name::Name) -> Option<BuiltinAttrExpander> {
    BuiltinAttrExpander::find_by_name(ident)
}

fn dummy_attr_expand(
    _db: &dyn SourceDatabase,
    _id: MacroCallId,
    tt: &tt::TopSubtree,
    _span: Span,
) -> ExpandResult<tt::TopSubtree> {
    ExpandResult::ok(tt.clone())
}

/// The static, and what calls it in the allocator's place: rustc's expansion defines `__rg_alloc`
/// and the others, which its allocator shim calls. These have names of ra-jit's, which the
/// stand-in rustc builds for a crate whose code is ra-jit's calls from a `#[global_allocator]`
/// of its own (ra-jit's `stub.rs`).
fn global_allocator_expand(
    _db: &dyn SourceDatabase,
    _id: MacroCallId,
    tt: &tt::TopSubtree,
    span: Span,
) -> ExpandResult<tt::TopSubtree> {
    let ident = |it: tt::TtElement<'_>| match it {
        tt::TtElement::Leaf(tt::Leaf::Ident(ident)) => Some(ident.clone()),
        _ => None,
    };
    let mut idents = tt.iter().filter_map(ident);
    idents.find(|it| it.sym.as_str() == "static");
    let Some(name) = idents.find(|it| it.sym.as_str() != "mut") else {
        return ExpandResult::ok(tt.clone());
    };
    let name = &name;
    let result = quote::quote! { span=>
        #tt
        #[export_name = "__ra_jit_global_alloc"]
        unsafe extern "C" fn __ra_jit_global_alloc(size: usize, align: usize) -> *mut u8 {
            unsafe {
                ::core::alloc::GlobalAlloc::alloc(
                    &#name,
                    ::core::alloc::Layout::from_size_align_unchecked(size, align),
                )
            }
        }
        #[export_name = "__ra_jit_global_dealloc"]
        unsafe extern "C" fn __ra_jit_global_dealloc(ptr: *mut u8, size: usize, align: usize) {
            unsafe {
                ::core::alloc::GlobalAlloc::dealloc(
                    &#name,
                    ptr,
                    ::core::alloc::Layout::from_size_align_unchecked(size, align),
                )
            }
        }
        #[export_name = "__ra_jit_global_realloc"]
        unsafe extern "C" fn __ra_jit_global_realloc(
            ptr: *mut u8,
            size: usize,
            align: usize,
            new_size: usize,
        ) -> *mut u8 {
            unsafe {
                ::core::alloc::GlobalAlloc::realloc(
                    &#name,
                    ptr,
                    ::core::alloc::Layout::from_size_align_unchecked(size, align),
                    new_size,
                )
            }
        }
        #[export_name = "__ra_jit_global_alloc_zeroed"]
        unsafe extern "C" fn __ra_jit_global_alloc_zeroed(size: usize, align: usize) -> *mut u8 {
            unsafe {
                ::core::alloc::GlobalAlloc::alloc_zeroed(
                    &#name,
                    ::core::alloc::Layout::from_size_align_unchecked(size, align),
                )
            }
        }
    };
    ExpandResult::ok(result)
}

/// The item keeps the list as a tool attribute, which type inference reads and name resolution
/// does not expand again.
fn define_opaque_expand(
    db: &dyn SourceDatabase,
    id: MacroCallId,
    tt: &tt::TopSubtree,
    span: Span,
) -> ExpandResult<tt::TopSubtree> {
    let MacroCallKind::Attr { attr_args: Some(args), .. } = &id.loc(db).kind else {
        return ExpandResult::ok(tt.clone());
    };
    let args = &**args;
    let result = quote::quote! { span=>
        #[rust_analyzer::define_opaque(#args)]
        #tt
    };
    ExpandResult::ok(result)
}

fn dummy_gate_test_expand(
    _db: &dyn SourceDatabase,
    _id: MacroCallId,
    tt: &tt::TopSubtree,
    span: Span,
) -> ExpandResult<tt::TopSubtree> {
    let result = quote::quote! { span=>
        #[cfg(test)]
        #tt
    };
    ExpandResult::ok(result)
}

/// We generate a very specific expansion here, as we do not actually expand the `#[derive]` attribute
/// itself in name res, but we do want to expand it to something for the IDE layer, so that the input
/// derive attributes can be downmapped, and resolved as proper paths.
/// This is basically a hack, that simplifies the hacks we need in a lot of ide layer places to
/// somewhat inconsistently resolve derive attributes.
///
/// As such, we expand `#[derive(Foo, bar::Bar)]` into
/// ```ignore
///  #![Foo]
///  #![bar::Bar]
/// ```
/// which allows fallback path resolution in hir::Semantics to properly identify our derives.
/// Since we do not expand the attribute in nameres though, we keep the original item.
///
/// The ideal expansion here would be for the `#[derive]` to re-emit the annotated item and somehow
/// use the input paths in its output as well.
/// But that would bring two problems with it, for one every derive would duplicate the item token tree
/// wasting a lot of memory, and it would also require some way to use a path in a way that makes it
/// always resolve as a derive without nameres recollecting them.
/// So this hacky approach is a lot more friendly for us, though it does require a bit of support in
/// hir::Semantics to make this work.
fn derive_expand(
    db: &dyn SourceDatabase,
    id: MacroCallId,
    tt: &tt::TopSubtree,
    span: Span,
) -> ExpandResult<tt::TopSubtree> {
    let loc = id.loc(db);
    let derives = match &loc.kind {
        MacroCallKind::Attr { attr_args: Some(attr_args), .. } if loc.def.is_attribute_derive() => {
            attr_args
        }
        _ => {
            return ExpandResult::ok(tt::TopSubtree::empty(tt::DelimSpan {
                open: span,
                close: span,
            }));
        }
    };
    pseudo_derive_attr_expansion(tt, derives, span)
}

pub fn pseudo_derive_attr_expansion(
    _: &tt::TopSubtree,
    args: &tt::TopSubtree,
    call_site: Span,
) -> ExpandResult<tt::TopSubtree> {
    let mk_leaf =
        |char| tt::Leaf::Punct(tt::Punct { char, spacing: tt::Spacing::Alone, span: call_site });

    let mut token_trees = tt::TopSubtreeBuilder::new(args.top_subtree().delimiter);
    let iter = args.token_trees().split(|tt| {
        matches!(tt, tt::TtElement::Leaf(tt::Leaf::Punct(tt::Punct { char: ',', .. })))
    });
    for tts in iter {
        token_trees.extend([mk_leaf('#'), mk_leaf('!')]);
        token_trees.open(tt::DelimiterKind::Bracket, call_site);
        token_trees.extend_with_tt(tts);
        token_trees.close(call_site);
    }
    ExpandResult::ok(token_trees.build())
}
