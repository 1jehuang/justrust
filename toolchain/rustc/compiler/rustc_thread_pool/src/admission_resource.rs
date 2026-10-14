//! Neutral immutable attachment transport. Owner registration, not this value,
//! supplies authority. This module has no compiler data-structures dependency.

use std::io::{self, Write};
use std::num::NonZeroU64;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::admission::{AdmissionAttachmentId, QueryRootNamespace};

/// Diagnostic-only registration failure, never an admission-policy decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindFailure {
    InvalidDescriptor,
    WrongOwner,
    NotPrepared,
    InvalidIdentity,
    CaptureMismatch,
    ObserverMismatch,
    QueryCaptureMismatch,
    Conflict,
    IdentityExhausted,
}

#[derive(Debug, Default)]
struct RegistrationState {
    attempts: AtomicU64,
    successes: AtomicU64,
    conflicts: AtomicU64,
    failures: AtomicU64,
    invalid: AtomicBool,
    overflowed: AtomicBool,
}

/// Shared diagnostic state only, without retaining a participant or scheduler.
#[derive(Clone, Debug, Default)]
pub struct EndorsementGate(Arc<RegistrationState>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegistrationSnapshot {
    pub attempts: u64,
    pub successes: u64,
    pub conflicts: u64,
    pub failures: u64,
    pub invalid: bool,
    pub overflowed: bool,
}

impl EndorsementGate {
    fn increment(&self, counter: &AtomicU64) -> Option<u64> {
        let mut previous = counter.load(Ordering::SeqCst);
        loop {
            let Some(next) = previous.checked_add(1) else {
                self.0.overflowed.store(true, Ordering::SeqCst);
                self.0.invalid.store(true, Ordering::SeqCst);
                return None;
            };
            match counter.compare_exchange_weak(previous, next, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return Some(previous),
                Err(actual) => previous = actual,
            }
        }
    }

    /// Exactly one attempt is eligible. Repeated attempts permanently conflict.
    pub fn begin_attempt(&self) -> Result<(), BindFailure> {
        if self.increment(&self.0.attempts) != Some(0) {
            self.increment(&self.0.conflicts);
            self.0.invalid.store(true, Ordering::SeqCst);
            return Err(BindFailure::Conflict);
        }
        Ok(())
    }

    pub fn succeeded(&self) {
        if self.increment(&self.0.successes) != Some(0) {
            self.0.invalid.store(true, Ordering::SeqCst);
        }
    }

    pub fn failed(&self) {
        self.increment(&self.0.failures);
        self.0.invalid.store(true, Ordering::SeqCst);
    }

    pub fn snapshot(&self) -> RegistrationSnapshot {
        RegistrationSnapshot {
            attempts: self.0.attempts.load(Ordering::SeqCst),
            successes: self.0.successes.load(Ordering::SeqCst),
            conflicts: self.0.conflicts.load(Ordering::SeqCst),
            failures: self.0.failures.load(Ordering::SeqCst),
            invalid: self.0.invalid.load(Ordering::SeqCst),
            overflowed: self.0.overflowed.load(Ordering::SeqCst),
        }
    }

    /// The serialization entry observation is the publication cutoff. Later
    /// diagnostic activity never rewrites an already serialized endorsement.
    pub fn is_complete(&self) -> bool {
        let s = self.snapshot();
        s.attempts == 1
            && s.successes == 1
            && s.conflicts == 0
            && s.failures == 0
            && !s.invalid
            && !s.overflowed
    }
}

fn safe_identity(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// Prepared outside all native locks, then shared without cloning strings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedDescriptor {
    attachment_capture_id: Arc<str>,
    attachment_id: AdmissionAttachmentId,
    controller_id: Arc<str>,
    resource_capture_id: Arc<str>,
    query_capture_id: Arc<str>,
    query_root_namespace: QueryRootNamespace,
    pool_capture_id: Arc<str>,
    pool_observer: NonZeroU64,
}

impl PreparedDescriptor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        attachment_capture_id: Arc<str>,
        attachment_id: AdmissionAttachmentId,
        controller_id: Arc<str>,
        resource_capture_id: Arc<str>,
        query_capture_id: Arc<str>,
        query_root_namespace: QueryRootNamespace,
        pool_capture_id: Arc<str>,
        pool_observer: NonZeroU64,
    ) -> Result<Self, BindFailure> {
        let captures =
            [&attachment_capture_id, &resource_capture_id, &query_capture_id, &pool_capture_id];
        if !safe_identity(&controller_id)
            || captures.iter().any(|id| !safe_identity(id))
            || captures.iter().enumerate().any(|(index, id)| captures[..index].contains(id))
            || captures.contains(&&controller_id)
        {
            return Err(BindFailure::InvalidIdentity);
        }
        Ok(Self {
            attachment_capture_id,
            attachment_id,
            controller_id,
            resource_capture_id,
            query_capture_id,
            query_root_namespace,
            pool_capture_id,
            pool_observer,
        })
    }

    pub fn attachment_capture_id(&self) -> &Arc<str> {
        &self.attachment_capture_id
    }
    pub fn attachment_id(&self) -> AdmissionAttachmentId {
        self.attachment_id
    }
    pub fn controller_id(&self) -> &Arc<str> {
        &self.controller_id
    }
    pub fn resource_capture_id(&self) -> &Arc<str> {
        &self.resource_capture_id
    }
    pub fn query_capture_id(&self) -> &Arc<str> {
        &self.query_capture_id
    }
    pub fn query_root_namespace(&self) -> QueryRootNamespace {
        self.query_root_namespace
    }
    pub fn pool_capture_id(&self) -> &Arc<str> {
        &self.pool_capture_id
    }
    pub fn pool_observer(&self) -> NonZeroU64 {
        self.pool_observer
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndorsementRole {
    Query,
    Pool,
    Resource,
}

/// Serialization value only. Each owner must construct this from its own
/// validated immutable registration, never synthesize another owner's role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachmentEndorsement {
    role: EndorsementRole,
    capture_id: Arc<str>,
    descriptor: PreparedDescriptor,
}

