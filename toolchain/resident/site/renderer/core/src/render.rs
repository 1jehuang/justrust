//! The site's world as HTML pages, as render/render.ts and render/widgets.ts: one function per element type of
//! content/schema.flecs; a page is an entity that is an instance of the Page prefab. The output matches the TS
//! renderer's byte for byte, so the order things are computed in follows its template literals where that order is
//! visible (citation numbers, the block list).

use std::collections::BTreeMap;

use crate::comments::{self, Discussion, PAGE_BLOCK, is_comment};
use crate::inline::{Refs, esc, inline};
use crate::js;
use crate::text::{dedent, prose};
use crate::world::{Entity, Id, Value, World};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub node: Id,
    /// `/`, `/rfcs`, `/rfcs/capabilities`.
    pub url: String,
    /// The file under dist/: `index.html`, `rfcs.html`, `rfcs/capabilities.html`.
    pub file: String,
    /// Comments are keyed by this: the URL without its slash (`rfcs/capabilities`).
    pub doc: String,
}

/// What a host needs to check a comment on a discussable page: its title and each block's text (the first 280 UTF-16
/// units), the page itself as `_page`.
#[derive(Clone, Debug, PartialEq)]
pub struct Blocks {
    pub doc: String,
    pub title: String,
    /// In the order the TS object has them: `_page` first, then each block as it was rendered.
    pub blocks: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NavItem {
    pub label: String,
    pub href: String,
    pub order: f64,
}

pub fn is_page(e: &Entity) -> bool {
    e.is_a("Page")
}

pub fn pages(world: &World, site: Id) -> Vec<Page> {
    fn walk(world: &World, site: Id, n: Id, out: &mut Vec<Page>) {
        let base = world.get(site).path.len() + 1;
        for &c in &world.get(n).children {
            let e = world.get(c);
            if !is_page(e) {
                continue;
            }
            let rel = e.path[base..].replace('.', "/");
            let url = if rel == "index" { "/".to_string() } else { format!("/{rel}") };
            let file =
                if url == "/" { "index.html".to_string() } else { format!("{}.html", &url[1..]) };
            let doc = if url == "/" { "index".to_string() } else { url[1..].to_string() };
            out.push(Page { node: c, url, file, doc });
            walk(world, site, c, out);
        }
    }
    let mut out = Vec::new();
    walk(world, site, site, &mut out);
    out
}

fn field(v: Option<&Value>, k: &str) -> String {
    js::string(v.and_then(|v| v.get(k)))
}

fn text(e: &Entity, comp: &str) -> String {
    field(e.value(comp), "text")
}

/// The top bar's links: pages with Nav and nav's Link children, by order.
pub fn bar(world: &World, list: &[Page]) -> Result<Vec<NavItem>, String> {
    let mut items: Vec<NavItem> = list
        .iter()
        .filter_map(|p| {
            let v = world.get(p.node).value("Nav")?;
            Some(NavItem {
                label: field(Some(v), "label"),
                href: p.url.clone(),
                order: js::to_number(v.get("order")),
            })
        })
        .collect();
    if let Some(nav) = world.lookup("nav") {
        for &c in &world.get(nav).children {
            let n = world.get(c);
            let v =
                n.value("Link").ok_or_else(|| format!("{}: nav's children are Links", n.path))?;
            let href = field(Some(v), "href");
            if !(href.starts_with("http://")
                || href.starts_with("https://")
                || href.starts_with('/'))
            {
                return Err(format!("{}: a Link's href is http(s) or /", n.path));
            }
            items.push(NavItem {
                label: field(Some(v), "label"),
                href,
                order: js::to_number(v.get("order")),
            });
        }
    }
    // Array.prototype.sort is stable, as sort_by is.
    items
        .sort_by(|a, b| (a.order - b.order).partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal));
    Ok(items)
}

/// Where the site is served (`Served` on `site`): the origin (`https://ix.dev`) and the path every page sits under
/// (`/blaze`). Both empty when the site has no `Served`: pages at the root, no canonical URL.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Served {
    pub origin: String,
    pub base: String,
}

