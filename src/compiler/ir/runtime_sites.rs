//! Immutable source-derived runtime panic locations. Expression IDs belong to
//! exactly one checked function, never a process-global location registry.
use super::ExprId;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeLocation {
    pub path: Arc<str>,
    pub line: usize,
    pub column: usize,
}

impl RuntimeLocation {
    pub fn diagnostic_location(&self) -> String {
        format!("{}:{}:{}", self.path, self.line, self.column)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeSites(Arc<[(ExprId, Arc<RuntimeLocation>)]>);

impl RuntimeSites {
    pub(crate) fn from_source(
        mut sites: Vec<(ExprId, Arc<RuntimeLocation>)>,
    ) -> Result<Self, String> {
        sites.sort_unstable_by_key(|(expression, _)| expression.0);
        if sites.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err("runtime panic location has repeated expression identity".into());
        }
        Ok(Self(sites.into()))
    }

    pub fn get(&self, expression: ExprId) -> Option<&Arc<RuntimeLocation>> {
        let index = self
            .0
            .binary_search_by_key(&expression.0, |(id, _)| id.0)
            .ok()?;
        Some(&self.0[index].1)
    }

    pub fn validate(&self, expressions: usize) -> Result<(), String> {
        if self.0.iter().any(|(id, site)| {
            id.0 >= expressions || site.path.is_empty() || site.line == 0 || site.column == 0
        }) {
            return Err("invalid source-derived runtime panic location".into());
        }
        Ok(())
    }
}