impl AttachmentEndorsement {
    pub fn new(
        role: EndorsementRole,
        capture_id: Arc<str>,
        descriptor: PreparedDescriptor,
    ) -> Result<Self, BindFailure> {
        if !safe_identity(&capture_id)
            || [
                &descriptor.attachment_capture_id,
                &descriptor.controller_id,
                &descriptor.resource_capture_id,
                &descriptor.query_capture_id,
                &descriptor.pool_capture_id,
            ]
            .contains(&&capture_id)
        {
            return Err(BindFailure::InvalidIdentity);
        }
        Ok(Self { role, capture_id, descriptor })
    }

    pub fn capture_id(&self) -> &Arc<str> {
        &self.capture_id
    }
    pub fn descriptor(&self) -> &PreparedDescriptor {
        &self.descriptor
    }

    /// All strings are validated safe ASCII, so no escaping or normalization.
    /// This is the exact separate schema1 document, not a pool2 header edit.
    pub fn write_json(&self, out: &mut impl Write) -> io::Result<()> {
        let d = &self.descriptor;
        let role = match self.role {
            EndorsementRole::Query => "query",
            EndorsementRole::Pool => "pool",
            EndorsementRole::Resource => "resource",
        };
        writeln!(
            out,
            "{{\"schema\":1,\"producer\":\"rustc-frontend-admission-attachment\",\"scope\":\"selected-resource-query-pool-attachment\",\"capture_id\":\"{}\",\"role\":\"{role}\",\"sealed\":true,\"expected_endorsements\":1,\"endorsement_allocator_baseline\":0,\"endorsement_allocator_high_water\":1,\"lost_events\":0,\"loss_overflowed\":false,\"attachment\":{{\"attachment_capture_id\":\"{}\",\"attachment_id\":{},\"controller_id\":\"{}\",\"resource_capture_id\":\"{}\",\"query_capture_id\":\"{}\",\"query_root_namespace\":{},\"pool_capture_id\":\"{}\",\"pool_observer\":{}}}}}",
            self.capture_id,
            d.attachment_capture_id,
            d.attachment_id.wire_value(),
            d.controller_id,
            d.resource_capture_id,
            d.query_capture_id,
            d.query_root_namespace.wire_value(),
            d.pool_capture_id,
            d.pool_observer
        )
    }
}

