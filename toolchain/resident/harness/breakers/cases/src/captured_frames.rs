//! The space game's sound died a second after it started (blaze bug D): its audio callback
//! captures a synthesizer of about 240 KB by value, and cpal wraps it in two more closures,
//! each calling the one inside through `&mut`. rust-analyzer's MIR gives a closure's body a
//! local for each capture and for each binding of the function the closure is written in;
//! in a closure called by reference nothing reads or writes them, but ra-jit gave each a
//! slot, so each of the two frames had room for the synthesizer twice. CoreAudio calls the
//! callback on a thread of 512 KB, and it overflowed. rustc's frames here are a few hundred
//! bytes; a local nothing mentions now has no slot.

pub struct Synth {
    table: [u32; 64 * 1024],
    phase: usize,
}

impl Synth {
    fn fill(&mut self, out: &mut [u32], tick: &u64) {
        for slot in out {
            *slot = self.table[self.phase].wrapping_add(*tick as u32);
            self.phase = (self.phase + 7) % self.table.len();
        }
    }
}

/// cpal's `monotonic_output_callback`: the time it hands on never goes back.
fn monotonic<D>(mut data_callback: D) -> impl FnMut(&mut [u32], &u64) + Send + 'static
where
    D: FnMut(&mut [u32], &u64) + Send + 'static,
{
    let mut floor = 0u64;
    move |data, tick| {
        let tick = (*tick).max(floor);
        floor = tick;
        data_callback(data, &tick);
    }
}

/// cpal's `build_output_stream_raw`: the render callback the audio thread is given.
fn stream<D>(data_callback: D) -> Box<dyn FnMut(&mut [u32]) + Send>
where
    D: FnMut(&mut [u32], &u64) + Send + 'static,
{
    let mut data_callback = monotonic(data_callback);
    let mut ticks = 0u64;
    Box::new(move |out| {
        ticks += 1;
        data_callback(out, &ticks);
    })
}

#[inline(never)]
fn synth() -> Box<Synth> {
    let mut synth = Box::new(Synth { table: [0; 64 * 1024], phase: 0 });
    for (at, slot) in synth.table.iter_mut().enumerate() {
        *slot = (at as u32).wrapping_mul(2_654_435_761);
    }
    synth
}

/// The callback is made where the stack is large, as the game's is on its main thread, and
/// run on a thread of CoreAudio's 512 KB.
pub fn run() -> u32 {
    let made = std::thread::Builder::new().stack_size(16 << 20).spawn(|| {
        let mut synth = *synth();
        stream(move |out: &mut [u32], tick: &u64| synth.fill(out, tick))
    });
    let mut render = made.expect("a thread").join().expect("the callback is made");
    let audio = std::thread::Builder::new().stack_size(512 << 10).spawn(move || {
        let mut out = [0u32; 64];
        let mut sum = 0u32;
        for _ in 0..100 {
            render(&mut out);
            sum = out.iter().fold(sum, |sum, it| sum.rotate_left(5) ^ it);
        }
        sum
    });
    audio.expect("a thread").join().expect("the callback fits the audio thread's stack")
}
