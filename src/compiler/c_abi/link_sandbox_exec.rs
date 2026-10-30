//! Same-thread ownership of the initial ptrace exec stop.
use std::{
    io,
    marker::PhantomData,
    process::Child,
    rc::Rc,
    time::{Duration, Instant},
};

#[path = "link_sandbox_launch.rs"]
mod launch;

unsafe extern "C" {
    fn ptrace(request: u32, pid: i32, address: usize, data: usize) -> i64;
    fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
    fn kill(pid: i32, signal: i32) -> i32;
}

/// Never Send: Linux ptrace ownership belongs to the spawning thread, not
/// merely its process. Until detach, do not use Rust Child wait methods:
/// they can cache a ptrace stop as if it were a terminal exit status.
pub(super) struct Guard {
    pub(super) child: Child,
    traced: bool,
    stopped: bool,
    reaped: bool,
    owner: PhantomData<Rc<()>>,
}
impl Guard {
    pub(super) fn new(child: Child) -> Self {
        Self {
            child,
            traced: true,
            stopped: false,
            reaped: false,
            owner: PhantomData,
        }
    }

    pub(super) fn await_exec(&mut self) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut status = 0;
            // WNOHANG: a missing exec trap must never hang the compiler.
            let result = unsafe { waitpid(self.child.id() as i32, &mut status, 1) };
            if result < 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            } else if result > 0 {
                if status & 0xff == 0x7f {
                    if status == (5 << 8) | 0x7f {
                        self.stopped = true;
                        return Ok(());
                    }
                    return Err(io::Error::other(format!(
                        "unexpected linker exec stop: {status:#x}"
                    )));
                }
                self.reaped = true;
                return Err(io::Error::other(format!(
                    "linker exited before exec stop: {status:#x}"
                )));
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "linker exec stop exceeded 5 seconds",
                ));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub(super) fn normalize_launch(&self, procdir: &std::fs::File) -> io::Result<()> {
        if !self.traced || !self.stopped || self.reaped {
            return Err(io::Error::other(
                "linker launch normalization requires owned exec stop",
            ));
        }
        #[repr(C)]
        struct IoVec {
            base: *mut u64,
            length: usize,
        }
        // NT_PRSTATUS native x86-64 user_regs_struct: 27 u64 words,
        // including RSP at word19 and CS at word17. Reject a compat regset
        // or a changed kernel shape rather than guessing a stack pointer.
        let mut registers = [0u64; 27];
        let mut vector = IoVec {
            base: registers.as_mut_ptr(),
            length: std::mem::size_of_val(&registers),
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            vector.length = std::mem::size_of_val(&registers);
            if unsafe {
                ptrace(
                    0x4204,
                    self.child.id() as i32,
                    1,
                    (&mut vector as *mut IoVec) as usize,
                )
            } != -1
            {
                break;
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "linker initial register read exceeded 5 seconds",
                ));
            }
        }
        if vector.length != std::mem::size_of_val(&registers) || registers[17] != 0x33 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "unsupported linker initial native64 register shape",
            ));
        }
        launch::normalize(procdir, registers[19])
    }

    pub(super) fn detach(&mut self) -> io::Result<()> {
        if !self.traced || !self.stopped || self.reaped {
            return Err(io::Error::other("linker detach requires owned exec stop"));
        }
        // Signal zero suppresses the synthetic SIGTRAP. No image instruction
        // executes until every parent launch-state operation has succeeded.
        if unsafe { ptrace(17, self.child.id() as i32, 0, 0) } == -1 {
            return Err(io::Error::last_os_error());
        }
        self.traced = false;
        self.stopped = false;
        Ok(())
    }

    pub(super) fn terminate(&mut self) {
        if self.reaped {
            return;
        }
        if !self.traced {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                self.reaped = true;
                return;
            }
            let _ = self.child.kill();
            if self.child.wait().is_ok() {
                self.reaped = true;
            }
            return;
        }
        // PID remains pinned by our unreaped child. SIGKILL kills even a
        // ptrace-stopped task. Consume any pending stop before terminal reap.
        unsafe {
            kill(self.child.id() as i32, 9);
        }
        loop {
            let mut status = 0;
            let result = unsafe { waitpid(self.child.id() as i32, &mut status, 0) };
            if result < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                break;
            }
            if status & 0xff == 0x7f {
                unsafe {
                    ptrace(7, self.child.id() as i32, 0, 9);
                }
                continue;
            }
            self.reaped = true;
            break;
        }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.terminate();
    }
}
