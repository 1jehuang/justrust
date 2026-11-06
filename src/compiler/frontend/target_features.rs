//! Per-function target features for the fixed Linux x86-64 backend.
//! Name membership follows the public Rust target-feature declarations at
//! rust-lang/rust 48a229ceaefd4985c50990b14116b6d856af0985,
//! compiler/rustc_target/src/target_features.rs. Only factual names are used,
//! not its implementation. Nonbaseline features never reach native codegen.
use super::*;
use crate::compiler::token_session::OriginalSpan;

// Complete x86 attribute-name membership at the pinned language version,
// including unstable names. LLVM-only -Ctarget-feature names are not attributes.
const OTHER_STABLE: &str = "adx aes avx avx2 avx512bf16 avx512bitalg avx512bw avx512cd avx512dq avx512f avx512fp16 avx512ifma avx512vbmi avx512vbmi2 avx512vl avx512vnni avx512vp2intersect avx512vpopcntdq avxifma avxneconvert avxvnni avxvnniint8 avxvnniint16 bmi1 bmi2 cmpxchg16b f16c fma gfni kl lzcnt movbe pclmulqdq popcnt rdrand rdseed sha sha512 sm3 sm4 sse3 sse4.1 sse4.2 sse4a ssse3 tbm vaes vpclmulqdq widekl xsave xsavec xsaveopt xsaves";
const UNSTABLE: &str = "amx-avx512 amx-bf16 amx-complex amx-fp8 amx-fp16 amx-int8 amx-movrs amx-tf32 amx-tile apxf avx10.1 avx10.2 clflushopt ermsb fma4 lahfsahf movrs prfchw rtm x87 xop";
const FORBIDDEN: &str =
    "retpoline-external-thunk retpoline-indirect-branches retpoline-indirect-calls soft-float";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Baseline(u8);

