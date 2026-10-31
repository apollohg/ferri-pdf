//! Malformed and hostile HTML and CSS always renders: invalid input recovers as the HTML and
//! CSS error handling rules say, and limits degrade the output instead of failing it.
mod common;

use common::{lossy_options, pdf_number_faults};
use ferri_pdf::render_html;
use std::fmt::Write;

/// Documents per run; each is small enough that the whole run stays within the normal suite.
const CASES: u64 = 48;
const FIRST_SEED: u64 = 0x5eed_f00d;
/// The golden-ratio increment, which spreads consecutive cases' seeds across the state space.
const SEED_STEP: u64 = 0x9e37_79b9_7f4a_7c15;
const MAX_ELEMENTS: usize = 24;
const MAX_RULES: usize = 10;
const MAX_DECLARATIONS: usize = 5;
/// Deep enough to cross the formatting, DOM and generated-content nesting limits.
const DEEP_NESTING: usize = 600;
/// One case in this many nests its content past the depth limits.
const DEEP_CASE_PERIOD: u64 = 8;

const TAGS: &[&str] = &[
    "div",
    "span",
    "p",
    "b",
    "i",
    "a",
    "table",
    "tr",
    "td",
    "th",
    "caption",
    "ul",
    "ol",
    "li",
    "pre",
    "h1",
    "h2",
    "section",
    "ruby",
    "rt",
    "rp",
    "select",
    "option",
    "input",
    "button",
    "textarea",
    "fieldset",
    "legend",
    "details",
    "summary",
    "math",
    "mi",
    "mo",
    "mn",
    "mfrac",
    "msqrt",
    "mroot",
    "msub",
    "mtable",
    "mtr",
    "mtd",
    "svg",
    "g",
    "rect",
    "circle",
    "text",
    "tspan",
    "path",
    "dl",
    "dt",
    "dd",
    "blockquote",
    "q",
    "sup",
    "sub",
    "br",
    "hr",
    "wbr",
    "template",
    "slot",
    "colgroup",
    "col",
    "thead",
    "tbody",
    "label",
    "progress",
    "meter",
];
/// Elements nested past the depth limits, and the context each nests in.
const DEEP_ELEMENTS: &[(&str, &str)] = &[
    ("", "div"),
    ("", "span"),
    ("", "li"),
    ("", "td"),
    ("", "b"),
    ("", "q"),
    ("<math>", "mrow"),
    ("<math>", "mfrac"),
    ("", "div><template shadowrootmode=open"),
];
const ATTRIBUTES: &[&str] = &[
    "id", "class", "lang", "dir", "colspan", "rowspan", "start", "value", "href", "width",
    "height", "d", "viewBox", "x", "y", "r", "span", "type", "hidden", "open", "title",
];
const PROPERTIES: &[&str] = &[
    "display",
    "position",
    "float",
    "width",
    "height",
    "margin",
    "padding",
    "border",
    "font-size",
    "line-height",
    "font-family",
    "font",
    "color",
    "background",
    "content",
    "counter-reset",
    "counter-increment",
    "counter-set",
    "string-set",
    "zoom",
    "box-shadow",
    "text-shadow",
    "transform",
    "columns",
    "column-count",
    "grid-template-columns",
    "grid-area",
    "flex",
    "order",
    "writing-mode",
    "direction",
    "text-transform",
    "letter-spacing",
    "word-spacing",
    "text-indent",
    "white-space",
    "list-style",
    "quotes",
    "break-before",
    "break-inside",
    "page",
    "footnote-display",
    "inset",
    "min-width",
    "max-height",
    "aspect-ratio",
    "overflow",
    "text-align",
    "vertical-align",
    "math-depth",
    "math-style",
    "fill",
    "stroke",
    "stroke-width",
    "opacity",
    "filter",
    "clip-path",
    "mask",
    "anchor-name",
    "position-anchor",
    "container-type",
    "scroll-initial-target",
    "initial-letter",
    "hyphens",
    "tab-size",
    "orphans",
    "widows",
    "table-layout",
    "border-collapse",
    "border-spacing",
    "counter-reset",
];
const VALUES: &[&str] = &[
    "0",
    "1",
    "-1",
    "1e38",
    "-1e38",
    "1e38px",
    "-1e38pt",
    "1e30pt",
    "1e-30pt",
    "100%",
    "10000%",
    "auto",
    "none",
    "inherit",
    "initial",
    "unset",
    "revert",
    "revert-layer",
    "block",
    "inline",
    "flex",
    "grid",
    "table-cell",
    "contents",
    "ruby",
    "list-item",
    "absolute",
    "fixed",
    "running(header)",
    "footnote",
    "left",
    "nearest",
    "calc(1e38px * 1e38)",
    "calc(1px / 0)",
    "calc(infinity * 1px)",
    "calc(-infinity * 1px)",
    "calc(NaN * 1px)",
    "calc(0 / 0 * 1px)",
    "calc(sqrt(-1) * 1px)",
    "clamp(1px, calc(infinity * 1px), 2px)",
    "max(1e38px, 1e38px) ",
    "var(--a)",
    "var(--b, 1px)",
    "var(--cycle)",
    "var(--missing)",
    "attr(title)",
    "attr(width type(<length>))",
    "counter(c)",
    "counters(c, '.')",
    "counter(c, unknown-style)",
    "target-counter(attr(href), page)",
    "target-counter('#nowhere', c)",
    "target-text(attr(href))",
    "string(header)",
    "content(first-letter)",
    "leader(dotted)",
    "element(header)",
    "\"text\" counter(page) \"/\" counter(pages)",
    "open-quote",
    "c 2147483647",
    "c -2147483648 d 999999999999",
    "repeat(100000, 1px)",
    "repeat(auto-fill, minmax(0, 1fr))",
    "1fr 1fr",
    "rotate(1e38deg) scale(1e38)",
    "translate(calc(infinity * 1px))",
    "url(missing.png)",
    "red",
    "rgb(1e38 -1e38 NaN)",
    "light-dark(red, blue)",
    "toggle(red, blue)",
    "if(media(print): 1px; else: 2px)",
    "1px solid",
    "0 0 1e30pt red",
    "1e30pt 0 blue inset",
    "vertical-rl",
    "upright",
    "math",
    "auto-add",
    "!important",
    "\\",
    "\"unterminated",
    "url(",
    "(((((",
    ")))",
    "{}",
    ";;",
    "@",
    "#",
    "\u{0}",
    "\u{feff}x",
];
const SELECTORS: &[&str] = &[
    "*",
    "div",
    "p span",
    "div > div > div",
    ":root",
    "::before",
    "::after",
    "::marker",
    "::first-line",
    "::first-letter",
    "::footnote-call",
    "::footnote-marker",
    "li::marker",
    "div::before",
    "p::first-letter",
    ":nth-child(2n+1)",
    ":nth-child(99999999999n)",
    ":not(:not(:not(:not(:not(:not(div))))))",
    ":is(:is(:is(:is(:is(div)))))",
    ":has(:has(div))",
    ":has(div div div div div div)",
    "div:has(+ p)",
    "[title]",
    "[class~=\"a\" i]",
    ":lang(en)",
    "a[href]",
    "td:nth-of-type(odd)",
    ":host",
    "::slotted(*)",
    "&",
    "div & span",
    ":unknown-pseudo",
    "div[",
    "!!!",
    "",
];
const AT_RULES: &[&str] = &[
    "@page { size: 1e38pt 1e38pt; margin: -1e38pt }",
    "@page { size: 0 0 }",
    "@page :first { @top-center { content: counter(page) string(header) } }",
    "@page { @bottom-right { content: element(header); width: 1e38pt } }",
    "@page wide { size: landscape; zoom: 1e20 }",
    "@media print { div { color: red } }",
    "@media (min-width: calc(1px/0)) { p { margin: 1e38pt } }",
    "@media garbage(((( { p { color: blue } }",
    "@supports (display: grid) and (not (garbage)) { div { display: grid } }",
    "@supports selector(:has(a)) { a { color: red } }",
    "@container (width > 1e38px) { span { display: none } }",
    "@container style(--a: 1) { div { color: green } }",
    "@layer a, b; @layer b { div { color: red } } @layer a { div { color: blue } }",
    "@property --a { syntax: '<length>'; inherits: false; initial-value: 1e38px }",
    "@property --b { syntax: '<garbage'; initial-value: x }",
    "@counter-style c { system: cyclic; symbols: ; }",
    "@counter-style d { system: extends d; }",
    "@counter-style e { system: fixed 2147483647; symbols: a b; }",
    "@font-feature-values Foo { @styleset { nice: 99999; } }",
    "@font-palette-values --p { font-family: Foo; override-colors: 99999 red; }",
    "@scope (div) to (span) { :scope { color: red } }",
    "@starting-style { div { color: red } }",
    "@position-try --t { top: anchor(--missing bottom); left: 1e38pt }",
    "@import url(missing.css) layer(x) supports(display: grid);",
    "@namespace svg url(http://www.w3.org/2000/svg);",
    "@charset \"utf-8\";",
    "@unknown-rule { whatever }",
    "@media",
    "@page {",
    "@footnote { float: bottom }",
];
const CUSTOM_PROPERTIES: &str = "--cycle: var(--cycle2); --cycle2: var(--cycle); --a: var(--b); --b: var(--a, 1px); --big: var(--big1) var(--big1) var(--big1); --big1: var(--big2) var(--big2) var(--big2); --big2: xxxxxxxxxxxxxxxxxxxx xxxxxxxxxxxxxxxxxxxx";

