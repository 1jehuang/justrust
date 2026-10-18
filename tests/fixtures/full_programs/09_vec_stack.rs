struct Stack<T> {
    items: Vec<T>,
}

impl<T> Stack<T> {
    fn new() -> Self {
        Stack { items: Vec::new() }
    }
    fn push(&mut self, item: T) {
        self.items.push(item);
    }
    fn pop(&mut self) -> Option<T> {
        self.items.pop()
    }
    fn peek(&self) -> Option<&T> {
        self.items.last()
    }
    fn len(&self) -> usize {
        self.items.len()
    }
    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

fn balanced(s: &str) -> bool {
    let mut stack = Stack::new();
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => stack.push(c),
            ')' | ']' | '}' => {
                let open = match stack.pop() {
                    Some(o) => o,
                    None => return false,
                };
                let ok = matches!((open, c), ('(', ')') | ('[', ']') | ('{', '}'));
                if !ok {
                    return false;
                }
            }
            _ => {}
        }
    }
    stack.is_empty()
}

fn main() {
    let mut s: Stack<i32> = Stack::new();
    for i in 1..=5 {
        s.push(i * i);
    }
    println!("len = {}, top = {:?}", s.len(), s.peek());
    while let Some(x) = s.pop() {
        print!("{} ", x);
    }
    println!();
    for case in ["(a[b]{c})", "(]", "((", "{[()()]}", ""] {
        println!("{:?} balanced: {}", case, balanced(case));
    }
}
