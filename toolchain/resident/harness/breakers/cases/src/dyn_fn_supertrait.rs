//! A call of a trait object whose trait is not `Fn` or `FnMut` but has one above it.

pub trait Filter: 'static + FnMut(&[u8]) -> Result<bool, String> + Send {}

impl<T> Filter for T where T: 'static + FnMut(&[u8]) -> Result<bool, String> + Send {}

pub trait Rule: Fn(u32) -> u32 {
    fn name(&self) -> &'static str {
        "rule"
    }
}

impl<T: Fn(u32) -> u32> Rule for T {}

struct Pipe {
    filter: Option<Box<dyn Filter>>,
}

impl Pipe {
    fn run(&mut self, buffer: &[u8]) -> Result<bool, String> {
        match &mut self.filter {
            Some(filter) => filter(buffer),
            None => Ok(true),
        }
    }
}

fn apply(rule: &dyn Rule, value: u32) -> (u32, &'static str) {
    (rule(value), rule.name())
}

pub fn run() -> (Vec<Result<bool, String>>, (u32, &'static str), usize) {
    let mut seen = 0;
    let mut pipe = Pipe {
        filter: Some(Box::new(move |bytes: &[u8]| {
            seen += bytes.len();
            if seen > 6 { Err(format!("{seen} bytes")) } else { Ok(bytes.len() % 2 == 0) }
        })),
    };
    let results = vec![pipe.run(b"ab"), pipe.run(b"abc"), pipe.run(b"abcd")];
    let mut none = Pipe { filter: None };
    let by_default = none.run(b"x").is_ok() as usize;
    let offset = 7;
    (results, apply(&move |it| it * 3 + offset, 5), by_default)
}
