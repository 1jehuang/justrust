#!/usr/bin/env python3
"""Associated-item native retention gate, reusing the established compiler runner."""
import importlib.util
from pathlib import Path

_spec = importlib.util.spec_from_file_location('query_retention_accept', Path(__file__).with_name('reuse-query-accept.py'))
query = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(query)

ASSOCIATED = '''
trait Dispatch {
    type Output;
    const BIAS: u32;
    fn number(&self) -> u32;
    fn opaque(&self) -> impl Copy { 1u8 }
}
struct Widget;
impl Dispatch for Widget {
    type Output = u32;
    const BIAS: u32 = 0;
    fn number(&self) -> u32 { 2 + Self::BIAS }
}
trait Other { fn number(&self) -> u32; }
impl Other for Widget { fn number(&self) -> u32 { 90 } }
macro_rules! define_inherent { () => { impl Widget { fn inherent() -> u32 { 0 } } }; }
define_inherent!();
'''
ORIGINAL_CASES = query.cases
ORIGINAL_BASE = query.BASE


def configure():
    query.FLAG = '-Zjr-cache-associated-items'
    query.SELECTED = ('associated_items',)
    query.BASE = ORIGINAL_BASE.replace('value() + ordinary()', 'value() + ordinary() + Dispatch::number(&Widget) - 2 + Widget::inherent()') + ASSOCIATED
    query.cases = cases


def cases():
    # Existing ordered edits include macros, owner churn, const/async changes,
    # dependency signature changes, and exact error/recovery comparisons.
    base = query.BASE
    return ORIGINAL_CASES() + [
        ('trait-method-edit', base.replace('2 + Self::BIAS', '3 + Self::BIAS'), query.DEP, '16\n'),
        ('trait-method-revert', base, query.DEP, '15\n'),
        ('associated-type-edit', base.replace('type Output = u32', 'type Output = u64'), query.DEP, '15\n'),
        ('associated-const-edit', base.replace('const BIAS: u32 = 0', 'const BIAS: u32 = 1'), query.DEP, '16\n'),
        ('associated-add', base.replace('type Output;','type Output; fn added(&self) -> u8 { 4 }'), query.DEP, '15\n'),
        ('associated-reorder', base.replace('    type Output;\n    const BIAS: u32;', '    const BIAS: u32;\n    type Output;'), query.DEP, '15\n'),
        ('rpitit-edit', base.replace('impl Copy { 1u8 }', 'impl Copy { 1u16 }'), query.DEP, '15\n'),
        ('symbol-churn', base.replace('extern crate retention_dep;', 'extern crate retention_dep;\n#[allow(dead_code)] mod churn { struct Zebra; struct Alpha; }'), query.DEP, '15\n'),
        ('missing-associated-item', base.replace('type Output = u32;', ''), query.DEP, None),
        ('duplicate-associated-item', base.replace('type Output = u32;', 'type Output = u32; type Output = u64;'), query.DEP, None),
        ('ambiguous-method', base.replace('Dispatch::number(&Widget)', 'Widget.number()'), query.DEP, None),
        ('associated-recovery', base, query.DEP, '15\n'),
    ]


if __name__ == '__main__':
    configure()
    query.main()
