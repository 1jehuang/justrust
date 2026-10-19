//! Metadata parsed from the genuine expanded token buffer, never printed and relexed.
//! Consumers must attach each record to its emitted item before publication.
use super::*;
use crate::compiler::expansion_span::SpanId;

#[cfg(test)]
mod tests {
    use super::*;

    fn program() -> Program {
        parse("macro_rules! item{()=>{#[doc=\"first\"]fn first(){}#[doc=\"second\"]fn second(){}}}item!();fn main(){}").unwrap()
    }

    #[test]
    fn generated_copy_binding_rejects_nominal_and_impl_transplants() {
        let original = parse("macro_rules! item{()=>{#[derive(Copy)]struct A;#[derive(Copy)]struct B;}}item!();fn main(){}").unwrap();
        validate(&original).unwrap();
        assert_eq!(original.generated_metadata.len(), 2);
        let mut moved = original.clone();
        let mut attachments = moved.generated_metadata.to_vec();
        attachments[0].copies[0].nominal = attachments[1].copies[0].nominal;
        moved.generated_metadata = attachments.into();
        assert!(validate(&moved).is_err());
        let mut moved = original.clone();
        let mut attachments = moved.generated_metadata.to_vec();
        attachments[0].copies[0].implementation = attachments[1].copies[0].implementation;
        moved.generated_metadata = attachments.into();
        assert!(validate(&moved).is_err());
        let mut moved = original.clone();
        let mut attachments = moved.generated_metadata.to_vec();
        attachments[0].copies.clear();
        moved.generated_metadata = attachments.into();
        assert!(validate(&moved).is_err());

        let mut duplicate =
            parse("macro_rules! item{()=>{#[derive(Copy,Copy)]struct A;}}item!();fn main(){}")
                .unwrap();
        validate(&duplicate).unwrap();
        let mut attachments = duplicate.generated_metadata.to_vec();
        attachments[0].copies[1].implementation = attachments[0].copies[0].implementation;
        duplicate.generated_metadata = attachments.into();
        assert!(validate(&duplicate).is_err());
    }

    fn combined_program() -> Program {
        use crate::compiler::source;
        let options = source::SourceOptions::default();
        let budget = crate::compiler::expansion_budget::ExpansionBudget::default();
        let unit = source::prepare_text_unit(
            "generated-authority.rs",
            "macro_rules! item{()=>{#[derive(Debug,Clone,Copy)]struct A;#[derive(Debug,Clone,Copy)]struct B;}}item!();fn main(){}",
            &options,
        );
        // The convenience frontend intentionally assembles user-only text. These
        // tests require the same real embedded source attachment as graph PARSE,
        // not a hand-built fmt declaration or a fabricated canonical trait ID.
        let mut assembler = source::SourceAssembler::new(&options);
        let mut step = assembler.provide(Arc::new(unit)).unwrap();
        let prepared = loop {
            match step {
                source::AssemblyStep::Done(prepared) => break prepared,
                source::AssemblyStep::Need(need) => {
                    let unit = match need.parent.as_deref() {
                        None => source::prepare_prelude_unit(&options, &budget),
                        Some(parent) => {
                            source::prepare_child_unit(parent, need.index, &options, &budget)
                        }
                    };
                    step = assembler.provide(Arc::new(unit)).unwrap();
                }
            }
        };
        resolve(&parse_prepared(&prepared).unwrap()).unwrap()
    }

    #[test]
    fn combined_derive_rejects_same_expansion_member_transplants() {
        let original = combined_program();
        validate(&original).unwrap();
        let mut moved = original.clone();
        let mut attachments = moved.generated_metadata.to_vec();
        attachments[0].debug[0].1.scope = attachments[1].debug[0].1.scope;
        moved.generated_metadata = attachments.into();
        assert!(validate(&moved).is_err());

        let mut moved = original.clone();
        let mut attachments = moved.generated_metadata.to_vec();
        attachments[0].debug[0].1.implementation = attachments[1].debug[0].1.implementation;
        moved.generated_metadata = attachments.into();
        assert!(validate(&moved).is_err());

        let mut foreign = combined_program();
        foreign.generated_metadata = original.generated_metadata;
        assert!(validate(&foreign).is_err());
    }