/// A deterministic xorshift generator, so a failing seed reproduces the same document.
struct Generator(u64);
impl Generator {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next() % bound as u64).expect("bound fits in usize")
    }
    fn pick<'a>(&mut self, choices: &[&'a str]) -> &'a str {
        choices[self.below(choices.len())]
    }
    fn chance(&mut self, period: usize) -> bool {
        self.below(period) == 0
    }

    fn declarations(&mut self) -> String {
        let mut text = String::new();
        if self.chance(3) {
            text.push_str(CUSTOM_PROPERTIES);
            text.push(';');
        }
        for _ in 0..=self.below(MAX_DECLARATIONS) {
            // Mostly single components, which more often survive parsing to be computed.
            let components = if self.chance(3) { 1 + self.below(3) } else { 1 };
            let value = (0..components)
                .map(|_| self.pick(VALUES))
                .collect::<Vec<_>>()
                .join(" ");
            let _ = write!(text, "{}:{value}", self.pick(PROPERTIES));
            if self.chance(6) {
                text.push_str(" !important");
            }
            text.push(';');
        }
        text
    }

    fn stylesheet(&mut self) -> String {
        let mut text = String::new();
        for _ in 0..self.below(MAX_RULES) {
            if self.chance(3) {
                text.push_str(self.pick(AT_RULES));
            } else {
                let selector = self.pick(SELECTORS);
                let declarations = self.declarations();
                if self.chance(5) {
                    let nested = self.pick(SELECTORS);
                    let inner = self.declarations();
                    let _ = write!(text, "{selector}{{{declarations} {nested}{{{inner}}}}}");
                } else {
                    let _ = write!(text, "{selector}{{{declarations}}}");
                }
            }
            text.push('\n');
        }
        text
    }

    fn element(&mut self, html: &mut String, open: &mut Vec<&'static str>) {
        match self.below(10) {
            0 if !open.is_empty() => {
                // Misnested or stray end tags.
                let tag = if self.chance(2) {
                    open.remove(self.below(open.len()))
                } else {
                    self.pick(TAGS)
                };
                let _ = write!(html, "</{tag}>");
            }
            1 => {
                let _ = write!(html, "text {} &amp; &bogus; &#x0; &#99999999;", self.next());
            }
            2 => html.push_str("<!-- unterminated comment"),
            3 => {
                let _ = write!(html, "<div style=\"{}\">", self.declarations());
            }
            _ => {
                let tag = self.pick(TAGS);
                let mut start = format!("<{tag}");
                for _ in 0..self.below(3) {
                    let attribute = self.pick(ATTRIBUTES);
                    let value = self.pick(VALUES).replace('"', "&quot;");
                    let _ = write!(start, " {attribute}=\"{value}\"");
                }
                if self.chance(3) {
                    let declarations = self.declarations().replace('"', "'");
                    let _ = write!(start, " style=\"{declarations}\"");
                }
                if self.chance(4) {
                    start.push_str(" href=\"#a\" id=\"a\" class=\"a\"");
                }
                html.push_str(&start);
                html.push('>');
                html.push('x');
                open.push(tag);
            }
        }
    }

    fn document(&mut self, deep: bool) -> String {
        let mut html = format!("<!doctype html><style>{}</style>", self.stylesheet());
        if self.chance(2) {
            let _ = write!(html, "<style>{}", self.stylesheet());
        }
        let mut open = Vec::new();
        for _ in 0..self.below(MAX_ELEMENTS) {
            self.element(&mut html, &mut open);
        }
        if deep {
            // Deep HTML and MathML; the end tags leave any foreign content the elements opened.
            html.push_str("</svg></math>");
            let (context, tag) = DEEP_ELEMENTS[self.below(DEEP_ELEMENTS.len())];
            html.push_str(context);
            html.push_str(
                &format!("<{tag} style=\"{}\">", self.declarations()).repeat(DEEP_NESTING),
            );
            html.push_str("deep");
        }
        html
    }
}

