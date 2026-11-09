//! E0446: an associated type value in an implementation of a public trait
//! for a public type may not name a less visible type or trait.
use super::*;

impl Resolver<'_> {
    fn nominal_visibility(
        &mut self,
        path: &AstPath,
        module: ModuleId,
    ) -> Result<Option<ir::Visibility>, String> {
        Ok(self
            .path(path, module, true, false)
            .ok()
            .flatten()
            .map(|item| item.visibility))
    }

    /// Whether every nominal and trait named by `ty` is `pub`. Unknown or
    /// catalog targets count as public.
    fn type_is_public(&mut self, ty: &Type, module: ModuleId) -> Result<Option<String>, String> {
        let mut stack = vec![ty];
        while let Some(ty) = stack.pop() {
            self.alias_budget.charge_work(1)?;
            match ty {
                Type::Named(path) => {
                    if self
                        .nominal_visibility(path, module)?
                        .is_some_and(|v| !v.is_public())
                    {
                        return Ok(Some(self.display(path)?.to_string()));
                    }
                }
                Type::Apply(path) => {
                    let name = path.path(self.names())?;
                    if path.anchor != PathAnchor::SelfType
                        && self
                            .nominal_visibility(&name, module)?
                            .is_some_and(|v| !v.is_public())
                    {
                        return Ok(Some(self.display(&name)?.to_string()));
                    }
                    for segment in &path.segments {
                        stack.extend(segment.args.iter());
                    }
                }
                Type::Dyn(trait_path) => {
                    if let TraitPath::Source(path) = trait_path.as_ref() {
                        let name = path.path(self.names())?;
                        if self
                            .nominal_visibility(&name, module)?
                            .is_some_and(|v| !v.is_public())
                        {
                            return Ok(Some(self.display(&name)?.to_string()));
                        }
                        for segment in &path.segments {
                            stack.extend(segment.args.iter());
                        }
                    }
                }
                Type::Ref {
                    referent: inner, ..
                }
                | Type::Array(inner, _)
                | Type::Slice(inner)
                | Type::RawPtr { pointee: inner, .. } => stack.push(inner),
                Type::Tuple(items) => stack.extend(items.iter()),
                Type::PhantomData(source) | Type::ManuallyDrop(source) | Type::Cell(source) => {
                    if let Some(argument) = &source.argument {
                        stack.push(argument);
                    }
                }
                _ => {}
            }
        }
        Ok(None)
    }

    pub(super) fn check_private_in_public_impl(&mut self, block: &ImplDecl) -> Result<(), String> {
        let Some(TraitPath::Source(trait_path)) = &block.trait_path else {
            return Ok(());
        };
        if block.associated_types.is_empty() {
            return Ok(());
        }
        let trait_name = trait_path.path(self.names())?;
        if !self
            .nominal_visibility(&trait_name, block.module)?
            .is_some_and(|v| v.is_public())
        {
            return Ok(());
        }
        for segment in &trait_path.segments {
            for argument in &segment.args {
                if self.type_is_public(argument, block.module)?.is_some() {
                    return Ok(());
                }
            }
        }
        if self.type_is_public(&block.self_ty, block.module)?.is_some() {
            return Ok(());
        }
        for associated in &block.associated_types {
            if let Some(name) = self.type_is_public(&associated.value, block.module)? {
                return Err(format!(
                    "E0446: private type or trait `{name}` in public interface"
                ));
            }
        }
        Ok(())
    }
}
