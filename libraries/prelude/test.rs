// Native libtest replacement: the compiler synthesizes a `main` under
// `cfg(test)` that drives this harness. Every test runs in a forked child
// whose stdout/stderr are captured through a pipe, so a panic (abort) only
// fails that test. Output follows stock libtest's sequential format.
use super::mem::{raw_alloc, raw_free};
use super::string::{String, str_find};

pub struct Harness {
    passed: usize,
    failed: usize,
    ignored: usize,
    failures: String,
    names: String,
    pid: usize,
    read_fd: usize,
}

fn sys(n: usize, a: usize, b: usize, c: usize) -> usize {
    unsafe { __justrust_linux_syscall6!(n, a, b, c, 0usize, 0usize, 0usize) }
}

impl Harness {
    pub fn start(count: usize) -> Harness {
        println!("");
        if count == 1usize {
            println!("running 1 test");
        } else {
            println!("running {} tests", count);
        }
        Harness {
            passed: 0usize,
            failed: 0usize,
            ignored: 0usize,
            failures: String::new(),
            names: String::new(),
            pid: 0usize,
            read_fd: 0usize,
        }
    }

    // Returns true in the child process, which must run the test and exit.
    pub fn begin(&mut self, name: &str, ignored: bool) -> bool {
        if ignored {
            println!("test {} ... ignored", name);
            self.ignored += 1usize;
            return false;
        }
        let fds = raw_alloc(4096usize);
        let _ = sys(22usize, fds, 0usize, 0usize);
        let word = fds as *const u64;
        let pair = unsafe { *word };
        raw_free(fds, 4096usize);
        let read_fd = (pair & 4294967295u64) as usize;
        let write_fd = (pair >> 32u64) as usize;
        let pid = sys(57usize, 0usize, 0usize, 0usize);
        if pid == 0usize {
            let _ = sys(33usize, write_fd, 1usize, 0usize);
            let _ = sys(33usize, write_fd, 2usize, 0usize);
            let _ = sys(3usize, read_fd, 0usize, 0usize);
            let _ = sys(3usize, write_fd, 0usize, 0usize);
            return true;
        }
        let _ = sys(3usize, write_fd, 0usize, 0usize);
        self.pid = pid;
        self.read_fd = read_fd;
        false
    }

    // mode: 0 = must pass, 1 = should_panic, 2 = should_panic(expected).
    pub fn end(&mut self, name: &str, mode: usize, expected: &str) {
        let mut output = String::new();
        let buffer = raw_alloc(4096usize);
        loop {
            let n = sys(0usize, self.read_fd, buffer, 4096usize);
            if n == 0usize || n > 4096usize {
                break;
            }
            let mut i = 0usize;
            while i < n {
                let p = (buffer + i) as *const u8;
                output.push_byte(unsafe { *p });
                i += 1usize;
            }
        }
        let _ = sys(3usize, self.read_fd, 0usize, 0usize);
        let status_cell = buffer;
        let _ = sys(61usize, self.pid, status_cell, 0usize);
        let status_pointer = status_cell as *const u32;
        let status = unsafe { *status_pointer };
        raw_free(buffer, 4096usize);
        let clean = status == 0u32;
        let ok = if mode == 0usize {
            clean
        } else if mode == 1usize {
            !clean
        } else {
            let found = str_find(output.as_str(), expected);
            !clean && found.is_some()
        };
        if mode == 0usize {
            print!("test {} ... ", name);
        } else {
            print!("test {} - should panic ... ", name);
        }
        if ok {
            println!("ok");
            self.passed += 1usize;
        } else {
            println!("FAILED");
            self.failed += 1usize;
            self.failures.push_str("---- ");
            self.failures.push_str(name);
            self.failures.push_str(" stdout ----\n");
            self.failures.push_str(output.as_str());
            if mode != 0usize && clean {
                self.failures.push_str("note: test did not panic as expected\n");
            }
            self.failures.push_str("\n");
            self.names.push_str("    ");
            self.names.push_str(name);
            self.names.push_str("\n");
        }
    }

    pub fn finish(&mut self) {
        if self.failed > 0usize {
            println!("");
            println!("failures:");
            println!("");
            print!("{}", self.failures.as_str());
            println!("");
            println!("failures:");
            print!("{}", self.names.as_str());
            println!("");
            print!("test result: FAILED. ");
        } else {
            println!("");
            print!("test result: ok. ");
        }
        println!(
            "{} passed; {} failed; {} ignored; 0 measured; 0 filtered out; finished in 0.00s",
            self.passed, self.failed, self.ignored
        );
        println!("");
        if self.failed > 0usize {
            super::process::exit(101);
        }
    }
}
