// bun's `task_tag::tags!`: `COUNT` recurses through `@count` with `$n + 1u8`, and the rule
// `@ $n:expr,` before it must not take `count` followed by the fragment `$n` for a call.
#[derive(Clone, Copy)]
pub struct TaskTag(pub u8);

#[allow(non_upper_case_globals)]
pub mod task_tag {
    use super::TaskTag;
    macro_rules! tags {
        ($($name:ident),* $(,)?) => {
            tags!(@ 0u8, $($name,)*);
            pub const COUNT: u8 = tags!(@count 0u8, $($name,)*);
            pub const NAMES: [&str; COUNT as usize] = [$(stringify!($name)),*];
        };
        (@ $n:expr, $head:ident, $($rest:ident,)*) => {
            pub const $head: TaskTag = TaskTag($n);
            tags!(@ $n + 1u8, $($rest,)*);
        };
        (@ $n:expr,) => {};
        (@count $n:expr, $head:ident, $($rest:ident,)*) => { tags!(@count $n + 1u8, $($rest,)*) };
        (@count $n:expr,) => { $n };
    }
    tags! {
        AnyTaskJob,
        AsyncModule,
        BakeHotReloadEvent,
        ChromePipeEvent,
        Close,
        CppTask,
    }
}

impl TaskTag {
    pub fn name(self) -> &'static str {
        task_tag::NAMES.get(self.0 as usize).copied().unwrap_or("?")
    }
}