    #[test]
    fn derived_signature_authority_never_authorizes_generated_body_names() {
        let original = combined_program();
        let recipe = &original.generated_metadata[0].debug[0].1;
        let index = recipe.implementation.unwrap();
        let function = &original.impls[index].items[0].function;
        let module = generated_debug::signature_module(&original, function)
            .unwrap()
            .unwrap();
        assert_eq!(module, original.traits[recipe.source_trait.unwrap()].module);
        name_validation::validate_function_names(&original, function, Some(function.module))
            .unwrap();

        let mut changed = function.clone();
        changed.result = Type::Bool;
        assert!(generated_debug::signature_module(&original, &changed).is_err());

        // A genuine source signature occurrence is not executable body authority.
        // Even with an otherwise exact generated signature, moving fmt's original
        // declaration occurrence into the generated body must remain a contract error.
        let foreign = original.traits[recipe.source_trait.unwrap()].name.clone();
        let mut changed = function.clone();
        changed.body.tail = Some(Box::new(Expr::Var(foreign)));
        assert!(
            name_validation::validate_function_names(&original, &changed, Some(changed.module))
                .is_err()
        );

        let mut changed = original.clone();
        changed.impls[index].items[0].function.visibility = ir::Visibility::Public;
        assert!(validate(&changed).is_err());
    }

    #[test]
    fn metadata_owner_and_order_are_exact_not_same_expansion_membership() {
        let original = program();
        validate(&original).unwrap();
        assert_eq!(original.generated_metadata.len(), 2);
        let mut moved = original.clone();
        let mut attachments = moved.generated_metadata.to_vec();
        attachments[0].owner = attachments[1].owner;
        moved.generated_metadata = attachments.into();
        assert!(validate(&moved).is_err());

        let mut moved = original.clone();
        let mut attachments = moved.generated_metadata.to_vec();
        attachments[0].attributes[0].close = attachments[1].attributes[0].close;
        moved.generated_metadata = attachments.into();
        assert!(validate(&moved).is_err());

        let mut foreign = program();
        foreign.generated_metadata = original.generated_metadata;
        assert!(validate(&foreign).is_err());
    }
}

#[derive(Clone, Debug)]
pub(super) struct Attachment {
    binding: crate::compiler::semantic_scope::ItemMetadataId,
    scope: ModuleId,
    admission: crate::compiler::semantic_scope::ItemExpansionAdmission,
    owner: SpanId,
    attributes: Vec<Attribute>,
    copies: Vec<generated_derive::CopyEmission>,
    debug: Vec<(AstIdent, generated_debug::DebugRecipe)>,
}

pub(super) fn debug_recipes(
    attachments: &[Attachment],
) -> impl Iterator<Item = &generated_debug::DebugRecipe> {
    attachments
        .iter()
        .flat_map(|attachment| attachment.debug.iter().map(|(_, recipe)| recipe))
}

pub(super) fn materialize_debug(
    program: &mut Program,
    source: Option<usize>,
) -> Result<std::ops::Range<usize>, String> {
    let start = program.impls.len();
    if !debug_recipes(&program.generated_metadata).any(|recipe| recipe.implementation.is_none()) {
        return Ok(start..start);
    }
    let source = source.ok_or(
        "unsupported: generated Debug requires the canonical native source fmt::Debug declaration",
    )?;
    let owner = &program.traits[source];
    let mut member = None;
    for (index, item) in owner.items.iter().enumerate() {
        program
            .names
            .budget()
            .charge(crate::compiler::expansion_budget::Counter::Work, 1)
            .map_err(|error| error.to_string())?;
        if program
            .names
            .spelling(item.signature.name)
            .map_err(native_adapter::name_diagnostic)?
            == "fmt"
            && member.replace(index).is_some()
        {
            return Err("duplicate canonical Debug method".into());
        }
    }
    let member = member.ok_or("canonical source Debug has no fmt member")?;
    let mut attachments = program.generated_metadata.to_vec();
    for attachment in &mut attachments {
        for (_, recipe) in &mut attachment.debug {
            if recipe.implementation.is_some() {
                return Err("generated Debug was already materialized".into());
            }
            let implementation = generated_debug::construct(program, recipe, source, member)?;
            recipe.implementation = Some(program.impls.len());
            recipe.source_trait = Some(source);
            recipe.source_member = Some(member);
            program
                .impls
                .try_reserve_exact(1)
                .map_err(|_| "generated Debug impl allocation failed")?;
            program.impls.push(implementation);
        }
    }
    program.generated_metadata = attachments.into();
    Ok(start..program.impls.len())
}