impl Served {
    /// `site`'s `Served`, checked: an http(s) origin with no path, and a base that is empty or `/a/b` with no trailing
    /// slash.
    pub fn of(world: &World, site: Id) -> Result<Served, String> {
        let Some(v) = world.get(site).value("Served") else { return Ok(Served::default()) };
        let (origin, base) = (field(Some(v), "origin"), field(Some(v), "base"));
        let host = origin.strip_prefix("https://").or_else(|| origin.strip_prefix("http://"));
        if !host.is_some_and(|h| !h.is_empty() && !h.contains('/')) {
            return Err(format!("site: Served origin {origin:?} is not http(s)://host"));
        }
        let ok_base = base.is_empty()
            || (base.starts_with('/')
                && !base.ends_with('/')
                && !base.contains("//")
                && base.bytes().all(|b| b.is_ascii_alphanumeric() || b"/-_.".contains(&b)));
        if !ok_base {
            return Err(format!(
                "site: Served base {base:?} is not empty or /path without a trailing slash"
            ));
        }
        Ok(Served { origin, base })
    }

    /// A link as the browser gets it: a site path (`/rfcs/x`, `/`) under the base (`/blaze/rfcs/x`, `/blaze/`);
    /// anything else (`https://...`, `#ref-x`, `//host`) as it is.
    pub fn href(&self, href: &str) -> String {
        if href.starts_with('/') && !href.starts_with("//") {
            format!("{}{href}", self.base)
        } else {
            href.to_string()
        }
    }
}

/// Everything a render reads: the world and the indexes over it the build makes once.
pub struct Site<'w> {
    pub world: &'w World,
    pub site: Id,
    pub pages: &'w [Page],
    pub nav: &'w [NavItem],
    pub refs: Option<Id>,
    pub served: &'w Served,
}

impl Site<'_> {
    fn page_of(&self, n: Id) -> Option<&Page> {
        self.pages.iter().find(|p| p.node == n)
    }
}

struct Ctx<'a, 'w> {
    s: &'a Site<'w>,
    page: &'a Page,
    cited: Vec<Id>,
    discussable: bool,
    blocks: Vec<(String, String)>,
    scripts: Vec<&'static str>,
    discussion: &'a Discussion,
    /// On a page with audio (an `Audio` value, set by the build): the blocks that have a spoken line. Each gets
    /// ` data-say="<id>"`, which the player (client/listen.ts) highlights and plays from.
    say: Option<std::collections::BTreeSet<String>>,
}

impl Ctx<'_, '_> {
    fn e(&self, n: Id) -> &Entity {
        self.s.world.get(n)
    }

    fn inline(&mut self, t: &str, wher: &str) -> Result<String, String> {
        let refs = Refs { world: self.s.world, parent: self.s.refs, served: self.s.served };
        inline(t, &refs, wher, &mut self.cited)
    }

    /// ` data-say=".."` when the block has a spoken line on a page with audio.
    fn say_attr(&self, n: Id) -> String {
        let id = self.block_id(n);
        match &self.say {
            Some(set) if set.contains(&id) => format!(" data-say=\"{}\"", esc(&id)),
            _ => String::new(),
        }
    }

    fn script(&mut self, s: &'static str) {
        if !self.scripts.contains(&s) {
            self.scripts.push(s);
        }
    }

    /// The block's anchor: its path under the page (`motivation.why`).
    fn block_id(&self, n: Id) -> String {
        self.e(n).path[self.e(self.page.node).path.len() + 1..].to_string()
    }
}

#[derive(Default)]
struct Anchor {
    /// ` data-block=".."` for the block's element.
    attr: String,
    /// The count button, inside the element.
    button: String,
    /// The thread, after the element (inside it for an li).
    thread: String,
}

/// What a block a reader can comment on carries, after checking it has a name. Empty outside a Discussable page.
fn anchor(n: Id, ctx: &mut Ctx, plain: &str) -> Result<Anchor, String> {
    let say = ctx.say_attr(n);
    if !ctx.discussable {
        return Ok(Anchor { attr: say, ..Anchor::default() });
    }
    let e = ctx.e(n);
    if e.name.starts_with('#') {
        let parent = e.parent.map(|p| ctx.e(p).path.clone()).unwrap_or_default();
        return Err(format!(
            "{}: a block in a Doc needs a name (anonymous child of {parent})",
            ctx.e(ctx.page.node).path
        ));
    }
    let id = ctx.block_id(n);
    let short = js::slice_utf16(plain, 280).to_string();
    match ctx.blocks.iter_mut().find(|(k, _)| *k == id) {
        Some((_, v)) => *v = short,
        None => ctx.blocks.push((id.clone(), short)),
    }
    let w = ctx.s.world;
    Ok(Anchor {
        attr: format!(" data-block=\"{}\"{say}", esc(&id)),
        button: comments::button(&id, w, n),
        thread: comments::thread(
            &id,
            w,
            n,
            &ctx.page.doc,
            &ctx.page.url,
            ctx.discussion,
            ctx.s.served,
        ),
    })
}

