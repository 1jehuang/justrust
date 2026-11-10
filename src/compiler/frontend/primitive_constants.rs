//! Reserved primitive impl admission from loader-issued original source only.
use super::*;
use crate::compiler::token_session::{OriginalSpan, SpanId};
#[cfg(test)]
#[path = "primitive_constants_tests.rs"]
mod tests;

pub(super) fn original_token(parser: &Parser<'_, '_>) -> Option<OriginalSpan> {
    let token = parser.token();
    if token.provenance != TokenProvenance::Prepared {
        return None;
    }
    match parser.names.session().span(token.span).ok()?.origin {
        crate::compiler::expansion_span::SpanOrigin::Original(span) => Some(span),
        _ => None,
    }
}
pub(super) fn original_impl_range(
    parser: &Parser<'_, '_>,
    start: Option<OriginalSpan>,
) -> Option<OriginalSpan> {
    let snapshot = parser.names.session().sources();
    let handle = start?.file();
    let start = snapshot.render_span(start?).ok()?;
    let end = snapshot.render_span(original_token(parser)?).ok()?;
    if start.file != end.file || start.lo > end.hi {
        return None;
    }
    snapshot.span(handle, start.lo, end.hi).ok()
}

/// Finite original initializer syntax, not a computed constant or magic value.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ScalarSource {
    Integer(u128, Option<ir::IntTy>),
    Neg(Option<SpanId>, Box<ScalarSource>),
    Sub(Option<SpanId>, Box<ScalarSource>, Box<ScalarSource>),
}
impl ScalarSource {
    fn nodes(value: &Expr, depth: usize) -> Result<usize, String> {
        if depth > 8 {
            return Err("unsupported: reserved primitive initializer syntax depth".into());
        }
        match value {
            Expr::Integer(_) => Ok(1),
            Expr::Unary {
                op: UnaryOp::Neg,
                value,
                ..
            } => Ok(1 + Self::nodes(value, depth + 1)?),
            Expr::Binary {
                op: BinaryOp::Sub,
                left,
                right,
                ..
            } => Ok(1 + Self::nodes(left, depth + 1)? + Self::nodes(right, depth + 1)?),
            _ => Err("unsupported: reserved primitive initializer syntax".into()),
        }
    }
    fn capture(value: &Expr, depth: usize) -> Result<Self, String> {
        if depth > 8 {
            return Err("unsupported: reserved primitive initializer syntax depth".into());
        }
        Ok(match value {
            Expr::Integer(value) => Self::Integer(value.magnitude, value.suffix),
            Expr::Unary {
                op: UnaryOp::Neg,
                value,
                site,
            } => Self::Neg(*site, Box::new(Self::capture(value, depth + 1)?)),
            Expr::Binary {
                op: BinaryOp::Sub,
                left,
                right,
                site,
            } => Self::Sub(
                *site,
                Box::new(Self::capture(left, depth + 1)?),
                Box::new(Self::capture(right, depth + 1)?),
            ),
            _ => return Err("unsupported: reserved primitive initializer syntax".into()),
        })
    }
}