/// Lengths at and past the ends of what layout can represent.
const EXTREME_LENGTHS: &[&str] = &[
    "1e30px",
    "-1e30px",
    "3e38pt",
    "-3e38pt",
    "1e-30px",
    "0",
    "100000%",
    "-100000%",
    "calc(1e38px * 100)",
    "max-content",
];
/// Layout and paint declarations, with `{}` where each takes an extreme length.
const GEOMETRY: &[&str] = &[
    "width:{}",
    "height:{}",
    "min-height:{}",
    "max-width:{}",
    "margin:{}",
    "padding:{}",
    "border:{} solid red",
    "border-radius:{}",
    "outline:{} solid;outline-offset:{}",
    "letter-spacing:{}",
    "word-spacing:{}",
    "text-indent:{}",
    "font-size:{}",
    "line-height:{}",
    "inset:{}",
    "gap:{}",
    "column-width:{}",
    "column-gap:{}",
    "flex-basis:{}",
    "transform:translate({},{})",
    "transform:scale(1e30) rotate(45deg)",
    "transform:matrix3d(1e30,0,0,0,0,1e30,0,0,0,0,1,1e30,0,0,0,1e-30)",
    "perspective:{}",
    "filter:blur({})",
    "backdrop-filter:blur({})",
    "box-shadow:{} {} {} {} red",
    "text-shadow:{} {} {} red",
    "float:left;shape-outside:circle({})",
    "offset-path:circle({});offset-distance:100%",
    "mask:linear-gradient(red,transparent) 0 0/{} {}",
    "background:linear-gradient(red,blue) 0 0/{} {}",
    "grid-template-columns:repeat(1000,{})",
    "tab-size:{}",
    "aspect-ratio:1e30/1e-30",
    "zoom:1e30",
    "column-count:1000",
];
/// Page geometry declarations, with `{}` where each takes an extreme length; page areas
/// that leave no room for content are among them.
const PAGE_GEOMETRY: &[&str] = &[
    "size:{} {}",
    "size:100pt 0.01pt",
    "size:0.01pt 100pt",
    "margin:{}",
    "margin:50pt 0",
    "padding:{}",
    "border:{} solid red",
];
/// Formatting contexts and positioning schemes: the start of an opening tag whose `style`
/// attribute the generated declarations continue, and the markup closing it.
const CONTEXTS: &[(&str, &str)] = &[
    ("<div style=\"", "</div>"),
    ("<div style=\"display:flex;flex-wrap:wrap;", "</div>"),
    ("<div style=\"display:grid;", "</div>"),
    (
        "<table><tr><td colspan=1000 rowspan=65534 style=\"",
        "</table>",
    ),
    (
        "<div style=\"columns:3;column-fill:auto;height:50pt;",
        "</div>",
    ),
    ("<div style=\"float:left;", "</div>"),
    ("<div style=\"position:absolute;", "</div>"),
    ("<div style=\"position:fixed;", "</div>"),
    ("<div style=\"position:sticky;top:0;", "</div>"),
    ("<span style=\"display:inline-block;", "</span>"),
    ("<ruby style=\"", "<rt>r</ruby>"),
    ("<span style=\"float:footnote;", "</span>"),
    ("<div style=\"writing-mode:vertical-rl;", "</div>"),
    (
        "<div style=\"anchor-name:--a;position:absolute;position-anchor:--a;top:anchor(bottom);",
        "</div>",
    ),
    ("<div style=\"container-type:inline-size;", "</div>"),
    ("<div style=\"break-after:page;", "</div>"),
];
const MAX_BOXES: usize = 8;