/// A superscript per reference the block cites with (Cites, refs.x).
fn cites(n: Id, ctx: &mut Ctx) -> Result<String, String> {
    let e = ctx.e(n);
    let targets = e.targets("Cites").map(<[String]>::to_vec).unwrap_or_default();
    let mut out = String::new();
    for t in targets {
        let r = ctx.s.world.lookup(&t).filter(|r| ctx.e(*r).has("Ref"));
        let Some(r) = r else {
            return Err(format!("{}: (Cites, {t}) is not a Ref", ctx.e(n).path));
        };
        if !ctx.cited.contains(&r) {
            ctx.cited.push(r);
        }
        let i = ctx.cited.iter().position(|x| *x == r).expect("just pushed") + 1;
        out +=
            &format!("<sup class=\"cite\"><a href=\"#ref-{}\">{i}</a></sup>", esc(&ctx.e(r).name));
    }
    Ok(out)
}

fn prosed(n: Id, comp: &str, ctx: &mut Ctx) -> Result<(String, String), String> {
    let t = prose(&dedent(&text(ctx.e(n), comp)));
    let path = ctx.e(n).path.clone();
    Ok((ctx.inline(&t, &path)?, t))
}

fn children(n: Id, ctx: &mut Ctx) -> Result<String, String> {
    let kids: Vec<Id> = ctx
        .e(n)
        .children
        .iter()
        .copied()
        .filter(|c| !is_page(ctx.e(*c)) && !is_comment(ctx.e(*c)))
        .collect();
    let mut out = Vec::with_capacity(kids.len());
    for c in kids {
        out.push(block(c, ctx)?);
    }
    Ok(out.join("\n"))
}

fn status(e: &Entity) -> String {
    match e.targets("Status").and_then(|t| t.first()) {
        Some(s) => {
            format!("<span class=\"status status-{}\">{}</span>", esc(&s.to_lowercase()), esc(s))
        }
        None => String::new(),
    }
}

const REFERENCES: &str = "<!--references-->";

/// The tags that render through a widget (render/widgets.ts), in the order the TS object lists them.
const WIDGETS: [&str; 4] = ["RfcIndex", "References", "SignupForm", "BenchHistory"];

const SIGNUP_FORM: &str = r#"<form method="POST" class="signup">
<input type="hidden" name="ref" value="" />
<label><span>Email</span><input name="email" type="email" required maxlength="254" autocomplete="email" /></label>
<label><span>Company <em>Optional</em></span><input name="company" type="text" maxlength="200" autocomplete="organization" /></label>
<label><span>Why do you want to use blaze? <em>Optional</em></span><textarea name="why" rows="3" maxlength="2000"></textarea></label>
<div class="turnstile-slot"></div>
<p class="error" role="alert" hidden></p>
<button type="submit">Send</button>
</form>
<div class="sent" hidden><p class="lede">Thanks, got it.</p><p class="dim"><a href="{home}">Back to blaze</a></p></div>"#;

