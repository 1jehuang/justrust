#![feature(proc_macro_span)]
extern crate proc_macro;
use proc_macro::{Delimiter, Span, TokenStream, TokenTree as T};
mod candidate;
mod control;
mod fast;

thread_local! { static TOTAL: std::cell::RefCell<std::collections::BTreeMap<&'static str,u128>>=Default::default(); static ERRORS: std::cell::Cell<usize>=const { std::cell::Cell::new(0) }; }
fn timed<T>(phase: &'static str, f: impl FnOnce() -> T) -> T {
    let t = std::time::Instant::now();
    let v = f();
    let ns = t.elapsed().as_nanos();
    TOTAL.with(|m| *m.borrow_mut().entry(phase).or_default() += ns);
    v
}
fn same_span(a: Span, b: Span) -> bool {
    a.eq(&b)
}
fn compare(a: TokenStream, b: TokenStream) -> Result<(), String> {
    let mut a = a.into_iter();
    let mut b = b.into_iter();
    loop {
        match (a.next(), b.next()) {
            (None, None) => return Ok(()),
            (Some(x), Some(y)) => {
                if !same_span(x.span(), y.span()) {
                    return Err(format!("span {x:?} != {y:?}"));
                }
                match (&x, &y) {
                    (T::Group(g), T::Group(h))
                        if g.delimiter() == h.delimiter()
                            && same_span(g.span_open(), h.span_open())
                            && same_span(g.span_close(), h.span_close()) =>
                    {
                        compare(g.stream(), h.stream())?
                    }
                    (T::Punct(p), T::Punct(q))
                        if (p.as_char(), p.spacing()) == (q.as_char(), q.spacing()) =>
                    {
                        ()
                    }
                    (T::Literal(p), T::Literal(q)) if p.to_string() == q.to_string() => (),
                    (T::Ident(p), T::Ident(q)) if p.to_string() == q.to_string() => (),
                    _ => return Err(format!("kind/value/spacing/delimiters {x:?} != {y:?}")),
                }
            }
            _ => return Err("length mismatch".into()),
        }
    }
}
fn visit(s: TokenStream, count: &mut usize, failures: &mut usize) {
    let t: Vec<_> = s.into_iter().collect();
    for (i, e) in t.iter().enumerate() {
        let T::Group(g) = e else { continue };
        if g.delimiter() == Delimiter::Bracket
            && i > 0
            && matches!(&t[i-1],T::Punct(p) if p.as_char()=='#')
        {
            let at: Vec<_> = g.stream().into_iter().collect();
            if at.len() >= 4
                && at[..4].iter().map(ToString::to_string).collect::<String>() == "gpui::test"
            {
                let args = match at.get(4) {
                    Some(T::Group(g)) => g.stream(),
                    None => TokenStream::new(),
                    _ => panic!("attribute shape"),
                };
                let end = (i + 1..t.len())
                    .find(|j| matches!(&t[*j],T::Group(g) if g.delimiter()==Delimiter::Brace))
                    .unwrap();
                let item = t[i + 1..=end].iter().cloned().collect::<TokenStream>();
                let ac = args.clone();
                let ic = item.clone();
                let (a, b) = if (*count % 2 == 0) ^ std::env::var_os("REPLAY_REVERSE").is_some() {
                    let a = timed("control.total", || control::test(ac, ic));
                    let b = timed("candidate.total", || candidate::test(args, item));
                    (a, b)
                } else {
                    let b = timed("candidate.total", || candidate::test(args, item));
                    let a = timed("control.total", || control::test(ac, ic));
                    (a, b)
                };
                if a.clone()
                    .into_iter()
                    .any(|t| matches!(t,T::Ident(i) if i.to_string()=="compile_error"))
                {
                    ERRORS.with(|e| e.set(e.get() + 1));
                }
                *count += 1;
                if let Err(err) = compare(a, b) {
                    *failures += 1;
                    if *failures <= 1 {
                        eprintln!("DIFFERENCE {:?} {err}", g.span());
                    }
                }
            }
        }
        visit(g.stream(), count, failures);
    }
}
#[proc_macro_attribute]
pub fn probe(args: TokenStream, item: TokenStream) -> TokenStream {
    if std::env::var_os("PROBE_CANDIDATE").is_some() {
        candidate::test(args, item)
    } else {
        control::test(args, item)
    }
}
#[proc_macro]
pub fn replay(s: TokenStream) -> TokenStream {
    let mut n = 0;
    let mut f = 0;
    visit(s, &mut n, &mut f);
    eprintln!("GPUI_REPLAY {n} {f} {}", ERRORS.with(|e| e.replace(0)));
    TOTAL.with(|m| {
        for (key, v) in std::mem::take(&mut *m.borrow_mut()) {
            eprintln!("GPUI_PHASE {key} {v}");
        }
    });
    TokenStream::new()
}
