//! Shared logical reservation telemetry. Admission policy belongs to each budget.

/// Logical budget reservations, not allocator instrumentation or process RSS.
/// Bytes include conservative estimates and retained storage. Telemetry counters
/// saturate independently of admission arithmetic and never change admission.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReservationAccounting {
    pub reservations: usize,
    pub current_reservations: usize,
    pub peak_reservations: usize,
    pub retentions: usize,
    pub current_bytes: usize,
    pub peak_bytes: usize,
    pub cumulative_bytes: usize,
    pub counters_saturated: bool,
}

impl ReservationAccounting {
    fn add(counter: &mut usize, amount: usize, saturated: &mut bool) {
        if let Some(next) = counter.checked_add(amount) {
            *counter = next;
        } else {
            *counter = usize::MAX;
            *saturated = true;
        }
    }
    pub(super) fn record(&mut self, bytes: usize, retained: bool) {
        if retained {
            Self::add(&mut self.retentions, 1, &mut self.counters_saturated);
        } else {
            Self::add(&mut self.reservations, 1, &mut self.counters_saturated);
            Self::add(
                &mut self.current_reservations,
                1,
                &mut self.counters_saturated,
            );
            self.peak_reservations = self.peak_reservations.max(self.current_reservations);
        }
        Self::add(&mut self.current_bytes, bytes, &mut self.counters_saturated);
        Self::add(
            &mut self.cumulative_bytes,
            bytes,
            &mut self.counters_saturated,
        );
        self.peak_bytes = self.peak_bytes.max(self.current_bytes);
    }
    pub(super) fn release(&mut self, bytes: usize) {
        self.current_reservations = self.current_reservations.saturating_sub(1);
        self.current_bytes = self.current_bytes.saturating_sub(bytes);
    }
    /// Update one existing logical reservation without creating a second lease.
    pub(super) fn resize_bytes(&mut self, old: usize, new: usize) {
        if new >= old {
            let additional = new - old;
            Self::add(
                &mut self.current_bytes,
                additional,
                &mut self.counters_saturated,
            );
            Self::add(
                &mut self.cumulative_bytes,
                additional,
                &mut self.counters_saturated,
            );
            self.peak_bytes = self.peak_bytes.max(self.current_bytes);
        } else {
            self.current_bytes = self.current_bytes.saturating_sub(old - new);
        }
    }
    /// Mark an existing lease as retained without charging its bytes twice.
    pub(super) fn record_retention(&mut self) {
        Self::add(&mut self.retentions, 1, &mut self.counters_saturated);
    }
    pub fn write_json(&self, out: &mut impl std::fmt::Write) -> std::fmt::Result {
        write!(
            out,
            "{{\"reservations\":{},\"current_reservations\":{},\"peak_reservations\":{},\"retentions\":{},\"current_bytes\":{},\"peak_bytes\":{},\"cumulative_bytes\":{},\"counters_saturated\":{}}}",
            self.reservations,
            self.current_reservations,
            self.peak_reservations,
            self.retentions,
            self.current_bytes,
            self.peak_bytes,
            self.cumulative_bytes,
            self.counters_saturated
        )
    }
}

#[cfg(test)]
mod tests {
    use super::ReservationAccounting;

    #[test]
    fn resize_and_promotion_do_not_double_count_an_owned_lease() {
        let mut accounting = ReservationAccounting::default();
        accounting.record(8, false);
        accounting.resize_bytes(8, 24);
        accounting.record_retention();
        assert_eq!(accounting.reservations, 1);
        assert_eq!(accounting.current_reservations, 1);
        assert_eq!(accounting.retentions, 1);
        assert_eq!(accounting.current_bytes, 24);
        assert_eq!(accounting.peak_bytes, 24);
        assert_eq!(accounting.cumulative_bytes, 24);
        accounting.resize_bytes(24, 3);
        assert_eq!(accounting.current_bytes, 3);
        assert_eq!(accounting.peak_bytes, 24);
        assert_eq!(accounting.cumulative_bytes, 24);
        accounting.release(3);
        assert_eq!(accounting.current_reservations, 0);
        assert_eq!(accounting.current_bytes, 0);
        assert!(!accounting.counters_saturated);
    }

    #[test]
    fn telemetry_saturation_remains_separate_from_admission() {
        let mut accounting = ReservationAccounting {
            cumulative_bytes: usize::MAX,
            retentions: usize::MAX,
            ..ReservationAccounting::default()
        };
        accounting.record(0, false);
        accounting.resize_bytes(0, 1);
        accounting.record_retention();
        assert!(accounting.counters_saturated);
        assert_eq!(accounting.cumulative_bytes, usize::MAX);
        assert_eq!(accounting.retentions, usize::MAX);
        assert_eq!(accounting.current_bytes, 1);
        accounting.release(1);
        assert_eq!(accounting.current_bytes, 0);
        assert_eq!(accounting.current_reservations, 0);
    }
}