fn widget(tag: &str, n: Id, ctx: &mut Ctx) -> Result<String, String> {
    match tag {
        "RfcIndex" => {
            let parent = ctx.e(n).parent.ok_or("RfcIndex has no parent")?;
            let mut items = String::new();
            for c in ctx.s.world.rfcs(parent) {
                let e = ctx.e(c);
                let p = ctx
                    .s
                    .page_of(c)
                    .ok_or_else(|| format!("{}: an Rfc that is not a page", e.path))?;
                let num = js::pad_zero(&field(e.value("Number"), "value"), 4);
                let st = e.targets("Status").and_then(|t| t.first()).cloned().unwrap_or_default();
                let desc = prose(&text(e, "Description"));
                let desc = if desc.is_empty() {
                    String::new()
                } else {
                    format!("<br /><span class=\"dim\">{}</span>", esc(&desc))
                };
                items += &format!(
                    "<li><a href=\"{}\"><span class=\"num mono\">{num}</span> {}</a> <span class=\"status status-{}\">{}</span>{desc}</li>",
                    esc(&ctx.s.served.href(&p.url)),
                    esc(&text(e, "Title")),
                    esc(&st.to_lowercase()),
                    esc(&st)
                );
            }
            Ok(format!("<ul class=\"rfc-index\">{items}</ul>"))
        }
        // Filled in once the whole page has rendered, so it lists citations made after it too.
        "References" => Ok(REFERENCES.into()),
        "SignupForm" => {
            ctx.script("updates");
            Ok(SIGNUP_FORM.replace("{home}", &esc(&ctx.s.served.href("/"))))
        }
        "BenchHistory" => {
            ctx.script("bench");
            // gen/bench/ (TS, at build time) renders the charts; the build stores the HTML as the node's Baked value.
            match ctx.e(n).value("Baked").and_then(|v| v.get("html")) {
                Some(crate::world::Scalar::Str(h)) => Ok(h.clone()),
                _ => Err(format!("{}: not baked (run the build)", ctx.e(n).path)),
            }
        }
        _ => unreachable!("WIDGETS lists the tags"),
    }
}

/// The HTML the build highlighted for a Code or Command block, if any.
fn highlighted(e: &Entity) -> Option<String> {
    match e.value("Highlighted")?.get("html")? {
        crate::world::Scalar::Str(h) => Some(h.clone()),
        _ => None,
    }
}

fn block(n: Id, ctx: &mut Ctx) -> Result<String, String> {
    if let Some(tag) = WIDGETS.iter().find(|t| ctx.e(n).has_tag(t)) {
        return widget(tag, n, ctx);
    }
    let e = ctx.e(n);
    if let Some(comp) = ["Lede", "P", "Dim"].into_iter().find(|c| e.has(c)) {
        let (html, plain) = prosed(n, comp, ctx)?;
        let cls =
            if comp == "P" { String::new() } else { format!(" class=\"{}\"", comp.to_lowercase()) };
        let a = anchor(n, ctx, &plain)?;
        let c = cites(n, ctx)?;
        return Ok(format!("<p{cls}{}>{html}{c}{}</p>{}", a.attr, a.button, a.thread));
    }
    if e.has("H3") {
        let (html, plain) = prosed(n, "H3", ctx)?;
        let a = anchor(n, ctx, &plain)?;
        return Ok(format!(
            "<h3 id=\"{}\"{}>{html}{}</h3>{}",
            esc(&ctx.block_id(n)),
            a.attr,
            a.button,
            a.thread
        ));
    }
    if e.has("Li") {
        let (html, plain) = prosed(n, "Li", ctx)?;
        let nested = children(n, ctx)?;
        let a = anchor(n, ctx, &plain)?;
        let c = cites(n, ctx)?;
        return Ok(format!("<li{}>{html}{c}{}{nested}{}</li>", a.attr, a.button, a.thread));
    }
    if let Some(c) = e.value("Code") {
        let lang = field(Some(c), "lang");
        let code = dedent(&field(Some(c), "text"));
        // The build highlights rust, toml and bash (highlight.js escapes every text run).
        let html = highlighted(e).unwrap_or_else(|| esc(&code));
        let a = anchor(n, ctx, &code)?;
        return Ok(format!(
            "<figure class=\"codeblock code-roles\"{}><pre class=\"hljs\" data-lang=\"{}\"><code>{html}</code></pre>{}</figure>{}",
            a.attr,
            esc(&lang),
            a.button,
            a.thread
        ));
    }
    if e.has("Command") {
        let cmd = prose(&text(e, "Command"));
        let html = highlighted(e).filter(|h| !h.is_empty()).unwrap_or_else(|| esc(&cmd));
        ctx.script("copy");
        return Ok(format!(
            "<div class=\"cmd code-roles\"{}><pre class=\"hljs\"><code>{html}</code></pre><button type=\"button\" data-copy=\"{}\" aria-label=\"Copy command\">Copy</button></div>",
            ctx.say_attr(n),
            esc(&cmd)
        ));
    }
    if let Some(i) = e.value("Img") {
        let alt = field(Some(i), "alt");
        let (src, w, h) =
            (field(Some(i), "src"), js::to_number(i.get("width")), js::to_number(i.get("height")));
        let a = anchor(n, ctx, &alt)?;
        return Ok(format!(
            "<img src=\"{}\" width=\"{}\" height=\"{}\" alt=\"{}\"{} />{}{}",
            esc(&ctx.s.served.href(&src)),
            js::number(w),
            js::number(h),
            esc(&alt),
            a.attr,
            a.button,
            a.thread
        ));
    }
    if let Some(v) = e.value("Section") {
        let title = field(Some(v), "title");
        let id = esc(&ctx.block_id(n));
        let a = anchor(n, ctx, &title)?;
        let path = ctx.e(n).path.clone();
        let t = ctx.inline(&title, &path)?;
        let kids = children(n, ctx)?;
        return Ok(format!(
            "<section id=\"{id}\"><h2{}><a class=\"hash\" href=\"#{id}\">{t}</a>{}</h2>{}\n{kids}\n</section>",
            a.attr, a.button, a.thread
        ));
    }
    if let Some(v) = e.value("Issue") {
        let title = field(Some(v), "title");
        let id = esc(&ctx.block_id(n));
        let a = anchor(n, ctx, &title)?;
        let path = ctx.e(n).path.clone();
        let t = ctx.inline(&title, &path)?;
        let st = status(ctx.e(n));
        let kids = children(n, ctx)?;
        return Ok(format!(
            "<div class=\"issue\" id=\"{id}\"><h3{}>{t} {st}{}</h3>{}\n{kids}\n</div>",
            a.attr, a.button, a.thread
        ));
    }
    if e.has_tag("Ul") || e.has_tag("Ol") {
        let t = if e.has_tag("Ul") { "ul" } else { "ol" };
        return Ok(format!("<{t}>\n{}\n</{t}>", children(n, ctx)?));
    }
    if e.has_tag("Table") {
        return table(n, ctx);
    }
    let tags = if e.tags.is_empty() { "none".to_string() } else { e.tags.join(", ") };
    let comps = if e.values.is_empty() {
        "none".to_string()
    } else {
        e.values.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(", ")
    };
    Err(format!("{}: no element type (tags {tags}; components {comps})", e.path))
}