impl Generator {
    fn geometry_declarations(&mut self, choices: &[&str]) -> String {
        (0..=self.below(MAX_DECLARATIONS))
            .map(|_| {
                let mut declaration = self.pick(choices).to_owned();
                while let Some(slot) = declaration.find("{}") {
                    let length = self.pick(EXTREME_LENGTHS);
                    declaration.replace_range(slot..slot + 2, length);
                }
                declaration + ";"
            })
            .collect()
    }

    /// Nested and sibling boxes with extreme geometry in every formatting context, around
    /// text that breaks across lines, on pages of extreme geometry.
    fn geometry_document(&mut self) -> String {
        let mut html = String::from("<!doctype html>");
        if self.chance(2) {
            let page = self.geometry_declarations(PAGE_GEOMETRY);
            let _ = write!(html, "<style>@page{{{page}}}</style>");
        }
        let mut closing = Vec::new();
        for _ in 0..=self.below(MAX_BOXES) {
            if self.chance(3)
                && let Some(close) = closing.pop()
            {
                html.push_str(close);
            }
            let (open, close) = CONTEXTS[self.below(CONTEXTS.len())];
            let _ = write!(
                html,
                "{open}{}\">words that wrap across lines ",
                self.geometry_declarations(GEOMETRY)
            );
            closing.push(close);
        }
        html
    }
}

