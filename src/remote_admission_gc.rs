use std::sync::{Arc, Mutex, OnceLock};

/// One slot per mirror registry. Never retain a free-space reading or reuse a
/// completed collection, even when waiters on that collection haven't exited.
#[derive(Default)]
pub(super) struct AdmissionGc(Mutex<Arc<OnceLock<()>>>);

impl AdmissionGc {
    fn flight(&self) -> Arc<OnceLock<()>> {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if slot.get().is_some() {
            *slot = Arc::default();
        }
        Arc::clone(&slot)
    }

    pub(super) fn below_after(
        &self,
        mut below: impl FnMut() -> Option<u64>,
        collect: impl FnOnce(),
    ) -> Option<u64> {
        below()?;
        self.flight().get_or_init(collect);
        below()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_admissions_share_work_not_disk_readings() {
        let gc = AdmissionGc::default();
        let running = gc.flight();
        std::thread::scope(|s| {
            let (entered, wait) = std::sync::mpsc::channel();
            let (release, hold) = std::sync::mpsc::channel();
            s.spawn(|| {
                running.get_or_init(move || {
                    entered.send(()).unwrap();
                    hold.recv().unwrap();
                });
            });
            wait.recv().unwrap();
            let joined = gc.flight();
            assert!(Arc::ptr_eq(&running, &joined));
            release.send(()).unwrap();
            joined.get_or_init(|| panic!("redundant scan"));
            assert!(!Arc::ptr_eq(&running, &gc.flight()));
        });
        for result in [None, Some(42)] {
            let mut readings = [Some(1), result].into_iter();
            assert_eq!(gc.below_after(|| readings.next().unwrap(), || {}), result);
            assert_eq!(readings.len(), 0);
        }
        assert_eq!(gc.below_after(|| None, || panic!("healthy disk")), None);
    }
}