/// Every child of the table is a row and every child of a row a cell, comments included, as the TS has it.
fn table(n: Id, ctx: &mut Ctx) -> Result<String, String> {
    let rows: Vec<Id> = ctx.e(n).children.clone();
    let (mut thead, mut tbody) = (String::new(), String::new());
    let mut plain_rows = Vec::new();
    for r in rows {
        let cells: Vec<Id> = ctx.e(r).children.clone();
        let head = cells.iter().all(|c| ctx.e(*c).has("Th"));
        let mut html = String::from("<tr>");
        let mut plain = Vec::new();
        for c in cells {
            let comp = if ctx.e(c).has("Th") { "Th" } else { "Td" };
            let tag = comp.to_lowercase();
            let raw = text(ctx.e(c), comp);
            let path = ctx.e(c).path.clone();
            html += &format!("<{tag}>{}</{tag}>", ctx.inline(&prose(&raw), &path)?);
            plain.push(raw);
        }
        html += "</tr>";
        if head {
            thead += &html
        } else {
            tbody += &html
        }
        plain_rows.push(plain.join(" | "));
    }
    let a = anchor(n, ctx, &plain_rows.join("\n"))?;
    let thead = if thead.is_empty() { String::new() } else { format!("<thead>{thead}</thead>") };
    Ok(format!(
        "<div class=\"ref\"{}><table>{thead}<tbody>{tbody}</tbody></table>{}</div>{}",
        a.attr, a.button, a.thread
    ))
}

fn references_html(ctx: &Ctx) -> String {
    if ctx.cited.is_empty() {
        return String::new();
    }
    let items: String = ctx
        .cited
        .iter()
        .map(|r| {
            let e = ctx.e(*r);
            let v = e.value("Ref");
            let note = field(v, "note");
            let note = if note.is_empty() {
                String::new()
            } else {
                format!(" <span class=\"dim\">{}</span>", esc(&prose(&note)))
            };
            format!(
                "<li id=\"ref-{}\"><a href=\"{}\">{}</a>{note}</li>",
                esc(&e.name),
                esc(&ctx.s.served.href(&field(v, "href"))),
                esc(&field(v, "label"))
            )
        })
        .collect();
    format!(
        "<section id=\"references\"><h2>References</h2><ol class=\"references\">{items}</ol></section>"
    )
}