/// The pinned grammar the engine's property table compiles from (`build.rs`).
const GRAMMAR_DIRECTORY: &str = "spec/css";
/// Values of each numeric type past what layout, painting and PDF represent: overflowing,
/// infinite and NaN calculations (CSS Values 4 §10.9), denormal magnitudes and integers past
/// 32 bits.
const NUMERIC_EXTREMES: &[(&str, &[&str])] = &[
    (
        "length",
        &[
            "3e38px",
            "-3e38px",
            "calc(1e30px*1e30)",
            "calc(-1e30px*1e30)",
            "1e-38px",
            "calc(infinity*1px)",
            "calc(-infinity*1px)",
            "calc(NaN*1px)",
        ],
    ),
    (
        "percentage",
        &[
            "3e38%",
            "-3e38%",
            "calc(1e30%*1e30)",
            "1e-38%",
            "calc(infinity*1%)",
            "calc(NaN*1%)",
        ],
    ),
    (
        "number",
        &[
            "3e38",
            "-3e38",
            "calc(1e30*1e30)",
            "calc(-1e30*1e30)",
            "1e-38",
            "calc(infinity)",
            "calc(-infinity)",
            "calc(NaN)",
        ],
    ),
    (
        "integer",
        &[
            "2147483647",
            "-2147483648",
            "99999999999999999999",
            "-99999999999999999999",
            "calc(infinity)",
            "calc(-infinity)",
            "calc(NaN)",
        ],
    ),
    (
        "angle",
        &[
            "3e38deg",
            "-3e38deg",
            "calc(1e30deg*1e30)",
            "1e-38deg",
            "calc(infinity*1deg)",
            "calc(NaN*1deg)",
        ],
    ),
];
/// Where the generated declarations apply: `{}` stands for them in each context's markup.
const VALUE_CONTEXTS: &[(&str, &str)] = &[
    (
        "block",
        "<div style=\"{}\">block text that wraps across lines</div>",
    ),
    (
        "inline text",
        "<p>before <span style=\"{}\">inline text that wraps</span> after</p>",
    ),
    (
        "table cell",
        "<table><tr><td style=\"{}\">cell</td><td>next</td></tr><tr><td>row</td></tr></table>",
    ),
    (
        "flex item",
        "<div style=\"display:flex;flex-wrap:wrap\"><div style=\"{}\">flex item</div><div>next</div></div>",
    ),
    (
        "grid item",
        "<div style=\"display:grid;grid-template-columns:1fr auto\"><div style=\"{}\">grid item</div><div>next</div></div>",
    ),
    (
        "float",
        "<div style=\"float:left;{}\">float</div><p>text flowing beside the float</p>",
    ),
    (
        "abspos",
        "<div style=\"position:relative\"><div style=\"position:absolute;{}\">positioned</div>in flow</div>",
    ),
    (
        "multicol",
        "<div style=\"columns:2;{}\">multi-column text that wraps across lines and columns</div>",
    ),
    (
        "footnote",
        "<p>body<span style=\"float:footnote;{}\">footnote text</span></p>",
    ),
    (
        "footnote area",
        "<style>@page{@footnote{{}}}</style><p>body<span style=\"float:footnote\">footnote text</span></p>",
    ),
    (
        "page-margin box",
        "<style>@page{@top-center{content:'head';{}}@bottom-center{content:counter(page)}}</style><p>body</p>",
    ),
    (
        "running element",
        "<style>@page{@top-center{content:element(run)}}</style><div style=\"position:running(run);{}\">running</div><p>body</p>",
    ),
    ("page box", "<style>@page{{}}</style><p>body</p>"),
    (
        "list marker",
        "<style>li::marker{{}}</style><ol><li>item</li><li style=\"{}\">styled item</li></ol>",
    ),
    (
        "inline SVG",
        "<svg width=100 height=100 style=\"{}\"><rect width=50 height=50 style=\"{}\"/><circle cx=50 cy=50 r=20 style=\"{}\"/><text x=5 y=20 style=\"{}\">svg</text></svg>",
    ),
    (
        "MathML",
        "<math style=\"{}\"><mfrac style=\"{}\"><mi>x</mi><msqrt><mn style=\"{}\">2</mn></msqrt></mfrac></math>",
    ),
    (
        "form control",
        "<input value=input style=\"{}\"><select style=\"{}\"><option>option</select><button style=\"{}\">button</button><progress value=1 max=2 style=\"{}\"></progress>",
    ),
    (
        "link and destination",
        "<h1 style=\"{}\">bookmarked</h1><p><a href=#target style=\"{}\">link</a></p><div id=target style=\"{}\">target</div>",
    ),
    (
        "first letter",
        "<style>p::first-letter{{}}</style><p>First letter of a paragraph</p>",
    ),
];
const MAX_COMPONENTS: usize = 4;
const MAX_FUNCTION_ARGUMENTS: usize = 3;
/// Extreme-value documents per run: each is small, and the run samples properties, values
/// and contexts rather than taking their product.
const EXTREME_VALUE_CASES: u64 = 128;

