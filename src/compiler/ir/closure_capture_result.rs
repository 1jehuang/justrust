//! Exact original expression occurrences and finalized source capture slots.
//! Callee metadata is a substitution recipe, never signature or loan authority.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::compiler) struct CaptureResultWitness {
    pub recipe: ClosureRecipeId,
    pub nodes: Arc<[CaptureResultNode]>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::compiler) struct CaptureResultNode {
    pub expression: ExprId,
    pub kind: CaptureResultKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::compiler) enum CaptureResultKind {
    Call {
        target: Callee,
        arguments: Arc<[ExprId]>,
    },
    ArrayToSlice {
        reference: ExprId,
    },
    Tuple {
        fields: Arc<[ExprId]>,
        source_ty: Ty,
    },
    Borrow {
        field: usize,
        source: CaptureSource,
        original: Place,
        prefix: Arc<[Projection]>,
    },
    Read {
        field: usize,
        source: CaptureSource,
        original: Place,
        prefix: Arc<[Projection]>,
    },
}