fn topbar(page: &Page, items: &[NavItem], served: &Served) -> String {
    let links: String = items
        .iter()
        .map(|i| {
            let here = i.href == page.url
                || (i.href != "/" && page.url.starts_with(&format!("{}/", i.href)));
            let outside = i.href.starts_with("http:") || i.href.starts_with("https:");
            format!(
                "<a href=\"{}\"{}{}>{}</a>",
                esc(&served.href(&i.href)),
                if here { " aria-current=\"page\"" } else { "" },
                if outside { " class=\"out\"" } else { "" },
                esc(&i.label)
            )
        })
        .collect();
    format!(
        "<header class=\"topbar\"><nav><a class=\"home\" href=\"{}\">blaze</a><span class=\"links\">{links}</span></nav></header>",
        esc(&served.href("/"))
    )
}

/// The trail of pages above this one: RFCs / .
fn trail(ctx: &Ctx) -> String {
    if ctx.page.url == "/" {
        return String::new();
    }
    let mut up = Vec::new();
    let mut n = ctx.e(ctx.page.node).parent;
    while let Some(id) = n {
        if id == ctx.s.site {
            break;
        }
        if let Some(p) = ctx.s.page_of(id) {
            up.insert(
                0,
                format!(
                    "<a href=\"{}\">{}</a>",
                    esc(&ctx.s.served.href(&p.url)),
                    esc(&text(ctx.e(id), "Title"))
                ),
            );
        }
        n = ctx.e(id).parent;
    }
    // The top bar links home, so the trail starts at the first page below it, and only below the top level.
    if up.is_empty() {
        return String::new();
    }
    format!("<nav class=\"back\">{} <span>/</span></nav>", up.join(" <span>/</span> "))
}

/// RFC 0001 · Draft · Read first: Charter
fn meta(ctx: &Ctx) -> Result<String, String> {
    let e = ctx.e(ctx.page.node);
    let mut parts = Vec::new();
    if let Some(num) = e.value("Number") {
        parts.push(format!("RFC {}", js::pad_zero(&js::string(num.get("value")), 4)));
    }
    if e.targets("Status").is_some() {
        parts.push(status(e));
    }
    let mut deps = Vec::new();
    for d in e.targets("DependsOn").unwrap_or_default() {
        let target = ctx.s.world.lookup(d);
        let p = target
            .and_then(|t| ctx.s.page_of(t))
            .ok_or_else(|| format!("{}: (DependsOn, {d}) is not a page", e.path))?;
        deps.push(format!(
            "<a href=\"{}\">{}</a>",
            esc(&ctx.s.served.href(&p.url)),
            esc(&text(ctx.e(p.node), "Title"))
        ));
    }
    if !deps.is_empty() {
        parts.push(format!("Read first: {}", deps.join(", ")));
    }
    Ok(if parts.is_empty() {
        String::new()
    } else {
        format!("<p class=\"meta\">{}</p>", parts.join(" <span>·</span> "))
    })
}

