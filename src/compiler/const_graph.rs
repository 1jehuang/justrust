//! Per-item constant providers. Metadata, qualification, and executed value
//! dependencies are separate graph reads. No whole-program constant table.
use super::*;

impl CompilerProvider {
    pub(super) async fn constant_node(&self, cx: &Context, key: &Key) -> Result<(), Error> {
        if key.params.len() != 1 {
            return Err(Error::new(
                "constant key must contain exactly one declaration identity",
            ));
        }
        let index = usize::try_from(key.params[0])
            .map_err(|_| Error::new("constant declaration key overflow"))?;
        let facts = Facts { cx };
        let plan = facts.types().await?;
        let declaration = plan.constant_key_decl(index).map_err(Error::new)?;
        let node = |kind, id: ir::DeclId| crate::compiler::keys::item(kind, id.index as usize);
        match key.kind {
            CONST_DECL => {
                let program: Arc<Arc<frontend::Program>> = facts
                    .get(crate::compiler::keys::ResolveDebtRoot.key())
                    .await?;
                let source = plan
                    .publish_constant_declaration(program.as_ref(), declaration, &self.0.budget)
                    .map_err(Error::new)?;
                cx.publish_fact(0, source)
            }
            CONST_BODY => {
                let source: Arc<const_source::DeclarationFact> =
                    facts.get(node(CONST_DECL, declaration)).await?;
                if source.declaration != declaration {
                    return Err(Error::new(
                        "contract: constant metadata handle identity changed",
                    ));
                }
                let (mut direct, _direct_storage) =
                    self.scratch_vec::<ir::DeclId>(source.expressions.len())?;
                for expression in source.expressions.iter() {
                    self.0.budget.charge_work(1).map_err(Error::new)?;
                    if let const_source::SyntaxKind::Reference(id) = expression.kind {
                        direct.push(id);
                    }
                }
                self.0
                    .budget
                    .charge_work(direct.len().saturating_mul(usize::BITS as usize))
                    .map_err(Error::new)?;
                direct.sort_unstable();
                direct.dedup();
                let (mut handles, _handle_storage) = self.scratch_vec(direct.len())?;
                for id in &direct {
                    handles.push(cx.request(node(CONST_DECL, *id)).get(0));
                }
                let (mut references, _reference_storage) =
                    self.scratch_vec::<Arc<const_source::DeclarationFact>>(direct.len())?;
                for (id, handle) in direct.iter().zip(handles) {
                    let fact: Arc<const_source::DeclarationFact> = cx.read_fact(handle).await?;
                    if fact.declaration != *id {
                        return Err(Error::new(
                            "contract: direct constant metadata handle changed",
                        ));
                    }
                    references.push(fact);
                }
                let body = const_check::check(source, &references, &plan, &self.0.budget)
                    .map_err(Error::new)?;
                let body = Arc::new(body);
                cx.publish_fact(0, body.clone())?;
                let (mut qualifications, _qualification_storage) =
                    self.scratch_vec(direct.len())?;
                for id in &direct {
                    qualifications.push(cx.request(node(CONST_BODY, *id)).get(1));
                }
                let (mut proofs, _proofs_storage) =
                    self.scratch_vec::<Arc<const_check::QualificationFact>>(direct.len())?;
                for (id, handle) in direct.iter().zip(qualifications) {
                    let proof: Arc<const_check::QualificationFact> = cx.read_fact(handle).await?;
                    if proof.body().source().declaration != *id {
                        return Err(Error::new(
                            "contract: direct constant qualification identity changed",
                        ));
                    }
                    proofs.push(proof);
                }
                let proof = const_check::QualificationFact::new(body, &proofs, &self.0.budget)
                    .map_err(Error::new)?;
                cx.publish_fact(1, proof)
            }
            CONST_VALUE => {
                let request = cx.request(node(CONST_BODY, declaration));
                let body_handle = request.get(0);
                let qualification_handle = request.get(1);
                let body: Arc<Arc<const_check::BodyFact>> = cx.read_fact(body_handle).await?;
                let proof: Arc<const_check::QualificationFact> =
                    cx.read_fact(qualification_handle).await?;
                if !Arc::ptr_eq(body.as_ref(), proof.body())
                    || body.source().declaration != declaration
                {
                    return Err(Error::new(
                        "contract: constant qualification belongs to another body publication",
                    ));
                }
                let value = const_check::ValueFact::evaluate(
                    body.as_ref().clone(),
                    &proof,
                    &self.0.budget,
                    |id| {
                        let facts = Facts { cx };
                        async move {
                            let value: Arc<const_check::ValueFact> =
                                facts.get(node(CONST_VALUE, id)).await?;
                            Ok(value)
                        }
                    },
                )
                .await?;
                cx.publish_fact(0, value)
            }
            _ => Err(Error::new("contract: invalid constant provider kind")),
        }
    }
}
