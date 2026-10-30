//! Explicit, best-effort worker placement. This never changes the spawning thread.

const MAX_CPUS: usize = 8192;
const WORD_BITS: usize = usize::BITS as usize;
const MAX_WORDS: usize = MAX_CPUS / WORD_BITS;
const MAX_LIST_BYTES: usize = 65536;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Affinity {
    words: Vec<usize>,
}

impl Affinity {
    fn empty() -> Self {
        Self {
            words: vec![0; MAX_WORDS],
        }
    }

    fn contains(&self, cpu: usize) -> bool {
        cpu < MAX_CPUS && self.words[cpu / WORD_BITS] & (1usize << (cpu % WORD_BITS)) != 0
    }

    fn insert(&mut self, cpu: usize) {
        self.words[cpu / WORD_BITS] |= 1usize << (cpu % WORD_BITS);
    }

    fn count(&self) -> usize {
        self.words
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum()
    }

    fn intersection(&self, other: &Self) -> Self {
        Self {
            words: self
                .words
                .iter()
                .zip(&other.words)
                .map(|(a, b)| a & b)
                .collect(),
        }
    }

    pub(super) fn cpus(&self) -> Vec<usize> {
        (0..MAX_CPUS).filter(|&cpu| self.contains(cpu)).collect()
    }

    pub(super) fn apply(&self) -> bool {
        #[cfg(target_os = "linux")]
        if let Some(current) = linux::inherited() {
            let restricted = self.intersection(&current);
            if restricted == *self && restricted.count() != 0 {
                // SAFETY: the buffer is initialized, correctly aligned and its byte size is exact.
                // pid zero targets only the calling thread, never the parent or other workers.
                let result = unsafe {
                    linux::sched_setaffinity(
                        0,
                        std::mem::size_of_val(restricted.words.as_slice()),
                        restricted.words.as_ptr().cast(),
                    )
                };
                return result == 0
                    && restricted == *self
                    && linux::inherited().as_ref() == Some(self);
            }
        }
        false
    }
}

fn enabled(value: Option<&str>) -> bool {
    value == Some("llc")
}

