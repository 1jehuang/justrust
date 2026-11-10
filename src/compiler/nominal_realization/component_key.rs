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
    /// Traverse the canonical envelope only. Member term authentication is
    /// still performed by the producer decoder or by matches_key at replay.
    fn write_envelope<'a>(
        world: u64,
        members: impl ExactSizeIterator<Item = &'a RealizationKey> + Clone,
        b: &ir::PlanBudget,
        mut word: impl FnMut(u64) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if members.len() == 0 {
            return Err(Error::new("empty nominal component"));
        }
        let mut previous: Option<&RealizationKey> = None;
        for member in members.clone() {
            b.charge_work(1).map_err(Error::new)?;
            if member.0.kind != KIND
                || member.0.version != 1
                || member.0.params.len() < 6
                || member.0.params[0] != VERSION
                || member.0.params[1] != super::CLOSED_REALIZATION
                || member.0.params[2] != world
            {
                return Err(Error::new("foreign or invalid component member envelope"));
            }
            if previous.is_some_and(|old| old.0.params >= member.0.params) {
                return Err(Error::new("noncanonical component member order"));
            }
            previous = Some(member);
        }
        for value in [VERSION, DOMAIN, world, members.len() as u64] {
            b.charge_work(1).map_err(Error::new)?;
            word(value)?;
        }
        for member in members {
            b.charge_work(1).map_err(Error::new)?;
            word(member.0.params.len() as u64)?;
            for &value in &member.0.params {
                b.charge_work(1).map_err(Error::new)?;
                word(value)?;
            }
        }
        Ok(())
    }

    /// Compare borrowed member envelopes without minting a key or validating
    /// their term payloads. Successful callers must still authenticate each
    /// member with the original source declaration and ordered actuals.
    pub(super) fn matches_members<'a>(
        &self,
        world: u64,
        members: impl ExactSizeIterator<Item = &'a RealizationKey> + Clone,
        b: &ir::PlanBudget,
    ) -> Result<bool, Error> {
        if self.0.kind != KIND || self.0.version != 1 {
            return Ok(false);
        }
        let mut at = 0usize;
        let mut equal = true;
        Self::write_envelope(world, members, b, |value| {
            equal &= self.0.params.get(at) == Some(&value);
            at = at
                .checked_add(1)
                .ok_or_else(|| Error::new("component envelope offset overflow"))?;
            Ok(())
        })?;
        Ok(equal && at == self.0.params.len())
    }

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
        Self::write_envelope(world, ordered.iter().copied(), b, |value| {
            words.push(value);
            Ok(())
        })?;
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