/// Only ExecutionObserver registration can construct this authority token.
#[derive(Clone, Debug)]
pub struct PoolEndorsementToken {
    pub(crate) endorsement: AttachmentEndorsement,
    pub(crate) gate: EndorsementGate,
}

impl PoolEndorsementToken {
    pub fn capture_id(&self) -> &Arc<str> {
        self.endorsement.capture_id()
    }
    pub fn descriptor(&self) -> &PreparedDescriptor {
        self.endorsement.descriptor()
    }
    pub fn write_json(&self, out: &mut impl Write) -> io::Result<()> {
        if !self.gate.is_complete() {
            return Err(io::Error::other("pool resource attachment registration incomplete"));
        }
        self.endorsement.write_json(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(
        captures: [&str; 4],
        controller: Arc<str>,
    ) -> Result<PreparedDescriptor, BindFailure> {
        PreparedDescriptor::new(
            Arc::from(captures[0]),
            AdmissionAttachmentId::mint().unwrap(),
            controller,
            Arc::from(captures[1]),
            Arc::from(captures[2]),
            QueryRootNamespace::mint().unwrap(),
            Arc::from(captures[3]),
            NonZeroU64::new(1).unwrap(),
        )
    }

    #[test]
    fn descriptor_rejects_unsafe_or_aliased_capture_identities() {
        for bad in ["", "a b", "a/b", "a.b", "a\"b", "a\\b", "é", "\n", &"a".repeat(129)] {
            for index in 0..4 {
                let mut captures = ["bridge", "resource", "query", "pool"];
                captures[index] = bad;
                assert_eq!(
                    descriptor(captures, Arc::from("controller")).unwrap_err(),
                    BindFailure::InvalidIdentity
                );
            }
            assert_eq!(
                descriptor(["bridge", "resource", "query", "pool"], Arc::from(bad)).unwrap_err(),
                BindFailure::InvalidIdentity
            );
        }
        for first in 0..4 {
            for second in first + 1..4 {
                let mut captures = ["bridge", "resource", "query", "pool"];
                captures[second] = captures[first];
                assert!(descriptor(captures, Arc::from("controller")).is_err());
            }
        }
        let controller: Arc<str> = Arc::from("Controller_123-safe");
        let d = descriptor(["bridge", "resource", "query", "pool"], controller.clone()).unwrap();
        assert!(Arc::ptr_eq(d.controller_id(), &controller));
        assert_ne!(d.attachment_id().wire_value(), 0);
        assert_ne!(d.query_root_namespace().wire_value(), 0);
        for capture in ["bridge", "resource", "query", "pool", "Controller_123-safe"] {
            assert!(
                AttachmentEndorsement::new(EndorsementRole::Pool, Arc::from(capture), d.clone())
                    .is_err()
            );
        }
    }

    #[test]
    fn gate_failures_conflicts_and_counter_exhaustion_are_sticky() {
        let gate = EndorsementGate::default();
        assert!(!gate.is_complete());
        gate.begin_attempt().unwrap();
        gate.succeeded();
        assert!(gate.is_complete());
        let token_gate = gate.clone();
        assert_eq!(gate.begin_attempt(), Err(BindFailure::Conflict));
        assert!(!token_gate.is_complete());
        assert_eq!(gate.snapshot().attempts, 2);
        assert_eq!(gate.snapshot().conflicts, 1);
        let failed = EndorsementGate::default();
        failed.begin_attempt().unwrap();
        failed.failed();
        failed.succeeded();
        assert!(!failed.is_complete());
        let exhausted = EndorsementGate::default();
        exhausted.0.attempts.store(u64::MAX, Ordering::SeqCst);
        assert_eq!(exhausted.begin_attempt(), Err(BindFailure::Conflict));
        assert_eq!(exhausted.snapshot().attempts, u64::MAX);
        assert!(exhausted.snapshot().overflowed);
        assert!(!exhausted.is_complete());
    }
}