pub(super) fn rebase(
    attachments: &mut [Attachment],
    nominal_offset: usize,
    implementation_offset: usize,
) -> Result<(), ParseError> {
    for attachment in attachments {
        for (_, recipe) in &mut attachment.debug {
            recipe.nominal = recipe.nominal.checked_add(nominal_offset).ok_or_else(|| {
                native_adapter::transport_error(
                    crate::compiler::token_session::TokenError::IdExhausted,
                )
            })?;
        }
        for emitted in &mut attachment.copies {
            emitted.nominal = emitted.nominal.checked_add(nominal_offset).ok_or_else(|| {
                native_adapter::transport_error(
                    crate::compiler::token_session::TokenError::IdExhausted,
                )
            })?;
            emitted.implementation = emitted
                .implementation
                .checked_add(implementation_offset)
                .ok_or_else(|| {
                    native_adapter::transport_error(
                        crate::compiler::token_session::TokenError::IdExhausted,
                    )
                })?;
        }
    }
    Ok(())
}

pub(super) fn validate(program: &Program) -> Result<(), String> {
    for attachment in program.generated_metadata.iter() {
        let spans = ordered_spans(&attachment.attributes, &*program.names)?;
        program
            .scopes
            .as_ref()
            .ok_or("generated metadata has no scope arena")?
            .validate_item_metadata(
                &*program.names,
                &*program.modules,
                attachment.binding,
                &attachment.admission,
                attachment.scope,
                attachment.owner,
                &spans,
            )
            .map_err(|error| format!("generated metadata owner association: {error:?}"))?;
        attachment
            .admission
            .check_modules(&*program.names, &*program.modules)
            .map_err(|error| format!("generated metadata admission: {error:?}"))?;
        let check = |span| {
            attachment
                .admission
                .validate_span(program.names.session(), span)
                .map_err(|error| format!("generated metadata occurrence: {error:?}"))
        };
        check(attachment.owner)?;
        let derives = attachment
            .attributes
            .iter()
            .flat_map(|attribute| match &attribute.value {
                Value::Derive(names) => names.as_slice(),
                _ => &[],
            });
        let debug_count = derives.clone().try_fold(0usize, |count, name| {
            let spelling = program
                .names
                .spelling(*name)
                .map_err(native_adapter::name_diagnostic)?;
            Ok::<_, String>(count + usize::from(spelling == "Debug"))
        })?;
        if debug_count != attachment.debug.len() {
            return Err("generated Debug emission inventory changed".into());
        }
        let derives = derives.filter(|name| {
            program
                .names
                .spelling(**name)
                .is_ok_and(|name| name != "Debug")
        });
        let expected_debug = attachment
            .attributes
            .iter()
            .flat_map(|attribute| match &attribute.value {
                Value::Derive(names) => names.as_slice(),
                _ => &[],
            })
            .filter(|name| {
                program
                    .names
                    .spelling(**name)
                    .is_ok_and(|name| name == "Debug")
            });
        for (expected, (derive, _)) in expected_debug.zip(&attachment.debug) {
            if !exact_source_ident(&program.names, *expected, *derive)? {
                return Err("generated Debug derive order/occurrence changed".into());
            }
        }
        for (derive, recipe) in &attachment.debug {
            let declaration = program
                .structs
                .get(recipe.nominal)
                .ok_or("generated Debug nominal missing")?;
            let nominal = *declaration
                .name
                .segments()
                .last()
                .ok_or("generated Debug nominal unnamed")?;
            program
                .scopes
                .as_ref()
                .ok_or("generated Debug scope missing")?
                .validate_derived_member(
                    &*program.names,
                    &*program.modules,
                    &recipe.admission,
                    attachment.binding,
                    program
                        .names
                        .span(*derive)
                        .map_err(native_adapter::name_diagnostic)?,
                    program
                        .names
                        .span(nominal)
                        .map_err(native_adapter::name_diagnostic)?,
                )
                .map_err(|error| format!("generated Debug attachment changed: {error:?}"))?;
            let mut retained = false;
            for attribute in &attachment.attributes {
                if let Value::Derive(names) = &attribute.value {
                    for name in names {
                        retained |= exact_source_ident(&program.names, *name, *derive)?;
                    }
                }
            }
            if !retained {
                return Err("generated Debug derive absent from original metadata".into());
            }
            generated_debug::validate_recipe(program, recipe)?;
        }
        let expected_count = derives.clone().count();
        if expected_count != attachment.copies.len()
            || attachment
                .copies
                .windows(2)
                .any(|pair| pair[0].implementation >= pair[1].implementation)
        {
            return Err("generated Copy emission inventory changed".into());
        }
        for (derive, emitted) in derives.zip(&attachment.copies) {
            let derive_span = program
                .names
                .span(*derive)
                .map_err(|error| error.to_string())?;
            if program
                .names
                .span(emitted.derive)
                .map_err(|error| error.to_string())?
                != derive_span
            {
                return Err("generated Copy derive occurrence changed".into());
            }
            let declaration = program
                .structs
                .get(emitted.nominal)
                .ok_or("generated Copy nominal outside source inventory")?;
            let implementation = program
                .impls
                .get(emitted.implementation)
                .ok_or("generated Copy impl outside source inventory")?;
            let nominal = declaration
                .name
                .segments()
                .last()
                .copied()
                .ok_or("generated Copy nominal has no original name")?;
            match &emitted.proof {
                generated_derive::EmissionProof::Copy(proof) => {
                    program
                        .scopes
                        .as_ref()
                        .ok_or("generated Copy has no scope arena")?
                        .validate_item_copy(
                            &*program.names,
                            *proof,
                            attachment.binding,
                            derive_span,
                            program
                                .names
                                .span(nominal)
                                .map_err(|error| error.to_string())?,
                        )
                        .map_err(|error| {
                            format!("generated Copy emission association: {error:?}")
                        })?;
                    generated_derive::validate_unit_copy(
                        &*program.names,
                        declaration,
                        implementation,
                    )?;
                }
                generated_derive::EmissionProof::Clone(proof) => {
                    program
                        .scopes
                        .as_ref()
                        .ok_or("generated Clone has no scope arena")?
                        .validate_derived_member(
                            &*program.names,
                            &*program.modules,
                            proof,
                            attachment.binding,
                            derive_span,
                            program
                                .names
                                .span(nominal)
                                .map_err(|error| error.to_string())?,
                        )
                        .map_err(|error| {
                            format!("generated Clone emission association: {error:?}")
                        })?;
                    generated_derive::validate_unit_clone(
                        &*program.names,
                        declaration,
                        implementation,
                        proof,
                        program
                            .scopes
                            .as_ref()
                            .ok_or("generated Clone scope missing")?,
                    )?;
                }
            }
        }
        for attribute in &attachment.attributes {
            check(attribute.open)?;
            check(attribute.close)?;
            check(
                program
                    .names
                    .span(attribute.name)
                    .map_err(|error| error.to_string())?,
            )?;
            let mut pending = Vec::new();
            if let Value::Doc(doc) = &attribute.value {
                pending.push(doc);
            }
            while let Some(doc) = pending.pop() {
                match doc {
                    Doc::Literal { site, text } => {
                        check(*site)?;
                        let _ = text;
                    }
                    Doc::Concat { name, site, parts } => {
                        check(*site)?;
                        check(
                            program
                                .names
                                .span(*name)
                                .map_err(|error| error.to_string())?,
                        )?;
                        pending.extend(parts);
                    }
                    Doc::Stringify { name, site, tokens } => {
                        check(*site)?;
                        check(
                            program
                                .names
                                .span(*name)
                                .map_err(|error| error.to_string())?,
                        )?;
                        for token in tokens {
                            check(*token)?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn ordered_spans(
    attributes: &[Attribute],
    names: &impl crate::compiler::semantic_path::NameAccess,
) -> Result<Vec<SpanId>, String> {
    let mut spans = Vec::new();
    for attribute in attributes {
        spans.push(attribute.open);
        spans.push(
            names
                .span(attribute.name)
                .map_err(|error| error.to_string())?,
        );
        let mut pending = Vec::new();
        if let Value::Doc(doc) = &attribute.value {
            pending.push(doc);
        }
        while let Some(doc) = pending.pop() {
            match doc {
                Doc::Literal { site, .. } => spans.push(*site),
                Doc::Concat { name, site, parts } => {
                    spans.push(*site);
                    spans.push(names.span(*name).map_err(|error| error.to_string())?);
                    pending.extend(parts.iter().rev());
                }
                Doc::Stringify { name, site, tokens } => {
                    spans.push(*site);
                    spans.push(names.span(*name).map_err(|error| error.to_string())?);
                    spans.extend(tokens);
                }
            }
        }
        if let Value::Derive(derives) = &attribute.value {
            for derive in derives {
                spans.push(names.span(*derive).map_err(|error| error.to_string())?);
            }
        }
        spans.push(attribute.close);
    }
    Ok(spans)
}

#[derive(Clone, Debug)]
pub(super) struct Attribute {
    pub(super) open: SpanId,
    pub(super) close: SpanId,
    pub(super) name: AstIdent,
    pub(super) value: Value,
}

#[derive(Clone, Debug)]
pub(super) enum Value {
    Doc(Doc),
    Hidden,
    Derive(Vec<AstIdent>),
}

#[derive(Clone, Debug)]
pub(super) enum Doc {
    Literal {
        site: SpanId,
        text: String,
    },
    Concat {
        name: AstIdent,
        site: SpanId,
        parts: Vec<Doc>,
    },
    Stringify {
        name: AstIdent,
        site: SpanId,
        tokens: Vec<SpanId>,
    },
}

impl Parser<'_, '_> {
    pub(super) fn finish_generated_metadata(
        &mut self,
        program: &mut ProgramDeclarations,
        metadata: std::ops::Range<usize>,
        nominal_start: usize,
    ) -> Result<(), ParseError> {
        for index in metadata {
            let mut derives = crate::compiler::token_session::BudgetVec::new(
                self.names.budget(),
                crate::compiler::expansion_budget::Storage::Temporary,
            )
            .map_err(native_adapter::transport_error)?;
            for attribute in &self.generated_metadata[index].attributes {
                if let Value::Derive(names) = &attribute.value {
                    for name in names {
                        derives
                            .push(*name)
                            .map_err(native_adapter::transport_error)?;
                    }
                }
            }
            if derives.len() == 0 {
                continue;
            }
            if program.structs.len()
                != nominal_start
                    .checked_add(1)
                    .ok_or_else(|| self.limit("generated nominal index overflow"))?
            {
                return Err(self.unsupported("generated Copy requires one original unit struct"));
            }
            let declaration = &program.structs[nominal_start];
            let nominal = declaration
                .name
                .segments()
                .last()
                .copied()
                .ok_or_else(|| self.error("generated struct has no declaration name"))?;
            let nominal = self
                .names
                .span(nominal)
                .map_err(native_adapter::name_error)?;
            for derive in derives.as_slice().iter().copied() {
                if self
                    .names
                    .spelling(derive)
                    .map_err(native_adapter::name_error)?
                    == "Debug"
                {
                    generated_derive::unit_copy(declaration)
                        .map_err(|message| self.unsupported(message))?;
                    let attachment = &self.generated_metadata[index];
                    let admission = self
                        .lexical
                        .scopes
                        .as_mut()
                        .ok_or_else(|| {
                            native_adapter::transport_error(
                                crate::compiler::token_session::TokenError::InvalidHandle,
                            )
                        })?
                        .admit_derived_member(
                            self.names,
                            &*self.modules,
                            attachment.binding,
                            &attachment.admission,
                            derive,
                            nominal,
                            crate::compiler::semantic_scope::DerivedMemberKind::Debug,
                        )
                        .map_err(scope_parse_error)?;
                    let recipe = self.unit_debug_recipe(nominal_start, admission)?;
                    self.names
                        .retain_parser_storage(std::mem::size_of::<(
                            AstIdent,
                            generated_debug::DebugRecipe,
                        )>())
                        .map_err(native_adapter::name_error)?;
                    self.generated_metadata[index]
                        .debug
                        .try_reserve_exact(1)
                        .map_err(|_| {
                            native_adapter::transport_error(
                                self.names.budget().poison_allocation().into(),
                            )
                        })?;
                    self.generated_metadata[index].debug.push((derive, recipe));
                    continue;
                }
                let _ = generated_derive::unit_copy(declaration)
                    .map_err(|message| self.unsupported(message))?;
                self.names
                    .retain_parser_storage(
                        std::mem::size_of::<ImplDecl>()
                            + std::mem::size_of::<generated_derive::CopyEmission>(),
                    )
                    .map_err(native_adapter::name_error)?;
                program.impls.try_reserve_exact(1).map_err(|_| {
                    native_adapter::transport_error(self.names.budget().poison_allocation().into())
                })?;
                self.generated_metadata[index]
                    .copies
                    .try_reserve_exact(1)
                    .map_err(|_| {
                        native_adapter::transport_error(
                            self.names.budget().poison_allocation().into(),
                        )
                    })?;
                let attachment = &self.generated_metadata[index];
                let derive_span = self
                    .names
                    .span(derive)
                    .map_err(native_adapter::name_error)?;
                let (proof, implementation) = if self
                    .names
                    .spelling(derive)
                    .map_err(native_adapter::name_error)?
                    == "Clone"
                {
                    let admission = self
                        .lexical
                        .scopes
                        .as_mut()
                        .ok_or_else(|| {
                            native_adapter::transport_error(
                                crate::compiler::token_session::TokenError::InvalidHandle,
                            )
                        })?
                        .admit_derived_member(
                            self.names,
                            &*self.modules,
                            attachment.binding,
                            &attachment.admission,
                            derive,
                            nominal,
                            crate::compiler::semantic_scope::DerivedMemberKind::Clone,
                        )
                        .map_err(scope_parse_error)?;
                    (
                        generated_derive::EmissionProof::Clone(admission),
                        self.unit_clone(declaration, admission)?,
                    )
                } else {
                    let proof = self
                        .lexical
                        .scopes
                        .as_mut()
                        .ok_or_else(|| {
                            native_adapter::transport_error(
                                crate::compiler::token_session::TokenError::InvalidHandle,
                            )
                        })?
                        .seal_item_copy(
                            self.names,
                            &*self.modules,
                            attachment.binding,
                            &attachment.admission,
                            derive_span,
                            nominal,
                        )
                        .map_err(scope_parse_error)?;
                    (
                        generated_derive::EmissionProof::Copy(proof),
                        generated_derive::unit_copy(declaration)
                            .map_err(|message| self.unsupported(message))?,
                    )
                };
                let implementation_index = program.impls.len();
                program.impls.push(implementation);
                self.generated_metadata[index]
                    .copies
                    .push(generated_derive::CopyEmission {
                        proof,
                        nominal: nominal_start,
                        implementation: implementation_index,
                        derive,
                    });
            }
        }
        Ok(())
    }
    fn generated_attribute_start(&self) -> bool {
        self.is("#") || (self.token().is_deferred_unsupported() && self.token().text == "#")
    }
    pub(super) fn retain_generated_item_metadata(&mut self) -> Result<(), ParseError> {
        if self.item_expansion.is_none() || !self.generated_attribute_start() {
            return Ok(());
        }
        let attributes = self.generated_item_attributes()?;
        if attributes
            .iter()
            .any(|attribute| matches!(attribute.value, Value::Derive(_)))
            && !(self.is("struct")
                || self.is("pub")
                    && self
                        .cursor
                        .peek(1)
                        .is_some_and(|token| token.matches("struct")))
        {
            return Err(self.unsupported("generated Copy currently requires a unit struct item"));
        }
        let owner = self.token().span;
        if self.is("") || self.is("}") || self.is(";") {
            return Err(self.error("generated outer attribute has no item owner"));
        }
        self.names
            .retain_parser_storage(std::mem::size_of::<Attachment>())
            .map_err(native_adapter::name_error)?;
        let admission = self.item_expansion.expect("checked expansion");
        let spans = ordered_spans(&attributes, &*self.names).map_err(|error| self.error(&error))?;
        let binding = self
            .lexical
            .scopes
            .as_mut()
            .ok_or_else(|| {
                native_adapter::transport_error(
                    crate::compiler::token_session::TokenError::InvalidHandle,
                )
            })?
            .seal_item_metadata(
                self.names,
                &*self.modules,
                &admission,
                self.module,
                owner,
                &spans,
            )
            .map_err(scope_parse_error)?;
        self.generated_metadata.push(Attachment {
            binding,
            scope: self.module,
            admission,
            owner,
            attributes,
            copies: Vec::new(),
            debug: Vec::new(),
        });
        Ok(())
    }
    pub(super) fn generated_item_attributes(&mut self) -> Result<Vec<Attribute>, ParseError> {
        if self.item_expansion.is_none() {
            return Err(self.error("generated metadata requires its item expansion admission"));
        }
        let mut attributes = Vec::new();
        while self.generated_attribute_start() {
            let open = self.token().span;
            self.item_expansion
                .as_ref()
                .expect("checked expansion")
                .validate_span(self.names.session(), open)
                .map_err(scope_parse_error)?;
            self.cursor.bump();
            if self.eat("!") {
                return Err(
                    self.unsupported("inner attributes in item macro output are unsupported")
                );
            }
            self.expect("[")?;
            let doc = self.is("doc");
            let derive = self.is("derive");
            let name = self.ident()?;
            let value = if doc {
                if self.eat("=") {
                    Value::Doc(self.generated_doc_value(0)?)
                } else {
                    self.expect("(")?;
                    self.expect("hidden")?;
                    self.eat(",");
                    self.expect(")")?;
                    Value::Hidden
                }
            } else if derive {
                self.expect("(")?;
                let mut names = Vec::new();
                while !self.is(")") {
                    self.names
                        .retain_parser_storage(std::mem::size_of::<AstIdent>())
                        .map_err(native_adapter::name_error)?;
                    if !(self.is("Copy") && self.builtin_macro_ahead("Copy")?
                        || self.is("Clone") && self.builtin_macro_ahead("Clone")?
                        || self.is("Debug") && self.builtin_macro_ahead("Debug")?)
                    {
                        return Err(self.unsupported("generated derive requires a supported canonical builtin emitter (currently unit Copy and Clone)"));
                    }
                    names.push(self.ident()?);
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(")")?;
                Value::Derive(names)
            } else {
                return Err(self.unsupported("unsupported active attribute in item macro output"));
            };
            let close = self.token().span;
            self.expect("]")?;
            self.names
                .retain_parser_storage(std::mem::size_of::<Attribute>())
                .map_err(native_adapter::name_error)?;
            attributes.push(Attribute {
                open,
                close,
                name,
                value,
            });
        }
        Ok(attributes)
    }

    fn generated_doc_value(&mut self, depth: usize) -> Result<Doc, ParseError> {
        if depth >= 64 {
            return Err(self.limit("generated documentation nesting exceeds metadata limit 64"));
        }
        let site = self.token().span;
        if self.token().is_opaque_literal() {
            let value = self.opaque_literal_expression()?;
            return match value {
                Expr::Str(text) => Ok(Doc::Literal { site, text }),
                _ => Err(self.error("documentation requires a string literal")),
            };
        }
        if self.token().string {
            let text = self.string_literal_value(self.token().text)?;
            self.cursor.bump();
            return Ok(Doc::Literal { site, text });
        }
        // A spelling match is insufficient. This is the same checked lexical and
        // module macro resolution used by executable builtin invocations.
        let concat = self.builtin_macro_ahead("concat")?;
        let stringify = !concat && self.builtin_macro_ahead("stringify")?;
        if !concat && !stringify {
            return Err(self.unsupported("documentation value requires a string or a resolved builtin concat/stringify invocation"));
        }
        let name = self.ident()?;
        self.expect("!")?;
        self.expect("(")?;
        if concat {
            let mut parts = Vec::new();
            while !self.is(")") {
                self.names
                    .retain_parser_storage(std::mem::size_of::<Doc>())
                    .map_err(native_adapter::name_error)?;
                parts.push(self.generated_doc_value(depth + 1)?);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
            Ok(Doc::Concat { name, site, parts })
        } else {
            // Preserve every original token occurrence. Rendering, if needed by
            // a documentation consumer, must not mint semantic names from text.
            let mut tokens = Vec::new();
            let mut nesting = 0usize;
            while nesting != 0 || !self.is(")") {
                if self.is("") {
                    return Err(self.error("unclosed stringify invocation"));
                }
                if self.is("(") {
                    nesting += 1;
                }
                if self.is(")") {
                    nesting -= 1;
                }
                self.names
                    .retain_parser_storage(std::mem::size_of::<SpanId>())
                    .map_err(native_adapter::name_error)?;
                tokens.push(self.token().span);
                self.cursor.bump();
            }
            self.expect(")")?;
            Ok(Doc::Stringify { name, site, tokens })
        }
    }
}
