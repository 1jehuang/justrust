//! Local-only incremental output journal. Recording errors never escape to cargo.
//!
//! The caller serializes append/flush with its output capture lock and flushes
//! on a periodic sampling tick and at completion. No network or telemetry path
//! exists here. A hard kill can lose the last tick's buffered bytes, and flush
//! does not promise power-loss durability (there is deliberately no fsync).
use crate::record::OutputLine;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

const BUFFER_BYTES: usize = 64 * 1024;

pub struct Journal<W: Write = File> {
    writer: Option<BufWriter<W>>,
}

impl Journal<File> {
    pub fn new(path: &Path) -> Self {
        Self {
            writer: File::create(path)
                .ok()
                .map(|file| BufWriter::with_capacity(BUFFER_BYTES, file)),
        }
    }
}

impl<W: Write> Journal<W> {
    /// Streaming serialization avoids allocating a second copy of large lines.
    /// After any failure the journal is disabled, never retried or duplicated.
    pub fn append(&mut self, line: &OutputLine) {
        let failed = self.writer.as_mut().is_some_and(|writer| {
            serde_json::to_writer(&mut *writer, line).is_err() || writer.write_all(b"\n").is_err()
        });
        if failed {
            self.disable();
        }
    }

    /// True only if all appended lines have been written successfully.
    pub fn flush(&mut self) -> bool {
        let success = self
            .writer
            .as_mut()
            .is_some_and(|writer| writer.flush().is_ok());
        if !success {
            self.disable();
        }
        success
    }

    fn disable(&mut self) {
        if let Some(writer) = self.writer.take() {
            // Do not let BufWriter::drop retry a partially failed write.
            let _ = writer.into_parts();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Sink {
        bytes: Arc<Mutex<Vec<u8>>>,
        writes: Arc<Mutex<usize>>,
        fail: bool,
    }

    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            *self.writes.lock().unwrap() += 1;
            if self.fail {
                return Err(io::Error::other("disk full"));
            }
            self.bytes.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn journal(sink: Sink) -> Journal<Sink> {
        Journal {
            writer: Some(BufWriter::with_capacity(BUFFER_BYTES, sink)),
        }
    }

    fn line(text: &str) -> OutputLine {
        OutputLine {
            t: 123.25,
            s: "e".into(),
            l: text.into(),
        }
    }

    #[test]
    fn ticks_preserve_exact_jsonl_without_duplicate_completion() {
        let sink = Sink::default();
        let mut journal = journal(sink.clone());
        let lines = [line("quote \" newline\n unicode 🦀"), line("second")];
        journal.append(&lines[0]);
        assert!(sink.bytes.lock().unwrap().is_empty());
        assert!(journal.flush());
        journal.append(&lines[1]);
        assert!(journal.flush());
        assert!(journal.flush());
        let expected: Vec<u8> = lines
            .iter()
            .flat_map(|line| {
                let mut bytes = serde_json::to_vec(line).unwrap();
                bytes.push(b'\n');
                bytes
            })
            .collect();
        assert_eq!(*sink.bytes.lock().unwrap(), expected);
        assert_eq!(*sink.writes.lock().unwrap(), 2);
    }

    #[test]
    fn large_lines_and_many_lines_keep_bounded_buffer_and_batch_writes() {
        let sink = Sink::default();
        let mut journal = journal(sink.clone());
        for _ in 0..10_000 {
            journal.append(&line("representative compiler diagnostic"));
            assert!(journal.writer.as_ref().unwrap().buffer().len() <= BUFFER_BYTES);
        }
        journal.append(&line(&"x".repeat(BUFFER_BYTES * 3)));
        assert!(journal.flush());
        assert!(*sink.writes.lock().unwrap() < 100);
        let bytes = sink.bytes.lock().unwrap();
        assert_eq!(bytes.iter().filter(|&&b| b == b'\n').count(), 10_001);
        for row in bytes.split(|&b| b == b'\n').filter(|row| !row.is_empty()) {
            assert!(serde_json::from_slice::<OutputLine>(row).is_ok());
        }
    }

    #[test]
    fn write_failure_disables_recording_without_retry_or_panic() {
        let sink = Sink {
            fail: true,
            ..Sink::default()
        };
        let mut journal = journal(sink.clone());
        journal.append(&line("visible on terminal regardless of disk"));
        assert!(!journal.flush());
        journal.append(&line("after failure"));
        assert!(!journal.flush());
        drop(journal);
        assert_eq!(*sink.writes.lock().unwrap(), 1);
    }

    #[test]
    fn unavailable_path_is_fail_open() {
        let mut journal = Journal::new(Path::new("/dev/null/output.jsonl"));
        journal.append(&line("still run cargo"));
        assert!(!journal.flush());
    }
}
