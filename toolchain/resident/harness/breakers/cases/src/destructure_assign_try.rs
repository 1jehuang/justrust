//! A destructuring assignment whose value is a `?` of a call, in a loop that walks down a
//! tree: cargo's `get_key_value` of a manifest path.

struct Node {
    name: &'static str,
    weight: u32,
    children: Vec<Node>,
}

impl Node {
    fn child(&self, name: &str) -> Option<(&'static str, &Node)> {
        self.children.iter().find(|it| it.name == name).map(|it| (it.name, it))
    }
}

fn walk<'a>(root: &'a Node, path: &[&str]) -> Option<(&'static str, &'a Node)> {
    let mut names = path.iter();
    let (mut key, mut node) = root.child(names.next()?)?;
    while let Some(name) = names.next() {
        if node.weight > 0 {
            (key, node) = node.child(name)?;
        } else {
            node = node.children.first()?;
        }
    }
    Some((key, node))
}

fn tree() -> Node {
    let leaf = |name, weight| Node { name, weight, children: Vec::new() };
    Node {
        name: "root",
        weight: 1,
        children: vec![
            Node { name: "a", weight: 2, children: vec![leaf("x", 3), leaf("y", 4)] },
            Node { name: "b", weight: 0, children: vec![leaf("z", 5)] },
        ],
    }
}

pub fn run() -> Vec<Option<(&'static str, u32)>> {
    let root = tree();
    [&["a"][..], &["a", "y"], &["a", "q"], &["b", "anything"], &["c"], &[]]
        .iter()
        .map(|path| walk(&root, path).map(|(key, node)| (key, node.weight)))
        .collect()
}