/// How a property's grammar takes numeric values: bare, or as the arguments of a functional
/// notation; each with the [`NUMERIC_EXTREMES`] types it accepts there.
struct NumericForm {
    function: Option<String>,
    types: Vec<usize>,
}
struct NumericProperty {
    name: String,
    forms: Vec<NumericForm>,
}

/// Property and production grammars: the pinned mdn-data with the errata and extensions
/// `build.rs` applies.
struct Grammar {
    properties: std::collections::BTreeMap<String, String>,
    syntaxes: std::collections::BTreeMap<String, String>,
}
impl Grammar {
    fn load() -> Self {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(GRAMMAR_DIRECTORY);
        let read = |file: &str| -> serde_json::Value {
            let path = directory.join(file);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            serde_json::from_str(&text)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        };
        let syntaxes_of = |value: &serde_json::Value| -> Vec<(String, String)> {
            value
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(name, entry)| {
                    Some((name.clone(), entry.get("syntax")?.as_str()?.to_owned()))
                })
                .collect()
        };
        let errata = read("errata.json");
        let mut grammar = Self {
            properties: syntaxes_of(&read("mdn-data/properties.json"))
                .into_iter()
                .collect(),
            syntaxes: syntaxes_of(&read("mdn-data/syntaxes.json"))
                .into_iter()
                .collect(),
        };
        grammar.syntaxes.extend(syntaxes_of(&errata["syntaxes"]));
        grammar
            .properties
            .extend(syntaxes_of(&errata["properties"]));
        grammar.properties.extend(syntaxes_of(&errata["additions"]));
        grammar
            .properties
            .extend(syntaxes_of(&read("extensions.json")["properties"]));
        grammar
    }

    /// Collects the numeric types `syntax` accepts outside functional notations, and the
    /// functional-notation productions it references.
    fn collect(
        &self,
        syntax: &str,
        seen: &mut std::collections::HashSet<String>,
        types: &mut std::collections::BTreeSet<usize>,
        functions: &mut std::collections::BTreeSet<String>,
    ) {
        let mut rest = syntax;
        while let Some(open) = rest.find('<') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('>') else { break };
            rest = &after[close + 1..];
            let name = after[..close]
                .trim_matches('\'')
                .split([' ', '['])
                .next()
                .unwrap_or_default()
                .trim_matches('\'');
            if let Some(index) = NUMERIC_EXTREMES.iter().position(|(kind, _)| *kind == name) {
                types.insert(index);
            } else if name.ends_with("()") {
                functions.insert(name.to_owned());
            } else if seen.insert(name.to_owned())
                && let Some(syntax) = self.syntaxes.get(name).or(self.properties.get(name))
            {
                self.collect(syntax, seen, types, functions);
            }
        }
    }

    /// Every property whose grammar accepts a numeric type, with how it takes each.
    fn numeric_properties(&self) -> Vec<NumericProperty> {
        let numeric_types = |syntax: &str| {
            let (mut types, mut functions) = Default::default();
            self.collect(syntax, &mut Default::default(), &mut types, &mut functions);
            (types, functions)
        };
        self.properties
            .iter()
            .filter(|(name, _)| !name.starts_with("--"))
            .filter_map(|(name, syntax)| {
                let (types, functions) = numeric_types(syntax);
                let bare = (!types.is_empty()).then(|| NumericForm {
                    function: None,
                    types: types.into_iter().collect(),
                });
                let forms: Vec<_> = bare
                    .into_iter()
                    .chain(functions.into_iter().filter_map(|function| {
                        let (types, _) = numeric_types(self.syntaxes.get(&function)?);
                        (!types.is_empty()).then(|| NumericForm {
                            function: Some(function.trim_end_matches("()").to_owned()),
                            types: types.into_iter().collect(),
                        })
                    }))
                    .collect();
                (!forms.is_empty()).then(|| NumericProperty {
                    name: name.clone(),
                    forms,
                })
            })
            .collect()
    }
}

