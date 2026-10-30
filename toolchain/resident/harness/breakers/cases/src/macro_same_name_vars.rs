//! petgraph's `defmac!` writes a `macro_rules!` whose parameters all come from one `$arg`
//! of the outer macro, so every one of them is spelled `marg` and they differ by the expansion
//! that made them. Bindings were kept by name alone, and the last `$marg` won for all.

pub struct Graph(Vec<u32>);

impl Graph {
    fn edges(&self, from: u32) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().map(move |to| from * 10 + to)
    }
}

macro_rules! defmac {
    (@nest $name:ident ($dol:tt) => ([$($arg:ident)*] $($body:tt)+)) => {
        macro_rules! $name {
            ($($dol $arg : expr), *) => { $($body)+ }
        }
    };
    (@nest $name:ident ($dol:tt) => ([$($arg:ident)*] $($body:tt)+) $p1:pat $(, $p2:pat)*) => {
        defmac!{@nest $name ($dol) => (
            [marg $($arg)*]
            match {$dol marg} { $p1 => $($body)+ }
        ) $($p2),* }
    };
    (@revpats [$($args:tt)*] [$($pr:pat),*]) => {
        defmac!{@nest $($args)* $($pr),*}
    };
    (@revpats [$($args:tt)*] [$($pr:pat),*] $p1:pat $(, $p2:pat)*) => {
        defmac!{@revpats [$($args)*] [$p1 $(, $pr)*] $($p2),*}
    };
    ($name:ident $($p1:pat),* => $result:expr) => {
        defmac!{@revpats [$name ($) => ([] $result)] [] $($p1),*}
    };
}

defmac!(edges ref graph, from => graph.edges(from).collect::<Vec<_>>());
defmac!(pair left, right => (left, right));

pub fn run() -> (Vec<u32>, (u32, &'static str)) {
    let graph = Graph(vec![1, 2, 3]);
    (edges!(graph, 4), pair!(1, "one"))
}
