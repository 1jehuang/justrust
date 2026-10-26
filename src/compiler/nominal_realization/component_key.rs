//! Exact typed domain 2 keys in the existing nominal realization operation.
//! Request coordinates never grant a successful graph read capability.
use super::{Error, KIND, Key, RealizationKey, VERSION, ir, producer, topology::storage};
pub(super) const DOMAIN: u64 = 2;

pub(super) fn application_words(
    key: &RealizationKey,
    b: &ir::PlanBudget,
) -> Result<Vec<u64>, Error> {
    b.charge_work(key.0.params.len()).map_err(Error::new)?;
    let mut words = storage(key.0.params.len(), b)?;
    words.extend_from_slice(&key.0.params);
    Ok(words)
}
pub(super) fn copy_application(
    key: &RealizationKey,
    b: &ir::PlanBudget,
) -> Result<RealizationKey, Error> {
    Ok(RealizationKey(crate::compiler::keys::words(
        KIND,
        application_words(key, b)?,
    )?))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ComponentKey(pub(super) Key);
impl ComponentKey {
    pub(super) fn encode(
        world: u64,
        members: &[RealizationKey],
        b: &ir::PlanBudget,
    ) -> Result<Self, Error> {
        if members.is_empty() {
            return Err(Error::new("empty nominal component"));
        }
        let mut ordered = storage(members.len(), b)?;
        let mut count = 4usize;
        for member in members {
            let (actual_world, _, _) = producer::decode(&member.0, b)?;
            if actual_world != world {
                return Err(Error::new("foreign-world nominal component member"));
            }
            count = count
                .checked_add(1)
                .and_then(|n| n.checked_add(member.0.params.len()))
                .ok_or_else(|| Error::new("nominal component key size overflow"))?;
            ordered.push(member);
        }
        b.charge_work(count.saturating_mul(members.len().max(1).ilog2() as usize + 1))
            .map_err(Error::new)?;
        ordered.sort_unstable_by(|a, b| a.0.params.cmp(&b.0.params));
        for pair in ordered.windows(2) {
            if pair[0] == pair[1] {
                return Err(Error::new("duplicate nominal component member"));
            }
        }
        let mut words = storage(count, b)?;
        words.extend([VERSION, DOMAIN, world, ordered.len() as u64]);
        for member in ordered {
            words.push(member.0.params.len() as u64);
            words.extend_from_slice(&member.0.params);
        }
        Ok(Self(crate::compiler::keys::words(KIND, words)?))
    }

    pub(super) fn decode(
        raw: &Key,
        b: &ir::PlanBudget,
    ) -> Result<(Self, u64, Vec<RealizationKey>), Error> {
        if raw.kind != KIND
            || raw.params.len() < 4
            || raw.params[0] != VERSION
            || raw.params[1] != DOMAIN
        {
            return Err(Error::new("invalid nominal component key domain"));
        }
        let count =
            usize::try_from(raw.params[3]).map_err(|_| Error::new("component count overflow"))?;
        if count == 0 || count > raw.params.len().saturating_sub(4) {
            return Err(Error::new("component count exceeds key input"));
        }
        let mut members = storage(count, b)?;
        let mut at = 4usize;
        for _ in 0..count {
            b.charge_work(1).map_err(Error::new)?;
            let len = usize::try_from(
                *raw.params
                    .get(at)
                    .ok_or_else(|| Error::new("missing component length"))?,
            )
            .map_err(|_| Error::new("component length overflow"))?;
            at = at
                .checked_add(1)
                .ok_or_else(|| Error::new("component offset overflow"))?;
            let end = at
                .checked_add(len)
                .ok_or_else(|| Error::new("component end overflow"))?;
            let input = raw
                .params
                .get(at..end)
                .ok_or_else(|| Error::new("truncated component member"))?;
            let mut words = storage(input.len(), b)?;
            words.extend_from_slice(input);
            let member = RealizationKey(crate::compiler::keys::words(KIND, words)?);
            producer::decode(&member.0, b)?;
            members.push(member);
            at = end;
        }
        if at != raw.params.len() {
            return Err(Error::new("trailing component key words"));
        }
        let canonical = Self::encode(raw.params[2], &members, b)?;
        if canonical.0 != *raw {
            return Err(Error::new("noncanonical component member order"));
        }
        Ok((canonical, raw.params[2], members))
    }
}