#[derive(Clone, Debug)]
pub(super) struct SourceImpl {
    implementation: usize,
    source: OriginalSpan,
    issuer: crate::compiler::source::EmbeddedOriginalFile,
    module: ModuleId,
    content: Content,
    _admitted: crate::compiler::expansion_budget::RetainedBytes,
}
#[derive(Clone, Debug)]
enum Content {
    Constant {
        member: AstIdent,
        visibility: ir::Visibility,
        initializer: Arc<ScalarSource>,
    },
    Methods(Arc<primitive_methods::Methods>),
}
fn single_constant(block: &ImplDecl) -> Result<&AssociatedConstDecl, String> {
    if block.trait_path.is_some()
        || !block.type_params.is_empty()
        || !block.bounds.is_empty()
        || !block.items.is_empty()
        || !block.associated_types.is_empty()
        || block.associated_constants.len() != 1
        || !matches!(block.self_ty, Type::Int(ir::IntTy::I32))
    {
        return Err("unsupported: reserved primitive source impl shape".into());
    }
    let member = &block.associated_constants[0];
    if !matches!(member.ty, Type::Int(ir::IntTy::I32)) {
        return Err("reserved primitive constant source changed declared width".into());
    }
    Ok(member)
}
/// Called only by the existing parser impl arm, before that declaration is
/// published. No extra inventory pass over the program is introduced.
pub(super) fn capture(
    block: &ImplDecl,
    implementation: usize,
    issuers: &[crate::compiler::source::EmbeddedOriginalFile],
    snapshot: &crate::compiler::token_session::SourceSnapshot,
    budget: &crate::compiler::expansion_budget::ExpansionBudget,
) -> Result<Option<Arc<SourceImpl>>, String> {
    if block.trait_path.is_some() || !matches!(block.self_ty, Type::Int(_) | Type::Bool) {
        return Ok(None);
    }
    let Some(source) = block.original_source else {
        return Ok(None);
    };
    let Some(issuer) = issuers
        .iter()
        .find(|issuer| issuer.authorizes(snapshot, source))
    else {
        return Ok(None);
    };
    let (methods, nodes) = if block.items.is_empty() {
        let member = single_constant(block)?;
        let initializer = member
            .initializer
            .as_ref()
            .ok_or("reserved primitive source initializer is missing")?;
        (None, ScalarSource::nodes(initializer, 0)?)
    } else {
        (Some(primitive_methods::Methods::capture(block, budget)?), 0)
    };
    use crate::compiler::expansion_budget::{Counter, Storage};
    budget
        .charge(Counter::Work, nodes as u64)
        .map_err(|error| error.to_string())?;
    let bytes = std::mem::size_of::<SourceImpl>() + nodes * std::mem::size_of::<ScalarSource>();
    let admitted = budget
        .reserve(Storage::Retained, bytes as u64)
        .and_then(|lease| lease.retain())
        .map_err(|error| error.to_string())?;
    let content = if let Some(methods) = methods {
        Content::Methods(Arc::new(methods))
    } else {
        let member = single_constant(block)?;
        Content::Constant {
            member: member.name,
            visibility: member.visibility.clone(),
            initializer: Arc::new(ScalarSource::capture(
                member
                    .initializer
                    .as_ref()
                    .ok_or("reserved primitive initializer missing")?,
                0,
            )?),
        }
    };
    Ok(Some(Arc::new(SourceImpl {
        implementation,
        source,
        issuer: issuer.clone(),
        module: block.module,
        content,
        _admitted: admitted,
    })))
}
impl SourceImpl {
    pub(super) fn relocate_parsed(
        &self,
        block: &ImplDecl,
        index: usize,
        offset: usize,
        program: &Program,
        budget: &crate::compiler::expansion_budget::ExpansionBudget,
    ) -> Result<Arc<Self>, String> {
        self.replay_source(block, index, &program.names, &program.embedded_issuers)?;
        let index = index
            .checked_add(offset)
            .ok_or("primitive implementation ordinal overflow")?;
        let admitted = budget
            .reserve(
                crate::compiler::expansion_budget::Storage::Retained,
                (std::mem::size_of::<Self>() + 2 * std::mem::size_of::<usize>()) as u64,
            )
            .and_then(|lease| lease.retain())
            .map_err(|error| error.to_string())?;
        let mut relocated = self.clone();
        relocated.implementation = index;
        relocated._admitted = admitted;
        Ok(Arc::new(relocated))
    }