fn features(value: &str) -> Result<Baseline, String> {
    let mut result = 0;
    let mut unsupported = false;
    for name in value.split(',') {
        let bit = match name {
            "fxsr" => 1,
            "sse" => 2,
            // SSE2 implies SSE. These are architecture guarantees, not a host
            // CPU probe and not permission to call an unsafe fn from safe code.
            "sse2" => 6,
            name if OTHER_STABLE
                .split_whitespace()
                .chain(UNSTABLE.split_whitespace())
                .any(|known| known == name) =>
            {
                unsupported = true;
                0
            }
            name if FORBIDDEN.split_whitespace().any(|known| known == name) => {
                return Err(format!(
                    "target feature `{name}` cannot be enabled on a function"
                ));
            }
            name => {
                return Err(format!(
                    "the feature named `{name}` is not valid for this target"
                ));
            }
        };
        result |= bit;
    }
    if unsupported {
        return Err("unsupported: nonbaseline target feature code generation".into());
    }
    Ok(Baseline(result))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Selection {
    Baseline(Baseline),
    Invalid(String),
}

fn selection(value: &str) -> Result<Selection, String> {
    match features(value) {
        Ok(features) => Ok(Selection::Baseline(features)),
        Err(message) if message.starts_with("unsupported:") => Err(message),
        Err(message) => Ok(Selection::Invalid(message)),
    }
}

#[derive(Clone, Debug)]
pub(super) struct Pending {
    source: OriginalSpan,
    features: Selection,
}

/// Immutable declaration-owned attribute. An unsafe baseline declaration keeps
/// its ordinary unsafe-call obligation. It adds no ambient unsafe capability.
#[derive(Clone, Debug)]
pub(in crate::compiler) struct Attribute {
    source: OriginalSpan,
    features: Selection,
    owner: OriginalSpan,
    module: ModuleId,
    name: AstIdent,
}

fn source_features(names: &impl NameAccess, source: OriginalSpan) -> Result<Selection, String> {
    let snapshot = names.session().sources();
    let span = snapshot
        .render_span(source)
        .map_err(|error| error.to_string())?;
    let text = snapshot
        .text(source.file())
        .map_err(|error| error.to_string())?;
    let meta = crate::compiler::source::source_cfg::parse_meta(
        &text[span.lo as usize..span.hi as usize],
        0,
    )
    .map_err(|error| error.message)?;
    let crate::compiler::source::source_cfg::MetaValue::List(values) = meta.value else {
        return Err("target_feature requires enable = a string literal".into());
    };
    if meta.name != "target_feature" || values.len() != 1 || values[0].name != "enable" {
        return Err("unsupported: target_feature requires one enable string".into());
    }
    let crate::compiler::source::source_cfg::MetaValue::String(value) = &values[0].value else {
        return Err("target_feature enable must be a string literal".into());
    };
    selection(value)
}

impl Parser<'_, '_> {
    pub(super) fn target_feature_attribute(&mut self) -> Result<(), ParseError> {
        if self.pending_target_feature.is_some() {
            return Err(self.unsupported("repeated target_feature attributes are unsupported"));
        }
        let first = primitive_constants::original_token(self)
            .ok_or_else(|| self.unsupported("target_feature requires original source"))?;
        self.expect("target_feature")?;
        self.expect("(")?;
        self.expect("enable")?;
        self.expect("=")?;
        if !self.token().string {
            return Err(self.error("target_feature enable must be a string literal"));
        }
        let value = selection(self.token().text).map_err(|message| {
            if let Some(message) = message.strip_prefix("unsupported: ") {
                self.unsupported(message)
            } else {
                self.error(&message)
            }
        })?;
        if let Selection::Invalid(message) = &value {
            self.names
                .retain_parser_text(message.len())
                .map_err(native_adapter::name_error)?;
        }
        self.cursor.bump();
        if self.eat(",") && !self.is(")") {
            return Err(self.unsupported("multiple target_feature enable clauses are unsupported"));
        }
        let last = primitive_constants::original_token(self)
            .ok_or_else(|| self.unsupported("target_feature requires original source"))?;
        self.expect(")")?;
        let snapshot = self.names.session().sources();
        let start = snapshot
            .render_span(first)
            .map_err(native_adapter::transport_error)?;
        let end = snapshot
            .render_span(last)
            .map_err(native_adapter::transport_error)?;
        if first.file() != last.file() || start.lo >= end.hi {
            return Err(self.unsupported("target_feature requires one original source range"));
        }
        let source = snapshot
            .span(first.file(), start.lo, end.hi)
            .map_err(native_adapter::transport_error)?;
        if source_features(self.names, source).map_err(|message| self.error(&message))? != value {
            return Err(self.error("target_feature original source differs from parsed features"));
        }
        self.pending_target_feature = Some(Pending {
            source,
            features: value,
        });
        Ok(())
    }
}

impl Pending {
    pub(super) fn bind(
        self,
        parser: &mut Parser<'_, '_>,
        head: &FunctionHead,
        owner: Option<OriginalSpan>,
        associated: bool,
        external: bool,
    ) -> Result<Arc<Attribute>, ParseError> {
        if associated || external || !head.type_params.is_empty() || !head.const_params.is_empty() {
            return Err(parser
                .unsupported("target_feature currently requires a nongeneric free Rust function"));
        }
        if head.constness != Constness::Runtime {
            return Err(
                parser.unsupported("const target_feature function semantics are unsupported")
            );
        }
        if !head.unsafety {
            return Err(
                parser.unsupported("safe target_feature function call contexts are unsupported")
            );
        }
        if parser
            .names
            .spelling(head.name)
            .map_err(native_adapter::name_error)?
            == "main"
        {
            return Err(parser.unsupported("target_feature entry point contexts are unsupported"));
        }
        let owner = owner.ok_or_else(|| {
            parser.unsupported("target_feature function requires original source")
        })?;
        if owner.file() != self.source.file() {
            return Err(parser.error("target_feature and function have different source owners"));
        }
        parser
            .names
            .retain_parser_storage(
                std::mem::size_of::<Attribute>() + 2 * std::mem::size_of::<usize>(),
            )
            .map_err(native_adapter::name_error)?;
        Ok(Arc::new(Attribute {
            source: self.source,
            features: self.features,
            owner,
            module: parser.module,
            name: head.name,
        }))
    }
}

pub(super) fn same(
    names: &NameStore,
    left: Option<&Attribute>,
    right: Option<&Attribute>,
) -> Result<bool, String> {
    match (left, right) {
        (None, None) => Ok(true),
        (Some(left), Some(right)) => Ok(left.source == right.source
            && left.owner == right.owner
            && left.features == right.features
            && left.module == right.module
            && exact_source_ident(names, left.name, right.name)?),
        _ => Ok(false),
    }
}

pub(super) fn validate(program: &Program, function: &Function) -> Result<(), String> {
    let Some(attribute) = &function.target_feature else {
        return Ok(());
    };
    let names = &*program.names;
    if source_features(names, attribute.source)? != attribute.features
        || attribute.module != function.module
        || !function
            .name
            .segments()
            .last()
            .map(|name| exact_source_ident(names, *name, attribute.name))
            .transpose()?
            .unwrap_or(false)
        || !function.unsafety
        || function.constness != Constness::Runtime
        || !function.type_params.is_empty()
        || !function.const_params.is_empty()
    {
        return Err("contract: changed source target_feature declaration".into());
    }
    let snapshot = names.session().sources();
    let owner = snapshot
        .render_span(attribute.owner)
        .map_err(|error| error.to_string())?;
    let text = snapshot
        .text(attribute.owner.file())
        .map_err(|error| error.to_string())?;
    if &text[owner.lo as usize..owner.hi as usize] != "unsafe" {
        return Err("contract: target_feature lost its original unsafe function owner".into());
    }
    if let Selection::Invalid(message) = &attribute.features {
        return Err(message.clone());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_target_features_use_one_pointer_per_function() {
        assert_eq!(
            std::mem::size_of::<Option<Arc<Attribute>>>(),
            std::mem::size_of::<usize>()
        );
    }

    #[test]
    fn catalog_keeps_known_unmodeled_features_distinct_from_invalid_names() {
        assert_eq!(features("sse2").unwrap(), features("sse,sse2,sse").unwrap());
        for name in OTHER_STABLE
            .split_whitespace()
            .chain(UNSTABLE.split_whitespace())
        {
            assert!(
                features(name).unwrap_err().starts_with("unsupported:"),
                "{name}"
            );
        }
        for name in ["not_an_x86_feature", "", "sse2,not_an_x86_feature"] {
            assert!(
                features(name)
                    .unwrap_err()
                    .contains("not valid for this target")
            );
        }
    }

    #[test]
    fn source_feature_and_unsafe_owner_changes_cannot_reuse_declaration_facts() {
        let source =
            "#[target_feature(enable=\"sse2\")] unsafe fn calculate(x:u32)->u32{x+1} fn main(){}";
        for change in 0..3 {
            let mut program = parse(source).unwrap();
            let budget = ir::PlanBudget::default();
            let plan = PlanCtx::from_program(&program, &budget).unwrap();
            let declaration = plan.function_decl(0).unwrap();
            match change {
                0 => {
                    Arc::make_mut(program.functions[0].target_feature.as_mut().unwrap()).features =
                        Selection::Baseline(Baseline(1))
                }
                1 => program.functions[0].unsafety = false,
                _ => program.functions[0].target_feature = None,
            }
            assert!(type_callable_with_plan(&program, declaration, &plan, &budget).is_err());
        }
    }
}