/// One page. On a discussable page, comment entities under its blocks are drawn as threads, for `discussion`'s
/// viewer. Returns the HTML and, for a discussable page, its blocks.
pub fn render(
    s: &Site,
    page: &Page,
    discussion: &Discussion,
) -> Result<(String, Option<Blocks>), String> {
    let n = s.world.get(page.node);
    let discussable = n.has_tag("Discussable");
    let mut ctx = Ctx {
        s,
        page,
        cited: Vec::new(),
        discussable,
        blocks: Vec::new(),
        scripts: Vec::new(),
        discussion,
        say: None,
    };
    // The build sets `Audio {manifest}` on a page whose generated audio matches its spoken lines (gen/build.ts).
    let audio = match n.value("Audio").and_then(|v| v.get("manifest")) {
        Some(crate::world::Scalar::Str(m)) if !m.is_empty() => Some(m.clone()),
        _ => None,
    };
    if audio.is_some() {
        let lines = crate::spoken::lines(s, page)?;
        ctx.say = Some(lines.into_iter().map(|l| l.block).collect());
        ctx.script("listen");
    }
    let title = text(n, "Title");
    if title.is_empty() {
        return Err(format!("{}: a page needs a Title", n.path));
    }
    let mut body = children(page.node, &mut ctx)?;
    let refs = references_html(&ctx);
    body = if body.contains(REFERENCES) {
        js::replace_first(&body, REFERENCES, &refs)
    } else {
        body + &refs
    };
    let mut out_blocks = None;
    if discussable {
        ctx.script("comments");
        // The page's own comments, and those whose block is gone, are children of the page entity.
        body += &format!(
            "\n<section class=\"discussion\" data-block=\"{PAGE_BLOCK}\"><h2>Discussion</h2><p class=\"dim\">Comments on the page as a whole. To comment on one paragraph, use the button beside it.</p>{}</section>{}",
            comments::thread(
                PAGE_BLOCK, s.world, page.node, &page.doc, &page.url, discussion, s.served
            ),
            comments::sign_out_form(&page.url, discussion, s.served)
        );
        let mut blocks = vec![(PAGE_BLOCK.to_string(), title.clone())];
        for (k, v) in std::mem::take(&mut ctx.blocks) {
            match blocks.iter_mut().find(|(x, _)| *x == k) {
                Some((_, old)) => *old = v,
                None => blocks.push((k, v)),
            }
        }
        out_blocks = Some(Blocks { doc: page.doc.clone(), title: title.clone(), blocks });
    }
    let tab = match text(n, "Tab") {
        t if !t.is_empty() => t,
        _ if page.url == "/" => title.clone(),
        _ => format!("{title} · blaze"),
    };
    let description = prose(&text(n, "Description"));
    // Where the page lives, for search engines and link previews: origin + base + path (none without an origin).
    let canonical = if s.served.origin.is_empty() {
        String::new()
    } else {
        let at = esc(&format!("{}{}", s.served.origin, s.served.href(&page.url)));
        let og_description = if description.is_empty() {
            String::new()
        } else {
            format!("\n<meta property=\"og:description\" content=\"{}\" />", esc(&description))
        };
        format!(
            "<link rel=\"canonical\" href=\"{at}\" />
<meta property=\"og:type\" content=\"website\" />
<meta property=\"og:site_name\" content=\"blaze\" />
<meta property=\"og:title\" content=\"{}\" />
<meta property=\"og:url\" content=\"{at}\" />{og_description}",
            esc(&title)
        )
    };
    let description = if description.is_empty() {
        String::new()
    } else {
        format!("<meta name=\"description\" content=\"{}\" />", esc(&description))
    };
    let width = if n.has_tag("Narrow") {
        " narrow"
    } else if n.has_tag("Wide") {
        " wide"
    } else {
        ""
    };
    let scripts = ctx
        .scripts
        .iter()
        .map(|js| {
            format!(
                "<script type=\"module\" src=\"{}\"></script>",
                esc(&s.served.href(&format!("/js/{js}.js")))
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    // No widget has extra <head> markup, so that line of the TS template is always empty.
    let extra_head = "";
    let doc_attr =
        if discussable { format!(" data-doc=\"{}\"", esc(&page.doc)) } else { String::new() };
    let html = format!(
        "<!doctype html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\" />
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />
<title>{}</title>
{description}
{canonical}
<link rel=\"stylesheet\" href=\"{css}\" />
{extra_head}
{scripts}
</head>
<body>
{}
<main class=\"doc{width}\"{doc_attr}>
{}
<h1{}>{}</h1>
{}{}
{body}
</main>
</body>
</html>
",
        esc(&tab),
        topbar(page, s.nav, s.served),
        trail(&ctx),
        if audio.is_some() {
            format!(" data-say=\"{}\"", crate::spoken::INTRO)
        } else {
            String::new()
        },
        esc(&title),
        meta(&ctx)?,
        match &audio {
            // The player (client/listen.ts) builds its controls in here; without JS it stays empty.
            Some(m) =>
                format!("\n<div class=\"listen\" data-audio=\"{}\"></div>", esc(&s.served.href(m))),
            None => String::new(),
        },
        css = esc(&s.served.href("/site.css")),
    );
    Ok((html, out_blocks))
}

/// `.gen/discussable.json`'s shape: doc to title and blocks.
pub type Discussable = BTreeMap<String, Blocks>;
