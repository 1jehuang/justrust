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
    member: AstIdent,
    visibility: ir::Visibility,
    initializer: Arc<ScalarSource>,
    _admitted: crate::compiler::expansion_budget::RetainedBytes,
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
    if block.trait_path.is_some() || !matches!(block.self_ty, Type::Int(_)) {
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
    let member = single_constant(block)?;
    let initializer = member
        .initializer
        .as_ref()
        .ok_or("reserved primitive source initializer is missing")?;
    let nodes = ScalarSource::nodes(initializer, 0)?;
    use crate::compiler::expansion_budget::{Counter, Storage};
    budget
        .charge(Counter::Work, nodes as u64)
        .map_err(|error| error.to_string())?;
    let bytes = std::mem::size_of::<SourceImpl>() + nodes * std::mem::size_of::<ScalarSource>();
    let admitted = budget
        .reserve(Storage::Retained, bytes as u64)
        .and_then(|lease| lease.retain())
        .map_err(|error| error.to_string())?;
    Ok(Some(Arc::new(SourceImpl {
        implementation,
        source,
        issuer: issuer.clone(),
        module: block.module,
        member: member.name,
        visibility: member.visibility.clone(),
        initializer: Arc::new(ScalarSource::capture(initializer, 0)?),
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
        let member = single_constant(block)?;
        if !exact_source_ident(names, member.name, self.member)?
            || member.visibility != self.visibility
            || ScalarSource::capture(
                member
                    .initializer
                    .as_ref()
                    .ok_or("reserved primitive initializer missing")?,
                0,
            )? != *self.initializer
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
            || actual != &ir::Ty::Int(ir::IntTy::I32)
        {
            return Err(
                "reserved primitive impl changed original source or canonical owner".into(),
            );
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
}
