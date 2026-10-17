//! Module-domain expansion authority, separate from function lexical frontiers.
use super::*;
use crate::compiler::module_identity::{ModuleArena, ModuleArenaBuilder, ModuleError, ModuleId};

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::ModuleArena {}
    impl Sealed for super::ModuleArenaBuilder {}
}
pub(crate) trait ModuleAccess: sealed::Sealed {
    fn check_names(&self, names: &impl NameAccess) -> Result<(), ModuleError>;
    fn check(&self, module: ModuleId) -> Result<(), ModuleError>;
    fn source_root(&self) -> Result<ModuleId, ModuleError>;
    fn parent(&self, module: ModuleId) -> Result<Option<ModuleId>, ModuleError>;
    fn root_of(&self, module: ModuleId) -> Result<ModuleId, ModuleError>;
    fn budget(&self) -> &ExpansionBudget;
}
macro_rules! module_access {
    ($ty:ty) => {
        impl ModuleAccess for $ty {
            fn check_names(&self, names: &impl NameAccess) -> Result<(), ModuleError> {
                <$ty>::check_names(self, names)
            }
            fn check(&self, module: ModuleId) -> Result<(), ModuleError> {
                <$ty>::check(self, module)
            }
            fn source_root(&self) -> Result<ModuleId, ModuleError> {
                <$ty>::source_root(self)
            }
            fn parent(&self, module: ModuleId) -> Result<Option<ModuleId>, ModuleError> {
                <$ty>::parent(self, module)
            }
            fn root_of(&self, module: ModuleId) -> Result<ModuleId, ModuleError> {
                <$ty>::root_of(self, module)
            }
            fn budget(&self) -> &ExpansionBudget {
                <$ty>::budget(self)
            }
        }
    };
}
module_access!(ModuleArena);
module_access!(ModuleArenaBuilder);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ModuleFrontierId {
    owner: NonZeroU64,
    index: u32,
}
#[derive(Clone, Copy, Debug)]
struct ModulePoint {
    module: ModuleId,
    previous: Option<ModuleFrontierId>,
    entry_parent: Option<ModuleFrontierId>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ItemExpansionAdmission {
    owner: NonZeroU64,
    session: SessionId,
    expansion: ExpansionId,
    definition: ModuleFrontierId,
    call: ModuleFrontierId,
    definition_module: ModuleId,
    target_module: ModuleId,
    source_root: ModuleId,
}
impl ItemExpansionAdmission {
    pub(crate) fn validate_session(&self, session: &TokenSession) -> Result<(), ScopeError> {
        session.budget().check()?;
        if session.id() != self.session {
            return Err(ScopeError::ForeignNames);
        }
        session.expansion(self.expansion)?;
        Ok(())
    }
    pub(crate) fn check_names(&self, names: &impl NameAccess) -> Result<(), ScopeError> {
        self.validate_session(names.session())
    }
    pub(crate) fn check_modules(
        &self,
        names: &impl NameAccess,
        modules: &impl ModuleAccess,
    ) -> Result<(), ScopeError> {
        self.check_names(names)?;
        check_modules(names, modules, self.source_root)?;
        for module in [self.definition_module, self.target_module] {
            if modules.root_of(module)? != self.source_root {
                return Err(ModuleError::InvalidParent.into());
            }
        }
        Ok(())
    }
    pub(crate) fn expansion(&self, names: &impl NameAccess) -> Result<ExpansionId, ScopeError> {
        self.check_names(names)?;
        Ok(self.expansion)
    }
    pub(crate) fn validate_span(
        &self,
        session: &TokenSession,
        span: SpanId,
    ) -> Result<(), ScopeError> {
        self.validate_session(session)?;
        session.budget().charge(Counter::Work, 1)?;
        let data = session.span(span)?;
        if data.expansion != Some(self.expansion) {
            return Err(
                TokenError::InvalidInput("item occurrence belongs to another expansion").into(),
            );
        }
        match *session.context(data.context)? {
            ContextData::Root => Ok(()),
            ContextData::Mark {
                parent,
                expansion,
                transparency: Transparency::SemiTransparent,
            } if expansion == self.expansion
                && matches!(session.context(parent)?, ContextData::Root) =>
            {
                Ok(())
            }
            _ => Err(ScopeError::UnsupportedProjection),
        }
    }
    pub(crate) fn validate_token(
        &self,
        session: &TokenSession,
        token: Token,
    ) -> Result<(), ScopeError> {
        token.validate(session)?;
        self.validate_span(session, token.span)
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum ExpansionTokenAdmission {
    Expression(ExpansionAdmission),
    Item(ItemExpansionAdmission),
}
impl From<ExpansionAdmission> for ExpansionTokenAdmission {
    fn from(p: ExpansionAdmission) -> Self {
        Self::Expression(p)
    }
}
impl From<ItemExpansionAdmission> for ExpansionTokenAdmission {
    fn from(p: ItemExpansionAdmission) -> Self {
        Self::Item(p)
    }
}
impl ExpansionTokenAdmission {
    pub(crate) fn validate_session(&self, s: &TokenSession) -> Result<(), ScopeError> {
        match self {
            Self::Expression(p) => p.validate_session(s),
            Self::Item(p) => p.validate_session(s),
        }
    }
    pub(crate) fn expansion(&self, n: &impl NameAccess) -> Result<ExpansionId, ScopeError> {
        match self {
            Self::Expression(p) => p.expansion(n),
            Self::Item(p) => p.expansion(n),
        }
    }
    pub(crate) fn validate_span(&self, s: &TokenSession, span: SpanId) -> Result<(), ScopeError> {
        match self {
            Self::Expression(p) => p.validate_span(s, span),
            Self::Item(p) => p.validate_span(s, span),
        }
    }
    pub(crate) fn validate_token(&self, s: &TokenSession, t: Token) -> Result<(), ScopeError> {
        match self {
            Self::Expression(p) => p.validate_token(s, t),
            Self::Item(p) => p.validate_token(s, t),
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum OccurrenceRole {
    OriginalModule(ModuleId),
    GeneratedDeclaration {
        module: ModuleId,
        function: ScopeId,
        declaration: AstIdent,
    },
    Function {
        module: ModuleId,
        function: ScopeId,
    },
}
#[derive(Clone, Copy, Debug)]
struct ItemProduct {
    function: ScopeId,
    proof: ItemExpansionAdmission,
    declaration: AstIdent,
    occurrence: u32,
}

pub(super) struct ItemScopeState {
    source_root: Option<ModuleId>,
    points: BudgetVec<ModulePoint>,
    anchors: BudgetVec<ItemExpansionAdmission>,
    products: BudgetVec<ItemProduct>,
    seals: BudgetVec<ScopeId>,
}
impl ItemScopeState {
    pub(super) fn new(b: &ExpansionBudget) -> Result<Self, ScopeError> {
        Ok(Self {
            source_root: None,
            points: BudgetVec::new(b, Storage::Retained)?,
            anchors: BudgetVec::new(b, Storage::Retained)?,
            products: BudgetVec::new(b, Storage::Retained)?,
            seals: BudgetVec::new(b, Storage::Retained)?,
        })
    }
}
fn check_modules(
    names: &impl NameAccess,
    modules: &impl ModuleAccess,
    root: ModuleId,
) -> Result<(), ScopeError> {
    names.budget().check()?;
    modules.check_names(names)?;
    modules.check(root)?;
    if !names.budget().same_authority(modules.budget()) {
        return Err(ModuleError::ForeignNames.into());
    }
    if modules.source_root()? != root {
        return Err(ModuleError::ForeignOwner.into());
    }
    Ok(())
}
impl ScopeArenaBuilder {
    pub(crate) fn module_root(
        &mut self,
        names: &impl NameAccess,
        modules: &impl ModuleAccess,
        module: ModuleId,
        parent: Option<ModuleFrontierId>,
    ) -> Result<ModuleFrontierId, ScopeError> {
        self.check_names(names)?;
        self.arena.transition()?;
        let root = modules.source_root()?;
        check_modules(names, modules, root)?;
        if modules.root_of(module)? != root {
            return Err(ModuleError::InvalidParent.into());
        }
        if let Some(expected) = self.arena.items.source_root
            && expected != root
        {
            return Err(ModuleError::ForeignOwner.into());
        }
        match parent {
            None if module == root => {}
            Some(point)
                if modules.parent(module)? == Some(self.arena.module_point(point)?.module) => {}
            _ => return Err(ModuleError::InvalidParent.into()),
        }
        self.arena.items.source_root = Some(root);
        self.arena.push_module_point(ModulePoint {
            module,
            previous: None,
            entry_parent: parent,
        })
    }
    pub(crate) fn module_step(
        &mut self,
        names: &impl NameAccess,
        modules: &impl ModuleAccess,
        before: ModuleFrontierId,
    ) -> Result<ModuleFrontierId, ScopeError> {
        self.check_names(names)?;
        self.arena.transition()?;
        let root = self
            .arena
            .items
            .source_root
            .ok_or(ScopeError::InvalidHandle)?;
        check_modules(names, modules, root)?;
        let p = *self.arena.module_point(before)?;
        self.arena.push_module_point(ModulePoint {
            module: p.module,
            previous: Some(before),
            entry_parent: p.entry_parent,
        })
    }
    pub(crate) fn module_visible_from(
        &self,
        definition: ModuleFrontierId,
        call: ModuleFrontierId,
    ) -> Result<bool, ScopeError> {
        self.arena.module_visible_from(definition, call)
    }
    pub(crate) fn module_of(&self, p: ModuleFrontierId) -> Result<ModuleId, ScopeError> {
        self.arena.module_of(p)
    }
    pub(crate) fn anchor_item_expansion(
        &mut self,
        names: &impl NameAccess,
        modules: &impl ModuleAccess,
        expansion: ExpansionId,
        definition: ModuleFrontierId,
        call: ModuleFrontierId,
    ) -> Result<ItemExpansionAdmission, ScopeError> {
        self.check_names(names)?;
        self.arena.transition()?;
        let root = self
            .arena
            .items
            .source_root
            .ok_or(ScopeError::InvalidHandle)?;
        check_modules(names, modules, root)?;
        let definition_module = self.arena.module_of(definition)?;
        let target_module = self.arena.module_of(call)?;
        if !self.arena.module_visible_from(definition, call)? {
            return Err(ScopeError::ConflictingAssociation);
        }
        let record = names.session().expansion(expansion)?;
        if record.parent.is_some() {
            return Err(ScopeError::UnsupportedProjection);
        }
        for span in [record.definition, record.call] {
            let data = names.session().span(span)?;
            if data.expansion.is_some()
                || !matches!(names.session().context(data.context)?, ContextData::Root)
            {
                return Err(ScopeError::UnsupportedProjection);
            }
        }
        let proof = ItemExpansionAdmission {
            owner: self.arena.owner,
            session: self.arena.names,
            expansion,
            definition,
            call,
            definition_module,
            target_module,
            source_root: root,
        };
        proof.check_modules(names, modules)?;
        if let Some(previous) = self.arena.item_anchor(expansion)? {
            return if previous == proof {
                Ok(previous)
            } else {
                Err(ScopeError::ConflictingAssociation)
            };
        }
        self.arena.items.anchors.push(proof)?;
        Ok(proof)
    }
    pub(crate) fn generated_function_root(
        &mut self,
        names: &impl NameAccess,
        modules: &impl ModuleAccess,
        proof: &ItemExpansionAdmission,
        declaration: AstIdent,
    ) -> Result<FrontierId, ScopeError> {
        self.check_names(names)?;
        proof.check_modules(names, modules)?;
        self.arena.check_item_proof(proof)?;
        proof.validate_span(names.session(), names.span(declaration)?)?;
        let occurrence = semantic_name::occurrence_index(names, declaration)?;
        for p in self.arena.items.products.as_slice() {
            self.arena.budget.charge(Counter::Work, 1)?;
            if p.proof.expansion == proof.expansion {
                return Err(ScopeError::ConflictingAssociation);
            }
        }
        let frontier = self.function_root(names)?;
        let function = self.function(frontier)?;
        self.arena.items.products.push(ItemProduct {
            function,
            proof: *proof,
            declaration,
            occurrence,
        })?;
        Ok(frontier)
    }
    pub(crate) fn seal_item_product(
        &mut self,
        names: &impl NameAccess,
        modules: &impl ModuleAccess,
        proof: &ItemExpansionAdmission,
        function: ScopeId,
        declaration: AstIdent,
    ) -> Result<(), ScopeError> {
        self.check_names(names)?;
        proof.check_modules(names, modules)?;
        self.arena.check_item_proof(proof)?;
        let p = self
            .arena
            .item_product(function)?
            .ok_or(ScopeError::InvalidHandle)?;
        if p.proof != *proof || p.occurrence != semantic_name::occurrence_index(names, declaration)?
        {
            return Err(ScopeError::ConflictingAssociation);
        }
        if self.arena.item_is_sealed(function)? {
            return Ok(());
        }
        self.arena.items.seals.push(function)?;
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn check_generated_function(
        &self,
        n: &impl NameAccess,
        m: &impl ModuleAccess,
        f: ScopeId,
        d: AstIdent,
        module: ModuleId,
    ) -> Result<(), ScopeError> {
        self.arena.check_generated_function(n, m, f, d, module)
    }
    #[cfg(test)]
    pub(crate) fn check_item_function_coverage(
        &self,
        visited: &[ScopeId],
    ) -> Result<(), ScopeError> {
        self.arena.check_item_function_coverage(visited)
    }
    #[cfg(test)]
    pub(crate) fn check_item_authority(
        &self,
        n: &impl NameAccess,
        m: &impl ModuleAccess,
    ) -> Result<(), ScopeError> {
        self.arena.check_item_authority(n, m)
    }
    #[cfg(test)]
    pub(crate) fn check_occurrence_in(
        &self,
        n: &impl NameAccess,
        m: &impl ModuleAccess,
        i: AstIdent,
        r: OccurrenceRole,
    ) -> Result<(), ScopeError> {
        self.arena.check_occurrence_in(n, m, i, r)
    }
}
impl ScopeArena {
    pub(crate) fn generated_function_declaration(
        &self,
        function: ScopeId,
    ) -> Result<Option<AstIdent>, ScopeError> {
        Ok(self.item_product(function)?.map(|p| p.declaration))
    }
    pub(crate) fn check_item_function_coverage(
        &self,
        visited: &[ScopeId],
    ) -> Result<(), ScopeError> {
        self.check_item_completion()?;
        if self.items.products.len() == 0 && visited.is_empty() {
            return Ok(());
        }
        let count = self.scopes.len();
        self.budget.charge(Counter::Work, count as u64)?;
        let mut lease = self.budget.reserve(Storage::Temporary, count as u64)?;
        let mut seen = Vec::<u8>::new();
        seen.try_reserve_exact(count)
            .map_err(|_| self.budget.poison_allocation())?;
        lease.resize(seen.capacity() as u64)?;
        seen.resize(count, 0);
        for p in self.items.products.as_slice() {
            self.budget.charge(Counter::Work, 1)?;
            self.check_function(p.function)?;
            let slot = &mut seen[p.function.index as usize];
            if *slot != 0 {
                return Err(ScopeError::ConflictingAssociation);
            }
            *slot = 1;
        }
        for function in visited {
            self.budget.charge(Counter::Work, 1)?;
            self.check_function(*function)?;
            let slot = &mut seen[function.index as usize];
            if *slot != 1 {
                return Err(ScopeError::ConflictingAssociation);
            }
            *slot = 2;
        }
        for p in self.items.products.as_slice() {
            self.budget.charge(Counter::Work, 1)?;
            if seen[p.function.index as usize] != 2 {
                return Err(ScopeError::ConflictingAssociation);
            }
        }
        Ok(())
    }
    fn push_module_point(&mut self, p: ModulePoint) -> Result<ModuleFrontierId, ScopeError> {
        let id = ModuleFrontierId {
            owner: self.owner,
            index: self.index(self.items.points.len())?,
        };
        self.items.points.push(p)?;
        Ok(id)
    }
    fn module_point(&self, p: ModuleFrontierId) -> Result<&ModulePoint, ScopeError> {
        self.budget.charge(Counter::Work, 1)?;
        if p.owner != self.owner {
            return Err(ScopeError::ForeignOwner);
        }
        self.items
            .points
            .as_slice()
            .get(p.index as usize)
            .ok_or(ScopeError::InvalidHandle)
    }
    pub(crate) fn module_of(&self, p: ModuleFrontierId) -> Result<ModuleId, ScopeError> {
        Ok(self.module_point(p)?.module)
    }
    pub(crate) fn module_visible_from(
        &self,
        definition: ModuleFrontierId,
        call: ModuleFrontierId,
    ) -> Result<bool, ScopeError> {
        self.module_point(definition)?;
        let mut at = Some(call);
        while let Some(p) = at {
            let node = self.module_point(p)?;
            if p == definition {
                return Ok(true);
            }
            at = node.previous.or(node.entry_parent)
        }
        Ok(false)
    }
    pub(super) fn item_anchor(
        &self,
        e: ExpansionId,
    ) -> Result<Option<ItemExpansionAdmission>, ScopeError> {
        self.budget.check()?;
        for p in self.items.anchors.as_slice() {
            self.budget.charge(Counter::Work, 1)?;
            if p.expansion == e {
                return Ok(Some(*p));
            }
        }
        Ok(None)
    }
    fn check_item_proof(&self, p: &ItemExpansionAdmission) -> Result<(), ScopeError> {
        self.budget.check()?;
        if p.owner != self.owner || p.session != self.names {
            return Err(ScopeError::ForeignOwner);
        }
        if self.item_anchor(p.expansion)? != Some(*p) {
            return Err(ScopeError::UnregisteredExpansion);
        }
        Ok(())
    }
    fn item_product(&self, f: ScopeId) -> Result<Option<ItemProduct>, ScopeError> {
        self.check_function(f)?;
        for p in self.items.products.as_slice() {
            self.budget.charge(Counter::Work, 1)?;
            if p.function == f {
                return Ok(Some(*p));
            }
        }
        Ok(None)
    }
    fn item_is_sealed(&self, f: ScopeId) -> Result<bool, ScopeError> {
        self.budget.check()?;
        for p in self.items.seals.as_slice() {
            self.budget.charge(Counter::Work, 1)?;
            if *p == f {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(super) fn check_item_completion(&self) -> Result<(), ScopeError> {
        self.budget.check()?;
        for p in self.items.anchors.as_slice() {
            self.budget.charge(Counter::Work, 1)?;
            let mut count = 0usize;
            for product in self.items.products.as_slice() {
                self.budget.charge(Counter::Work, 1)?;
                if product.proof.expansion == p.expansion {
                    if !self.item_is_sealed(product.function)? {
                        return Err(ScopeError::ConflictingAssociation);
                    }
                    count += 1;
                }
            }
            if count != 1 {
                return Err(ScopeError::ConflictingAssociation);
            }
        }
        Ok(())
    }
    pub(crate) fn check_generated_function(
        &self,
        names: &impl NameAccess,
        modules: &impl ModuleAccess,
        function: ScopeId,
        declaration: AstIdent,
        module: ModuleId,
    ) -> Result<(), ScopeError> {
        self.check_names(names)?;
        let root = modules.source_root()?;
        check_modules(names, modules, root)?;
        if modules.root_of(module)? != root {
            return Err(ModuleError::InvalidParent.into());
        }
        let Some(p) = self.item_product(function)? else {
            let span = names.session().span(names.span(declaration)?)?;
            return if span.expansion.is_none()
                && matches!(names.session().context(span.context)?, ContextData::Root)
            {
                Ok(())
            } else {
                Err(ScopeError::FunctionMismatch)
            };
        };
        p.proof.check_modules(names, modules)?;
        self.check_item_proof(&p.proof)?;
        if p.proof.target_module != module
            || p.occurrence != semantic_name::occurrence_index(names, declaration)?
            || !self.item_is_sealed(function)?
        {
            return Err(ScopeError::ConflictingAssociation);
        }
        p.proof
            .validate_span(names.session(), names.span(declaration)?)?;
        Ok(())
    }
    pub(crate) fn check_item_authority(
        &self,
        names: &impl NameAccess,
        modules: &impl ModuleAccess,
    ) -> Result<(), ScopeError> {
        self.check_names(names)?;
        let root = modules.source_root()?;
        check_modules(names, modules, root)?;
        if let Some(expected) = self.items.source_root
            && expected != root
        {
            return Err(ModuleError::ForeignOwner.into());
        }
        for p in self.items.points.as_slice() {
            self.budget.charge(Counter::Work, 1)?;
            if modules.root_of(p.module)? != root {
                return Err(ModuleError::InvalidParent.into());
            }
            if let Some(previous) = p.previous
                && self.module_of(previous)? != p.module
            {
                return Err(ScopeError::ConflictingAssociation);
            }
            if let Some(parent) = p.entry_parent {
                if modules.parent(p.module)? != Some(self.module_of(parent)?) {
                    return Err(ModuleError::InvalidParent.into());
                }
            } else if p.module != root {
                return Err(ModuleError::InvalidParent.into());
            }
        }
        for p in self.items.anchors.as_slice() {
            self.budget.charge(Counter::Work, 1)?;
            p.check_modules(names, modules)?;
            if self.module_of(p.definition)? != p.definition_module
                || self.module_of(p.call)? != p.target_module
                || !self.module_visible_from(p.definition, p.call)?
            {
                return Err(ScopeError::ConflictingAssociation);
            }
        }
        self.check_item_completion()?;
        for p in self.items.products.as_slice() {
            self.budget.charge(Counter::Work, 1)?;
            self.check_generated_function(
                names,
                modules,
                p.function,
                p.declaration,
                p.proof.target_module,
            )?;
        }
        Ok(())
    }
    pub(super) fn check_item_body_occurrence(
        &self,
        names: &impl NameAccess,
        ident: AstIdent,
        function: ScopeId,
    ) -> Result<(), ScopeError> {
        self.check_names(names)?;
        let p = self
            .item_product(function)?
            .ok_or(ScopeError::FunctionMismatch)?;
        self.check_item_proof(&p.proof)?;
        p.proof.validate_span(names.session(), names.span(ident)?)?;
        Ok(())
    }
    pub(super) fn check_item_context(
        &self,
        names: &impl NameAccess,
        e: ExpansionId,
        function: ScopeId,
    ) -> Result<(), ScopeError> {
        let p = self
            .item_product(function)?
            .ok_or(ScopeError::FunctionMismatch)?;
        if p.proof.expansion != e {
            return Err(ScopeError::FunctionMismatch);
        }
        self.check_item_proof(&p.proof)?;
        p.proof.check_names(names)
    }
    pub(crate) fn check_occurrence_in(
        &self,
        names: &impl NameAccess,
        modules: &impl ModuleAccess,
        ident: AstIdent,
        role: OccurrenceRole,
    ) -> Result<(), ScopeError> {
        self.check_names(names)?;
        let module = match role {
            OccurrenceRole::OriginalModule(m)
            | OccurrenceRole::GeneratedDeclaration { module: m, .. }
            | OccurrenceRole::Function { module: m, .. } => m,
        };
        let root = modules.source_root()?;
        check_modules(names, modules, root)?;
        if modules.root_of(module)? != root {
            return Err(ModuleError::InvalidParent.into());
        }
        // The immutable function association chooses this domain. Looking at
        // the candidate span first would let an unrelated original root name
        // bypass generated-body provenance by replacing a generated type/name.
        match role {
            OccurrenceRole::Function { function, .. } => {
                if let Some(product) = self.item_product(function)? {
                    self.check_generated_function(
                        names,
                        modules,
                        function,
                        product.declaration,
                        module,
                    )?;
                    return self.check_item_body_occurrence(names, ident, function);
                }
            }
            OccurrenceRole::GeneratedDeclaration {
                function,
                declaration,
                ..
            } => {
                self.check_generated_function(names, modules, function, declaration, module)?;
                if self.item_product(function)?.is_none()
                    || semantic_name::occurrence_index(names, ident)?
                        != semantic_name::occurrence_index(names, declaration)?
                {
                    return Err(ScopeError::ConflictingAssociation);
                }
                return self.check_item_body_occurrence(names, ident, function);
            }
            OccurrenceRole::OriginalModule(_) => {}
        }
        let span = names.session().span(names.span(ident)?)?;
        if let Some(e) = span.expansion
            && self.item_anchor(e)?.is_some()
        {
            return match role {
                OccurrenceRole::OriginalModule(_) => Err(ScopeError::FunctionMismatch),
                OccurrenceRole::GeneratedDeclaration {
                    function,
                    declaration,
                    ..
                } => {
                    self.check_generated_function(names, modules, function, declaration, module)?;
                    if semantic_name::occurrence_index(names, ident)?
                        != semantic_name::occurrence_index(names, declaration)?
                    {
                        return Err(ScopeError::ConflictingAssociation);
                    }
                    self.check_item_body_occurrence(names, ident, function)
                }
                OccurrenceRole::Function { function, .. } => {
                    let p = self
                        .item_product(function)?
                        .ok_or(ScopeError::FunctionMismatch)?;
                    self.check_generated_function(names, modules, function, p.declaration, module)?;
                    self.check_item_body_occurrence(names, ident, function)
                }
            };
        }
        let function = match role {
            OccurrenceRole::OriginalModule(_) => None,
            OccurrenceRole::GeneratedDeclaration { function, .. }
            | OccurrenceRole::Function { function, .. } => Some(function),
        };
        self.check_occurrence(names, ident, function)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::expansion_span::ExpansionRecord;
    use crate::compiler::semantic_name::NameStoreBuilder;
    use crate::compiler::token::{Edition, Ident, TokenKind};
    use crate::compiler::token_session::SourceSnapshot;
    use std::path::Path;

    struct Fixture {
        names: NameStoreBuilder,
        modules: ModuleArenaBuilder,
        original: SpanId,
        expansion: ExpansionId,
        marked: Token,
        captured: Token,
    }
    fn fixture() -> Fixture {
        let budget = ExpansionBudget::default();
        let source = SourceSnapshot::from_text(Path::new("item.rs"), "x", &budget).unwrap();
        let file = source.file(0).unwrap();
        let physical = source.span(file, 0, 1).unwrap();
        let mut session = TokenSession::new(source, budget).unwrap();
        let original = session.original_span(physical).unwrap();
        let defining_crate = session.register_crate_root(file).unwrap();
        let expansion = session
            .add_expansion(ExpansionRecord {
                definition: original,
                call: original,
                parent: None,
                defining_crate,
                edition: Edition::E2024,
            })
            .unwrap();
        let root = session.root_context();
        let marked_context = session
            .apply_mark(root, expansion, Transparency::SemiTransparent)
            .unwrap();
        let symbol = session.intern_symbol("x").unwrap();
        let mut token = |context| Token {
            kind: TokenKind::Ident(Ident {
                symbol,
                raw: false,
                context,
            }),
            span: session.reissue_span(original, context, expansion).unwrap(),
        };
        let marked = token(marked_context);
        let captured = token(root);
        let names = NameStoreBuilder::new(session, Edition::E2024).unwrap();
        let modules = ModuleArenaBuilder::for_source(&names).unwrap();
        Fixture {
            names,
            modules,
            original,
            expansion,
            marked,
            captured,
        }
    }
    fn anchor(f: &Fixture, scopes: &mut ScopeArenaBuilder) -> ItemExpansionAdmission {
        let module = f.modules.source_root().unwrap();
        let point = scopes
            .module_root(&f.names, &f.modules, module, None)
            .unwrap();
        scopes
            .anchor_item_expansion(&f.names, &f.modules, f.expansion, point, point)
            .unwrap()
    }

    #[test]
    fn module_entry_freezes_parent_textual_frontier_and_children_do_not_escape() {
        let mut f = fixture();
        let child_name = f.names.intern_root("child", f.original).unwrap();
        let root = f.modules.source_root().unwrap();
        let child = f.modules.intern_child(&f.names, root, child_name).unwrap();
        let mut scopes = ScopeArenaBuilder::for_names(&f.names).unwrap();
        let early = scopes
            .module_root(&f.names, &f.modules, root, None)
            .unwrap();
        let entered = scopes
            .module_root(&f.names, &f.modules, child, Some(early))
            .unwrap();
        let late = scopes.module_step(&f.names, &f.modules, early).unwrap();
        let child_late = scopes.module_step(&f.names, &f.modules, entered).unwrap();
        assert!(scopes.module_visible_from(early, child_late).unwrap());
        assert!(!scopes.module_visible_from(late, child_late).unwrap());
        assert!(!scopes.module_visible_from(child_late, late).unwrap());
        assert!(
            scopes
                .module_root(&f.names, &f.modules, child, None)
                .is_err()
        );
        assert!(
            scopes
                .module_root(&f.names, &f.modules, root, Some(entered))
                .is_err()
        );
    }

    #[test]
    fn item_mark_miss_never_captures_root_parameter_but_exact_binders_work() {
        let mut f = fixture();
        let mut scopes = ScopeArenaBuilder::for_names(&f.names).unwrap();
        let proof = anchor(&f, &mut scopes);
        let declaration = f
            .names
            .admit_item_token(f.marked, Edition::E2024, &proof)
            .unwrap();
        let start = scopes
            .generated_function_root(&f.names, &f.modules, &proof, declaration)
            .unwrap();
        let root_x = f
            .names
            .admit_item_token(f.captured, Edition::E2024, &proof)
            .unwrap();
        let (with_parameter, parameter) = scopes.bind(&f.names, start, root_x).unwrap();
        let marked_use = f
            .names
            .admit_item_token(f.marked, Edition::E2024, &proof)
            .unwrap();
        assert_eq!(
            scopes
                .resolve_reference(&f.names, with_parameter, marked_use)
                .unwrap(),
            None
        );
        let captured_use = f
            .names
            .admit_item_token(f.captured, Edition::E2024, &proof)
            .unwrap();
        assert_eq!(
            scopes
                .resolve_reference(&f.names, with_parameter, captured_use)
                .unwrap(),
            Some(parameter)
        );
        let marked_binder = f
            .names
            .admit_item_token(f.marked, Edition::E2024, &proof)
            .unwrap();
        let (with_local, local) = scopes
            .bind(&f.names, with_parameter, marked_binder)
            .unwrap();
        let local_use = f
            .names
            .admit_item_token(f.marked, Edition::E2024, &proof)
            .unwrap();
        assert_eq!(
            scopes
                .resolve_reference(&f.names, with_local, local_use)
                .unwrap(),
            Some(local)
        );
        assert_ne!(parameter, local);
        let function = scopes.function(start).unwrap();
        scopes
            .seal_item_product(&f.names, &f.modules, &proof, function, declaration)
            .unwrap();
        let module = f.modules.source_root().unwrap();
        scopes
            .check_occurrence_in(
                &f.names,
                &f.modules,
                local_use,
                OccurrenceRole::Function { module, function },
            )
            .unwrap();
        let transplanted = f.names.intern_root("x", f.original).unwrap();
        assert!(
            scopes
                .check_occurrence_in(
                    &f.names,
                    &f.modules,
                    transplanted,
                    OccurrenceRole::Function { module, function }
                )
                .is_err()
        );
        assert!(
            scopes
                .check_occurrence_in(
                    &f.names,
                    &f.modules,
                    local_use,
                    OccurrenceRole::OriginalModule(module)
                )
                .is_err()
        );
        assert!(
            scopes
                .generated_function_root(&f.names, &f.modules, &proof, declaration)
                .is_err()
        );
    }

    #[test]
    fn sealed_product_requires_exact_actual_function_coverage_even_when_empty() {
        let mut f = fixture();
        let mut scopes = ScopeArenaBuilder::for_names(&f.names).unwrap();
        let proof = anchor(&f, &mut scopes);
        let declaration = f
            .names
            .admit_item_token(f.marked, Edition::E2024, &proof)
            .unwrap();
        let start = scopes
            .generated_function_root(&f.names, &f.modules, &proof, declaration)
            .unwrap();
        let function = scopes.function(start).unwrap();
        assert!(scopes.check_item_function_coverage(&[]).is_err());
        scopes
            .seal_item_product(&f.names, &f.modules, &proof, function, declaration)
            .unwrap();
        assert!(scopes.check_item_function_coverage(&[]).is_err());
        assert!(
            scopes
                .check_item_function_coverage(&[function, function])
                .is_err()
        );
        let ordinary_start = scopes.function_root(&f.names).unwrap();
        let ordinary = scopes.function(ordinary_start).unwrap();
        let original = f.names.intern_root("x", f.original).unwrap();
        scopes
            .check_generated_function(
                &f.names,
                &f.modules,
                ordinary,
                original,
                f.modules.source_root().unwrap(),
            )
            .unwrap();
        assert!(scopes.check_item_function_coverage(&[ordinary]).is_err());
        assert!(
            scopes
                .check_generated_function(
                    &f.names,
                    &f.modules,
                    function,
                    original,
                    f.modules.source_root().unwrap()
                )
                .is_err()
        );
        scopes.check_item_function_coverage(&[function]).unwrap();
        let frozen = scopes.freeze().unwrap();
        frozen.check_item_authority(&f.names, &f.modules).unwrap();
        frozen.check_item_function_coverage(&[function]).unwrap();
    }

    #[test]
    fn item_anchors_reject_foreign_owners_and_freeze_rejects_unconsumed_output() {
        let f = fixture();
        let mut scopes = ScopeArenaBuilder::for_names(&f.names).unwrap();
        let proof = anchor(&f, &mut scopes);
        let foreign = fixture();
        assert!(proof.check_names(&foreign.names).is_err());
        let same_names_foreign_modules = ModuleArenaBuilder::for_source(&f.names).unwrap();
        assert!(
            proof
                .check_modules(&f.names, &same_names_foreign_modules)
                .is_err()
        );
        assert!(scopes.freeze().is_err());
    }

    #[test]
    fn item_authority_preserves_latched_resource_failure_and_releases_storage() {
        let f = fixture();
        let budget = f.names.budget().clone();
        let mut scopes = ScopeArenaBuilder::for_names(&f.names).unwrap();
        let proof = anchor(&f, &mut scopes);
        let failure = budget.charge(Counter::Work, u64::MAX).unwrap_err();
        assert!(
            matches!(proof.check_names(&f.names), Err(ScopeError::Token(TokenError::Budget(error))) if error == failure)
        );
        assert!(scopes.check_item_authority(&f.names, &f.modules).is_err());
        drop(scopes);
        drop(f);
        assert_eq!(budget.snapshot().retained_bytes, 0);
        assert_eq!(budget.snapshot().temporary_bytes, 0);
    }
}
