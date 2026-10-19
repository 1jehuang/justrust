//! SysV AMD64 classification for the supported INTEGER/SSE subset.
//! No x87, vectors, variadics, or C++ invisible-reference arguments.
use super::layout::{Layout, Scalar, align_up};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    None,
    Integer,
    Sse,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Location {
    Integer(u8),
    Sse(u8),
    Stack(usize),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transport {
    Ignore,
    Registers(Vec<(usize, Location)>),
    Memory { offset: usize },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallPlan {
    pub arguments: Vec<Transport>,
    pub result: Transport,
    pub stack_bytes: usize,
    pub integer_registers: u8,
    pub sse_registers: u8,
    pub sret: bool,
}
pub fn classes(layout: &Layout) -> Result<Option<Vec<Class>>, String> {
    if layout.size == 0 {
        return Ok(Some(Vec::new()));
    }
    if layout.size > 16 {
        return Ok(None);
    }
    let mut classes = vec![Class::None; layout.size.div_ceil(8)];
    for leaf in &layout.leaves {
        let end = leaf
            .c_offset
            .checked_add(leaf.bytes)
            .ok_or("invalid C layout leaf")?;
        if end > layout.size || leaf.bytes == 0 || leaf.bytes > 8 {
            return Err("invalid C layout leaf".into());
        }
        if leaf.c_offset % leaf.bytes != 0 {
            return Ok(None);
        }
        let class = if leaf.scalar == Scalar::Float {
            Class::Sse
        } else {
            Class::Integer
        };
        for slot in &mut classes[leaf.c_offset / 8..=(end - 1) / 8] {
            *slot = match (*slot, class) {
                (Class::Integer, _) | (_, Class::Integer) => Class::Integer,
                (Class::None, x) => x,
                (x, _) => x,
            };
        }
    }
    Ok(Some(classes))
}
pub fn plan(params: &[Layout], result: &Layout) -> Result<CallPlan, String> {
    if params.len() > 4096 {
        return Err("resource limit: C ABI parameter count exceeds 4096".into());
    }
    let result_classes = classes(result)?;
    let sret = result_classes.is_none();
    let result = match result_classes {
        None => Transport::Memory { offset: 0 },
        Some(c) if c.is_empty() => Transport::Ignore,
        Some(c) => {
            let (mut i, mut s) = (0, 0);
            Transport::Registers(
                c.into_iter()
                    .enumerate()
                    .filter_map(|(n, c)| match c {
                        Class::Integer => {
                            let r = i;
                            i += 1;
                            Some((n * 8, Location::Integer(r)))
                        }
                        Class::Sse => {
                            let r = s;
                            s += 1;
                            Some((n * 8, Location::Sse(r)))
                        }
                        Class::None => None,
                    })
                    .collect(),
            )
        }
    };
    let (mut integer, mut sse, mut stack) = (u8::from(sret), 0u8, 0usize);
    let mut arguments = Vec::with_capacity(params.len());
    for layout in params {
        let c = classes(layout)?;
        if layout.size == 0 {
            arguments.push(Transport::Ignore);
            continue;
        }
        let needed = c.as_ref().map(|c| {
            (
                c.iter().filter(|x| **x == Class::Integer).count() as u8,
                c.iter().filter(|x| **x == Class::Sse).count() as u8,
            )
        });
        if let (Some(c), Some((ni, ns))) = (c, needed)
            && integer + ni <= 6
            && sse + ns <= 8
        {
            let mut locations = Vec::new();
            for (n, c) in c.into_iter().enumerate() {
                match c {
                    Class::Integer => {
                        locations.push((n * 8, Location::Integer(integer)));
                        integer += 1;
                    }
                    Class::Sse => {
                        locations.push((n * 8, Location::Sse(sse)));
                        sse += 1;
                    }
                    Class::None => {}
                }
            }
            arguments.push(Transport::Registers(locations));
        } else {
            // Roll back the ENTIRE aggregate, not only the eightbyte that
            // failed register allocation. Later scalar arguments can use the
            // unconsumed registers.
            stack = align_up(stack, layout.align.max(8))?;
            arguments.push(Transport::Memory { offset: stack });
            stack = align_up(stack.checked_add(layout.size).ok_or("C stack overflow")?, 8)?;
        }
    }
    Ok(CallPlan {
        arguments,
        result,
        stack_bytes: align_up(stack, 16)?,
        integer_registers: integer,
        sse_registers: sse,
        sret,
    })
}