    pub(super) fn replay_source(
        &self,
        block: &ImplDecl,
        index: usize,
        names: &NameStore,
        issuers: &[crate::compiler::source::EmbeddedOriginalFile],
    ) -> Result<(), String> {
        if self.implementation != index
            || block.module != self.module
            || block.original_source != Some(self.source)
            || !self
                .issuer
                .authorizes(names.session().sources(), self.source)
            || !issuers
                .iter()
                .any(|issuer| issuer.same_issuer(&self.issuer))
        {
            return Err("reserved primitive impl changed original source occurrence".into());
        }
        let Content::Constant {
            member: original_member,
            visibility,
            initializer,
        } = &self.content
        else {
            let Content::Methods(methods) = &self.content else {
                unreachable!()
            };
            return methods.replay(block, names);
        };
        let member = single_constant(block)?;
        if !exact_source_ident(names, member.name, *original_member)?
            || member.visibility != *visibility
            || ScalarSource::capture(
                member
                    .initializer
                    .as_ref()
                    .ok_or("reserved primitive initializer missing")?,
                0,
            )? != **initializer
        {
            return Err(
                "reserved primitive constant changed original member occurrence or initializer"
                    .into(),
            );
        }
        Ok(())
    }
    /// Consumed inside the existing canonical impl-registration operation.
    pub(super) fn replay_registration(
        &self,
        program: &Program,
        registry: &DeclRegistry,
        index: usize,
        declaration: ir::DeclId,
        actual: &ir::Ty,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        let block = program
            .impls
            .get(index)
            .ok_or("reserved primitive source impl missing")?;
        let info = registry.declaration(declaration)?;
        if self.implementation != index
            || info.kind != ir::DeclKind::Impl
            || info.source_index != index
            || registry.impl_ids.get(index) != Some(&declaration)
        {
            return Err(
                "reserved primitive impl changed original source or canonical owner".into(),
            );
        }
        if let Content::Methods(methods) = &self.content {
            let owner_matches = match (&block.self_ty, actual) {
                (Type::Bool, ir::Ty::Bool) => true,
                (Type::Int(a), ir::Ty::Int(b)) => a == b,
                _ => false,
            };
            if !owner_matches {
                return Err("primitive method changed canonical owner".into());
            }
            methods.require_resolved()?;
            methods.charge_registration(budget)?;
            budget.charge_work(block.items.len())?;
            return self.replay_source(block, index, &program.names, &program.embedded_issuers);
        }
        if actual != &ir::Ty::Int(ir::IntTy::I32) {
            return Err("primitive constant changed canonical owner".into());
        }
        let member = single_constant(block)?;
        let nodes = ScalarSource::nodes(
            member
                .initializer
                .as_ref()
                .ok_or("reserved primitive initializer missing")?,
            0,
        )?;
        budget.charge_work(nodes)?;
        budget.retain_bytes(nodes * std::mem::size_of::<ScalarSource>())?;
        self.replay_source(block, index, &program.names, &program.embedded_issuers)
    }

    pub(super) fn has_methods(&self) -> bool {
        matches!(self.content, Content::Methods(_))
    }

    pub(super) fn require_original_methods(&self) -> Result<(), String> {
        if let Content::Methods(methods) = &self.content {
            methods.require_original()?;
        }
        Ok(())
    }

    pub(super) fn resolved_methods(
        self: &Arc<Self>,
        resolution: root_resolution::PrimitiveResolution,
        budget: &crate::compiler::expansion_budget::ExpansionBudget,
    ) -> Result<Arc<Self>, String> {
        let resolved = resolution.consume(self)?;
        if self.has_methods() != resolved.is_some() {
            return Err("primitive resolution changed source impl kind".into());
        }
        let Some(resolved) = resolved else {
            return Ok(self.clone());
        };
        let admitted = budget
            .reserve(
                crate::compiler::expansion_budget::Storage::Retained,
                std::mem::size_of::<Self>() as u64,
            )
            .and_then(|lease| lease.retain())
            .map_err(|e| e.to_string())?;
        let mut source = (**self).clone();
        source.content = Content::Methods(Arc::new(resolved.into_resolved()?));
        source._admitted = admitted;
        Ok(Arc::new(source))
    }
}