impl Generator {
    fn extreme(&mut self, types: &[usize]) -> &'static str {
        let (_, values) = NUMERIC_EXTREMES[types[self.below(types.len())]];
        self.pick(values)
    }

    /// One declaration of `property` whose numeric components all take extreme values.
    fn extreme_declaration(&mut self, property: &NumericProperty) -> String {
        let components: Vec<_> = (0..=self.below(MAX_COMPONENTS))
            .map(|_| {
                let form = &property.forms[self.below(property.forms.len())];
                match &form.function {
                    None => self.extreme(&form.types).to_owned(),
                    Some(function) => {
                        let separator = if self.chance(2) { ", " } else { " " };
                        let arguments: Vec<_> = (0..=self.below(MAX_FUNCTION_ARGUMENTS))
                            .map(|_| self.extreme(&form.types))
                            .collect();
                        format!("{function}({})", arguments.join(separator))
                    }
                }
            })
            .collect();
        format!("{}:{}", property.name, components.join(" "))
    }

    /// Declarations giving numeric properties extreme values, in one sampled context.
    fn extreme_value_document(&mut self, properties: &[NumericProperty]) -> String {
        let declarations: Vec<_> = (0..=self.below(MAX_DECLARATIONS))
            .map(|_| {
                let property = &properties[self.below(properties.len())];
                self.extreme_declaration(property)
            })
            .collect();
        let (context, markup) = VALUE_CONTEXTS[self.below(VALUE_CONTEXTS.len())];
        format!(
            "<!doctype html><!-- {context} -->{}",
            markup.replace("{}", &declarations.join(";"))
        )
    }
}

