//! Function qualifier grammar, shared by free functions and associated items.
use super::*;

fn rank(token: &TokenView<'_>) -> Option<u8> {
    if token.string {
        return None;
    }
    match token.text {
        "const" => Some(1),
        "async" => Some(2),
        "unsafe" => Some(3),
        "extern" => Some(4),
        _ => None,
    }
}

impl Parser<'_, '_> {
    /// A bounded lookahead distinguishes const declarations from qualified fns.
    /// Invalid qualifier orders still reach the grammar checker, not a const name.
    pub(super) fn starts_qualified_function(&self) -> bool {
        let mut at = 0;
        while at < 8 {
            let token = if at == 0 {
                Some(self.token())
            } else {
                self.cursor.peek(at)
            };
            let Some(token) = token else {
                return false;
            };
            if token.matches("fn") {
                return true;
            }
            if rank(&token).is_none() {
                return false;
            }
            at += 1;
            if token.matches("extern") && self.cursor.peek(at).is_some_and(|token| token.string) {
                at += 1;
            }
        }
        false
    }

    pub(super) fn function_qualifiers(
        &mut self,
        associated: bool,
    ) -> Result<(Constness, bool), ParseError> {
        let mut previous = 0;
        let mut constant = false;
        let mut unsafety = false;
        let mut asynchronous = false;
        let mut external = false;
        while let Some(next) = rank(&self.token()) {
            if next <= previous {
                return Err(self.error(
                    "function qualifiers must appear once in const async unsafe extern order",
                ));
            }
            previous = next;
            constant |= next == 1;
            asynchronous |= next == 2;
            unsafety |= next == 3;
            external |= next == 4;
            self.cursor.bump();
            if next == 4 && self.token().string {
                self.cursor.bump();
            }
        }
        if !self.is("fn") {
            return Err(self.error("expected fn after function qualifiers"));
        }
        if constant && asynchronous {
            return Err(self.error("a function cannot be both const and async"));
        }
        if asynchronous {
            return Err(self.unsupported("async function semantics are unsupported"));
        }
        if external {
            return Err(self.unsupported("extern ABI function qualifiers are unsupported"));
        }
        Ok((
            if constant && !associated {
                Constness::Const
            } else {
                Constness::Runtime
            },
            unsafety,
        ))
    }
}
