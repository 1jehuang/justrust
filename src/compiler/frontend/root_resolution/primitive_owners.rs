//! Primitive type syntax as an associated-call owner, after source lookup.
//! This grants no method authority. Ordinary trait selection still resolves
//! the exact declaration, implementation and checked source body.
use super::*;

impl Resolver<'_> {
    pub(super) fn primitive_associated_path(
        &mut self,
        prefix: &GenericPath,
        item: &PathSegment,
        module: ModuleId,
    ) -> Result<Option<(Type, AstIdent, Vec<Type>)>, String> {
        if prefix.anchor != PathAnchor::Relative || prefix.segments.len() != 1 {
            return Ok(None);
        }
        let segment = &prefix.segments[0];
        if !segment.args.is_empty() || !segment.region_args.is_empty() {
            return Ok(None);
        }
        let spelling = self.ident_display(segment.name)?;
        let owner = if let Some(integer) = integer_type(spelling) {
            Type::Int(integer)
        } else if let Some(float) = float_type(spelling) {
            Type::Float(float)
        } else {
            match spelling {
                "char" => Type::Char,
                "bool" => Type::Bool,
                "str" => Type::Str,
                _ => return Ok(None),
            }
        };
        self.resolved_associated_owner(owner, item, module)
    }

    pub(super) fn resolved_associated_owner(
        &mut self,
        mut owner: Type,
        item: &PathSegment,
        module: ModuleId,
    ) -> Result<Option<(Type, AstIdent, Vec<Type>)>, String> {
        self.ty(&mut owner, module)?;
        clone_vector(&item.args, &self.alias_budget)?;
        let mut args = item.args.clone();
        for arg in &mut args {
            self.ty(arg, module)?;
        }
        Ok(Some((owner, item.name, args)))
    }
}