/// Renders the documents `generate` makes for each of `cases` from its seed, collecting every
/// failure to render a valid PDF whose numbers are finite and within the PDF limit.
fn assert_generated_documents_render(
    kind: &str,
    cases: u64,
    generate: impl Fn(&mut Generator, u64) -> String,
) {
    let mut failures = Vec::new();
    for case in 0..cases {
        let seed = FIRST_SEED.wrapping_add(case.wrapping_mul(SEED_STEP)) | 1;
        let html = generate(&mut Generator(seed), case);
        let result = std::panic::catch_unwind(|| render_html(&html, &lossy_options()));
        match result {
            Ok(Ok(output)) => {
                assert!(
                    output.page_count > 0 && output.pdf.starts_with(b"%PDF-"),
                    "seed {seed:#x}: rendering produced no PDF pages ({} pages)",
                    output.page_count
                );
                let faults = pdf_number_faults(&output.pdf);
                if !faults.is_empty() {
                    failures.push(format!(
                        "seed {seed:#x}: invalid PDF:\n  {}\n{html}",
                        faults.join("\n  ")
                    ));
                }
            }
            Ok(Err(error)) => failures.push(format!("seed {seed:#x}: error: {error}\n{html}")),
            Err(_) => failures.push(format!("seed {seed:#x}: panicked\n{html}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {cases} hostile {kind} failed to render:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn generated_hostile_documents_always_render() {
    assert_generated_documents_render("documents", CASES, |generator, case| {
        generator.document(case % DEEP_CASE_PERIOD == 0)
    });
}

#[test]
fn generated_hostile_geometry_always_renders() {
    assert_generated_documents_render("geometries", CASES, |generator, _| {
        generator.geometry_document()
    });
}

#[test]
fn generated_extreme_numeric_values_render_valid_pdfs() {
    let properties = Grammar::load().numeric_properties();
    // Each numeric type is reached, bare and through functional notations, so the loader
    // follows the grammar's productions.
    for (name, kind, function) in [
        ("width", "length", None),
        ("opacity", "number", None),
        ("z-index", "integer", None),
        ("rotate", "angle", None),
        ("transform", "angle", Some("rotate")),
        ("color", "percentage", Some("rgb")),
    ] {
        let kind = NUMERIC_EXTREMES
            .iter()
            .position(|(extreme, _)| *extreme == kind);
        assert!(
            properties.iter().any(|property| property.name == name
                && property
                    .forms
                    .iter()
                    .any(|form| form.function.as_deref() == function
                        && kind.is_some_and(|kind| form.types.contains(&kind)))),
            "the grammar gives {name} a numeric form {function:?} taking {kind:?}"
        );
    }
    assert_generated_documents_render("extreme values", EXTREME_VALUE_CASES, |generator, _| {
        generator.extreme_value_document(&properties)
    });
}
