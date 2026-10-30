//! tracing-subscriber makes two spans and an event in one closure. Each expands to a block
//! with a `static __CALLSITE` whose initializer has a `static META` of its own in it, and
//! the three of one name were taken for one another.

struct Site {
    meta: &'static Meta,
}

struct Meta {
    name: &'static str,
    line: u32,
    site: &'static Site,
}

macro_rules! site {
    ($name:expr) => {{
        static SITE: Site = {
            static META: Meta = Meta { name: $name, line: line!(), site: &SITE };
            Site { meta: &META }
        };
        &SITE
    }};
}

fn sites<T: Default + std::fmt::Debug>() -> String {
    let all = || {
        let first = site!("first");
        let second = site!("second");
        let third = site!("third");
        [first, second, third]
    };
    let names: Vec<String> =
        all().iter().map(|it| format!("{} {}", it.meta.site.meta.name, it.meta.line)).collect();
    format!("{:?} {}", T::default(), names.join(", "))
}

fn plain() -> [&'static str; 2] {
    let first = site!("one");
    let second = site!("two");
    [first.meta.name, second.meta.name]
}

/// A site in a closure in a closure, after those of the outer closure: its statics are
/// told from the others by the places of both.
fn nested() -> Vec<&'static str> {
    let outer = || {
        let first = site!("first");
        let second = site!("second");
        let third = site!("third");
        let inner = || site!("inner");
        vec![first, second, third, inner()]
    };
    outer().iter().map(|it| it.meta.site.meta.name).collect()
}

pub fn run() -> (String, [&'static str; 2], Vec<&'static str>) {
    (sites::<u8>(), plain(), nested())
}
