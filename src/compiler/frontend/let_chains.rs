//! Edition 2024 `if let` / `while let` chains:
//! `if let P = a && cond && let Q = b { T } else { E }`.
//!
//! The parser desugars a chain into nested `if` / `if let` expressions whose
//! failure paths each carry a copy of the else branch. Exactly one copy runs,
//! so evaluation order, short-circuiting and binding scopes match stock:
//! every operand sees the bindings of the `let`s before it, and the then
//! block sees all of them. `while` chains become `loop { if chain { body }
//! else { break } }` under the same label.
use super::{AstIdent, Block, ConditionPurpose, Expr, ParseError, Parser, Pattern, Stmt};

pub(super) enum ChainCond {
    Bool(Expr),
    Let(Box<Pattern>, Box<Expr>),
}

impl<'input, 'names> Parser<'input, 'names> {
    /// True when the cursor is at `&& let`, the start of a further chain operand.
    pub(super) fn at_chain_let(&self) -> bool {
        self.is("&&") && self.cursor.peek(1).is_some_and(|t| t.matches("let"))
    }

    /// Parse the remaining `&& operand` items of a chain whose first operand
    /// is `first`, binding each pattern for the operands that follow. The
    /// caller owns the lexical frontier and restores it.
    pub(super) fn chain_rest(&mut self, first: ChainCond) -> Result<Vec<ChainCond>, ParseError> {
        if self.names.edition() != super::super::token::Edition::E2024 {
            return Err(self.error("let chains are only allowed in Rust 2024 or later"));
        }
        let mut conds = vec![first];
        if let Some(ChainCond::Let(pattern, _)) = conds.last() {
            let pattern: &Pattern = pattern;
            self.bind_pattern_lexical(pattern)?;
        }
        while self.eat("&&") {
            if conds.len() >= 256 {
                return Err(self.limit("let chain length exceeds limit 256"));
            }
            let cond = if self.eat("let") {
                let pattern = self.pattern()?;
                self.expect("=")?;
                let scrutinee = self.condition_with(3, ConditionPurpose::Conditional)?;
                self.bind_pattern_lexical(&pattern)?;
                ChainCond::Let(
                    self.names
                        .try_parser_box(pattern)
                        .map_err(super::native_adapter::name_error)?,
                    self.names
                        .try_parser_box(scrutinee)
                        .map_err(super::native_adapter::name_error)?,
                )
            } else {
                ChainCond::Bool(self.condition_with(3, ConditionPurpose::Conditional)?)
            };
            conds.push(cond);
        }
        if self.is("||") {
            return Err(self.error("`||` operators are not supported in let chain conditions"));
        }
        Ok(conds)
    }

    pub(super) fn chain_expr(
        conds: Vec<ChainCond>,
        then_block: Block,
        else_branch: Option<Expr>,
    ) -> Expr {
        let else_block = |branch: &Option<Expr>| match branch {
            Some(Expr::Block(block)) => block.clone(),
            Some(other) => Block {
                statements: Vec::new(),
                tail: Some(Box::new(other.clone())),
            },
            None => Block {
                statements: Vec::new(),
                tail: None,
            },
        };
        let mut body = then_block;
        let mut result = None;
        for cond in conds.into_iter().rev() {
            let expr = match cond {
                ChainCond::Let(pattern, scrutinee) => Expr::IfLet {
                    pattern,
                    scrutinee,
                    then_block: body,
                    else_branch: else_branch.clone().map(Box::new),
                },
                ChainCond::Bool(condition) => Expr::If {
                    condition: Box::new(condition),
                    then_block: body,
                    else_block: else_block(&else_branch),
                },
            };
            body = Block {
                statements: Vec::new(),
                tail: Some(Box::new(expr.clone())),
            };
            result = Some(expr);
        }
        result.expect("a let chain has at least one operand")
    }

    pub(super) fn while_chain(label: Option<AstIdent>, conds: Vec<ChainCond>, body: Block) -> Stmt {
        let exit = Expr::Break { label, value: None };
        let exit = Expr::Block(Block {
            statements: vec![Stmt::Expr(exit)],
            tail: None,
        });
        let test = Self::chain_expr(conds, body, Some(exit));
        Stmt::While {
            label,
            condition: Expr::Bool(true),
            body: Block {
                statements: vec![Stmt::ImplicitExpr(test)],
                tail: None,
            },
        }
    }
}

impl<'input, 'names> Parser<'input, 'names> {
    pub(super) fn if_chain(
        &mut self,
        first: ChainCond,
        statement: bool,
    ) -> Result<Expr, ParseError> {
        let frontier = self.enter_lexical()?;
        let parsed = (|| {
            let conds = self.chain_rest(first)?;
            let then_block = self.block()?;
            Ok((conds, then_block))
        })();
        self.restore_frontier(frontier);
        let (conds, then_block) = parsed?;
        let else_branch = if self.eat("else") {
            Some(if self.is("if") {
                if statement {
                    self.statement_expr()?
                } else {
                    self.expr(0)?
                }
            } else {
                Expr::Block(self.block()?)
            })
        } else {
            None
        };
        Ok(Self::chain_expr(conds, then_block, else_branch))
    }

    pub(super) fn while_chain_statement(
        &mut self,
        label: Option<AstIdent>,
        first: ChainCond,
    ) -> Result<Stmt, ParseError> {
        let frontier = self.enter_lexical()?;
        let parsed = (|| {
            let conds = self.chain_rest(first)?;
            let body = self.block()?;
            Ok((conds, body))
        })();
        self.restore_frontier(frontier);
        let (conds, body) = parsed?;
        self.eat(";");
        Ok(Self::while_chain(label, conds, body))
    }
}