pub(super) fn selected(workers: usize) -> Option<Affinity> {
    if !enabled(std::env::var("JR_GRAPH_LOCALITY").ok().as_deref()) || workers == 0 {
        return None;
    }
    #[cfg(target_os = "linux")]
    {
        linux::discover(workers)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

fn parse_list(text: &str) -> Option<Affinity> {
    if text.len() > MAX_LIST_BYTES {
        return None;
    }
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut mask = Affinity::empty();
    let mut previous = None;
    for part in text.split(',') {
        let mut ends = part.split('-');
        let number = |s: &str| -> Option<usize> {
            if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let n = s.parse::<usize>().ok()?;
            (n < MAX_CPUS).then_some(n)
        };
        let first = number(ends.next()?)?;
        let last = match ends.next() {
            Some(s) => number(s)?,
            None => first,
        };
        if ends.next().is_some() || first > last {
            return None;
        }
        // Reject overlapping or unordered lists, avoiding repeated range expansion.
        if previous.is_some_and(|end| first <= end) {
            return None;
        }
        previous = Some(last);
        for cpu in first..=last {
            mask.insert(cpu);
        }
    }
    Some(mask)
}

fn record_cache(
    highest: &mut Option<(usize, Affinity)>,
    cpu: usize,
    level: usize,
    domain: Affinity,
) -> Option<()> {
    if level == 0 || level > 256 || !domain.contains(cpu) {
        return None;
    }
    match highest {
        Some((old, mask)) if *old == level && *mask != domain => return None,
        Some((old, _)) if *old >= level => {}
        _ => *highest = Some((level, domain)),
    }
    Some(())
}

fn choose(
    inherited: &Affinity,
    domains: impl IntoIterator<Item = Affinity>,
    workers: usize,
) -> Option<Affinity> {
    if workers == 0 {
        return None;
    }
    domains
        .into_iter()
        .map(|domain| domain.intersection(inherited))
        .filter(|domain| domain.count() >= workers)
        .min_by(|a, b| {
            a.count().cmp(&b.count()).then_with(|| {
                (0..MAX_CPUS)
                    .filter(|&cpu| a.contains(cpu))
                    .cmp((0..MAX_CPUS).filter(|&cpu| b.contains(cpu)))
            })
        })
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::io::Read;
    use std::path::Path;

    unsafe extern "C" {
        fn sched_getaffinity(pid: i32, size: usize, mask: *mut std::ffi::c_void) -> i32;
        pub(super) fn sched_setaffinity(
            pid: i32,
            size: usize,
            mask: *const std::ffi::c_void,
        ) -> i32;
    }

    pub(super) fn inherited() -> Option<Affinity> {
        let mut words = 1;
        loop {
            let mut buffer = vec![0usize; words];
            // SAFETY: writable, initialized storage with an accurate byte length.
            let result = unsafe {
                sched_getaffinity(
                    0,
                    std::mem::size_of_val(buffer.as_slice()),
                    buffer.as_mut_ptr().cast(),
                )
            };
            if result == 0 {
                buffer.resize(MAX_WORDS, 0);
                let mask = Affinity { words: buffer };
                return (mask.count() != 0).then_some(mask);
            }
            if std::io::Error::last_os_error().raw_os_error() != Some(22) || words == MAX_WORDS {
                return None;
            }
            words = (words * 2).min(MAX_WORDS);
        }
    }

    fn read(path: &Path) -> Option<String> {
        let file = std::fs::File::open(path).ok()?;
        let mut bytes = Vec::new();
        file.take((MAX_LIST_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() > MAX_LIST_BYTES {
            return None;
        }
        String::from_utf8(bytes).ok()
    }

    pub(super) fn discover(workers: usize) -> Option<Affinity> {
        let allowed = inherited()?;
        let mut domains = Vec::new();
        for cpu in (0..MAX_CPUS).filter(|&cpu| allowed.contains(cpu)) {
            let path = format!("/sys/devices/system/cpu/cpu{cpu}/cache");
            let mut highest: Option<(usize, Affinity)> = None;
            for (index, entry) in std::fs::read_dir(path).ok()?.enumerate() {
                if index >= 256 {
                    return None;
                }
                let entry = entry.ok()?;
                let name = entry.file_name();
                let name = name.to_str()?;
                if !name.starts_with("index") {
                    continue;
                }
                let path = entry.path();
                let kind = read(&path.join("type"))?;
                if !matches!(kind.trim(), "Data" | "Unified") {
                    continue;
                }
                let level = read(&path.join("level"))?.trim().parse::<usize>().ok()?;
                let domain = parse_list(&read(&path.join("shared_cpu_list"))?)?;
                record_cache(&mut highest, cpu, level, domain)?;
            }
            let (_, domain) = highest?;
            if !domains.contains(&domain) {
                domains.push(domain);
            }
        }
        choose(&allowed, domains, workers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mask(s: &str) -> Affinity {
        parse_list(s).unwrap()
    }

    #[test]
    fn exact_opt_in() {
        for value in [None, Some(""), Some("LLC"), Some("llc "), Some("auto")] {
            assert!(!enabled(value));
        }
        assert!(enabled(Some("llc")));
    }

    #[test]
    fn sparse_and_ranges() {
        let parsed = mask("0,2-4,8191\n");
        assert_eq!(parsed.count(), 5);
        assert!(parsed.contains(8191));
        assert!(!parsed.contains(1));
    }

    #[test]
    fn invalid_overflow_and_bounds() {
        for input in [
            "",
            " ",
            "1,",
            ",1",
            "-1",
            "1-",
            "2-1",
            "1--2",
            "1-2-3",
            "1,1",
            "1-3,2",
            "1, 2",
            "8192",
            "0-8192",
            "999999999999999999999999999999999999",
        ] {
            assert!(parse_list(input).is_none(), "{input:?}");
        }
        assert!(parse_list(&"0".repeat(MAX_LIST_BYTES + 1)).is_none());
        assert_eq!(mask("0-8191").count(), MAX_CPUS);
    }

    #[test]
    fn highest_cache_and_malformed_topology() {
        let mut highest = None;
        assert!(record_cache(&mut highest, 2, 1, mask("2-3")).is_some());
        assert!(record_cache(&mut highest, 2, 3, mask("0-7")).is_some());
        assert!(record_cache(&mut highest, 2, 2, mask("2-3")).is_some());
        assert_eq!(highest, Some((3, mask("0-7"))));
        assert!(record_cache(&mut highest, 2, 3, mask("2-3")).is_none());
        assert!(record_cache(&mut highest, 2, 4, mask("8-9")).is_none());
        assert!(record_cache(&mut highest, 2, 0, mask("2-3")).is_none());
        assert!(record_cache(&mut highest, 2, 257, mask("2-3")).is_none());
    }

    #[test]
    fn inherited_restrictions_and_insufficient_domains() {
        let allowed = mask("2-3,8");
        assert_eq!(
            choose(&allowed, [mask("0-3"), mask("8-15")], 2),
            Some(mask("2-3"))
        );
        assert_eq!(choose(&allowed, [mask("0-3"), mask("8-15")], 3), None);
        assert_eq!(choose(&allowed, [mask("0-15")], 0), None);
    }

    #[test]
    fn smallest_and_deterministic_tie() {
        let allowed = mask("0-15");
        let domains = [mask("8-9"), mask("0-7"), mask("2-3")];
        assert_eq!(choose(&allowed, domains.clone(), 2), Some(mask("2-3")));
        assert_eq!(
            choose(&allowed, domains.into_iter().rev(), 2),
            Some(mask("2-3"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn apply_only_changes_fresh_worker_and_reintersects() {
        let Some(parent) = linux::inherited() else {
            return;
        };
        let expected_parent = parent.clone();
        std::thread::spawn(move || {
            let Some(before) = linux::inherited() else {
                return;
            };
            let Some(cpu) = (0..MAX_CPUS).find(|&cpu| before.contains(cpu)) else {
                return;
            };
            let mut single = Affinity::empty();
            single.insert(cpu);
            single.apply();
            let Some(after) = linux::inherited() else {
                return;
            };
            // Affinity is best effort, including environments denying the syscall.
            assert!(after == before || after == single);
            mask("0-8191").apply();
            if let Some(final_mask) = linux::inherited() {
                assert_eq!(final_mask, after);
            }
        })
        .join()
        .unwrap();
        assert_eq!(linux::inherited(), Some(expected_parent));
    }
}
