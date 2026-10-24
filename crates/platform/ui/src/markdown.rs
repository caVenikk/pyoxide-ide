//! Markdown as GPUI elements (ADR 0010): the documentation that language
//! services send for quick documentation, hovers and completion items.
//!
//! CommonMark (with strikethrough) is parsed by `pulldown-cmark` into a
//! small block model, [`Markdown`], which keeps what documentation uses:
//! paragraphs, headings, emphasis, inline code, links, lists, block quotes,
//! rules and code blocks. The code of a code block is coloured by a
//! [`CodeLexer`] the caller passes (the language of a fence is the lexer's
//! business; the platform knows none). Images show their text, HTML its
//! text without tags, tables their cells as text.
//!
//! Drawing ([`Markdown::render`]) uses the theme: prose in the UI font,
//! code in the editor font with the colours of the theme's text roles,
//! links in the link colour (a click opens a web link in the browser).

use std::ops::Range;

use gpui::{
    AnyElement, App, ElementId, Font, FontStyle, FontWeight, Hsla, IntoElement, ParentElement,
    SharedString, StrikethroughStyle, Styled, StyledText, TextRun, div, font, px,
};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde_json::{Value as Json, json};
use themes::{TextRole, UiRole};

use crate::theme::ThemeExt;

/// Colours code: role spans of a text as byte ranges, sorted and not
/// overlapping (parts without a span keep the default text colour).
pub trait CodeLexer {
    /// The spans of `code` in `language`, the info string of its fence;
    /// `None` for a block without one (the language of the documented
    /// code).
    fn lex(&self, language: Option<&str>, code: &str) -> Vec<(Range<usize>, TextRole)>;
}

/// How a part of inline text is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    /// Inline code: the editor font.
    pub code: bool,
}

impl Style {
    fn is_plain(self) -> bool {
        self == Style::default()
    }

    fn name(self) -> String {
        let parts: Vec<&str> = [
            (self.bold, "bold"),
            (self.italic, "italic"),
            (self.strikethrough, "strikethrough"),
            (self.code, "code"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
        parts.join("+")
    }
}

/// Text with styled parts and links: the content of a paragraph or a
/// heading.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Inline {
    pub text: String,
    /// Byte ranges of `text` with their style, sorted and not overlapping;
    /// the parts not listed are plain.
    pub styles: Vec<(Range<usize>, Style)>,
    /// Byte ranges of `text` that are links, sorted, with their targets.
    pub links: Vec<(Range<usize>, String)>,
}

impl Inline {
    /// Plain text.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    /// Appends text in a style.
    pub fn push(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(text);
        if style.is_plain() {
            return;
        }
        let end = self.text.len();
        match self.styles.last_mut() {
            Some((range, last)) if range.end == start && *last == style => range.end = end,
            _ => self.styles.push((start..end, style)),
        }
    }

    /// Appends a link with its text in a style.
    pub fn push_link(&mut self, text: &str, href: &str, style: Style) {
        let start = self.text.len();
        self.push(text, style);
        if self.text.len() > start {
            self.links.push((start..self.text.len(), href.to_owned()));
        }
    }

    fn trim_end(&mut self) {
        let len = self.text.trim_end().len();
        self.text.truncate(len);
        self.styles.retain_mut(|(r, _)| {
            r.end = r.end.min(len);
            r.start < r.end
        });
        self.links.retain_mut(|(r, _)| {
            r.end = r.end.min(len);
            r.start < r.end
        });
    }

    fn to_json(&self) -> Json {
        json!({
            "text": self.text,
            "styles": self.styles.iter().map(|(r, s)| json!({
                "range": [r.start, r.end],
                "style": s.name(),
            })).collect::<Vec<_>>(),
            "links": self.links.iter().map(|(r, href)| json!({
                "range": [r.start, r.end],
                "href": href,
            })).collect::<Vec<_>>(),
        })
    }
}

/// A block of code with the colours of its tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code {
    /// The info string of its fence (`python`); `None` without one.
    pub language: Option<String>,
    /// The code without its last line break.
    pub text: String,
    /// Role spans of `text` as byte ranges, sorted and not overlapping.
    pub spans: Vec<(Range<usize>, TextRole)>,
}

impl Code {
    /// A code block coloured by `lexer`.
    pub fn new(language: Option<String>, text: String, lexer: Option<&dyn CodeLexer>) -> Self {
        let mut spans = lexer
            .map(|l| l.lex(language.as_deref(), &text))
            .unwrap_or_default();
        // Keep only spans that fit the text, in order, on char boundaries.
        let mut end = 0;
        spans.retain(|(r, _)| {
            let fits = r.start >= end
                && r.start < r.end
                && r.end <= text.len()
                && text.is_char_boundary(r.start)
                && text.is_char_boundary(r.end);
            if fits {
                end = r.end;
            }
            fits
        });
        Self {
            language,
            text,
            spans,
        }
    }

    fn to_json(&self) -> Json {
        json!({
            "type": "code",
            "language": self.language,
            "text": self.text,
            "spans": self.spans.iter().map(|(r, role)| json!({
                "range": [r.start, r.end],
                "role": role.name(),
            })).collect::<Vec<_>>(),
        })
    }
}

/// A block of a Markdown document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(Inline),
    Heading {
        /// 1 to 6.
        level: u8,
        text: Inline,
    },
    Code(Code),
    List {
        /// The number of the first item of a numbered list; `None` for
        /// bullets.
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    Quote(Vec<Block>),
    Rule,
}

impl Block {
    fn to_json(&self) -> Json {
        match self {
            Block::Paragraph(text) => {
                let mut j = text.to_json();
                j["type"] = "paragraph".into();
                j
            }
            Block::Heading { level, text } => {
                let mut j = text.to_json();
                j["type"] = "heading".into();
                j["level"] = (*level).into();
                j
            }
            Block::Code(code) => code.to_json(),
            Block::List { start, items } => json!({
                "type": "list",
                "start": start,
                "items": items.iter().map(|blocks| blocks_json(blocks)).collect::<Vec<_>>(),
            }),
            Block::Quote(blocks) => json!({ "type": "quote", "blocks": blocks_json(blocks) }),
            Block::Rule => json!({ "type": "rule" }),
        }
    }
}

fn blocks_json(blocks: &[Block]) -> Json {
    Json::Array(blocks.iter().map(Block::to_json).collect())
}

/// A parsed Markdown document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Markdown {
    pub blocks: Vec<Block>,
}

impl Markdown {
    /// Parses CommonMark; code blocks are coloured by `lexer`.
    pub fn parse(text: &str, lexer: Option<&dyn CodeLexer>) -> Self {
        let mut builder = Builder::new(lexer);
        for event in Parser::new_ext(text, Options::ENABLE_STRIKETHROUGH) {
            builder.event(event);
        }
        builder.finish()
    }

    /// Plain text: paragraphs at blank lines, the line breaks inside one
    /// kept.
    pub fn plain(text: &str) -> Self {
        let text = text.replace("\r\n", "\n");
        let blocks = text
            .split("\n\n")
            .map(|p| p.trim_matches('\n'))
            .filter(|p| !p.trim().is_empty())
            .map(|p| Block::Paragraph(Inline::plain(p)))
            .collect();
        Self { blocks }
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// About how many lines the document takes: lines of text wrapped at
    /// `per_line` chars, and lines of code (to size a popup before it is
    /// drawn).
    pub fn line_counts(&self, per_line: usize) -> (usize, usize) {
        fn wrapped(text: &str, per_line: usize) -> usize {
            text.lines()
                .map(|l| l.chars().count().div_ceil(per_line.max(1)).max(1))
                .sum()
        }
        fn count(blocks: &[Block], per_line: usize) -> (usize, usize) {
            blocks
                .iter()
                .fold((0, 0), |(text, code), block| match block {
                    Block::Paragraph(t) | Block::Heading { text: t, .. } => {
                        (text + wrapped(&t.text, per_line), code)
                    }
                    Block::Code(c) => (text, code + c.text.lines().count().max(1)),
                    Block::List { items, .. } => {
                        let (t, c) = items.iter().fold((0, 0), |(t, c), item| {
                            let (it, ic) = count(item, per_line.saturating_sub(3));
                            (t + it, c + ic)
                        });
                        (text + t, code + c)
                    }
                    Block::Quote(blocks) => {
                        let (t, c) = count(blocks, per_line.saturating_sub(2));
                        (text + t, code + c)
                    }
                    Block::Rule => (text, code),
                })
        }
        count(&self.blocks, per_line)
    }

    /// The block tree as JSON (for tests and state queries).
    pub fn to_json(&self) -> Json {
        blocks_json(&self.blocks)
    }

    /// The blocks as elements, one under the other. `id` makes the ids of
    /// the interactive parts (links) unique in the window.
    pub fn render(&self, id: &str, cx: &App) -> AnyElement {
        let mut ids = Ids::new(id);
        render_blocks(&self.blocks, &mut ids, cx).into_any_element()
    }
}

// ---- parsing ---------------------------------------------------------------

/// A block being read that holds other blocks.
enum Container {
    Root(Vec<Block>),
    Quote(Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    Item(Vec<Block>),
}

impl Container {
    fn push(&mut self, block: Block) {
        match self {
            Container::Root(blocks) | Container::Quote(blocks) | Container::Item(blocks) => {
                blocks.push(block)
            }
            // Only items go into a list; anything else becomes one.
            Container::List { items, .. } => items.push(vec![block]),
        }
    }
}

enum InlineKind {
    Paragraph,
    Heading(u8),
}

struct Builder<'a> {
    lexer: Option<&'a dyn CodeLexer>,
    stack: Vec<Container>,
    /// The paragraph or heading being read.
    inline: Option<(InlineKind, Inline)>,
    /// The code block being read: its language and text.
    code: Option<(Option<String>, String)>,
    bold: u32,
    italic: u32,
    strikethrough: u32,
    /// Open links: target and start in the inline text.
    links: Vec<(String, usize)>,
    /// Inside an HTML block.
    html: bool,
}

impl<'a> Builder<'a> {
    fn new(lexer: Option<&'a dyn CodeLexer>) -> Self {
        Self {
            lexer,
            stack: vec![Container::Root(Vec::new())],
            inline: None,
            code: None,
            bold: 0,
            italic: 0,
            strikethrough: 0,
            links: Vec::new(),
            html: false,
        }
    }

    fn style(&self) -> Style {
        Style {
            bold: self.bold > 0,
            italic: self.italic > 0,
            strikethrough: self.strikethrough > 0,
            code: false,
        }
    }

    fn push_block(&mut self, block: Block) {
        if let Some(container) = self.stack.last_mut() {
            container.push(block);
        }
    }

    /// Ends the paragraph or heading being read.
    fn flush(&mut self) {
        let Some((kind, mut inline)) = self.inline.take() else {
            return;
        };
        inline.trim_end();
        if inline.text.trim().is_empty() {
            return;
        }
        self.push_block(match kind {
            InlineKind::Paragraph => Block::Paragraph(inline),
            InlineKind::Heading(level) => Block::Heading {
                level,
                text: inline,
            },
        });
    }

    /// Inline content goes to the paragraph being read, or to a new one
    /// (list items of a tight list have no paragraph of their own).
    fn inline(&mut self) -> &mut Inline {
        &mut self
            .inline
            .get_or_insert_with(|| (InlineKind::Paragraph, Inline::default()))
            .1
    }

    fn text(&mut self, text: &str, style: Style) {
        if let Some((_, code)) = &mut self.code {
            code.push_str(text);
            return;
        }
        self.inline().push(text, style);
    }

    fn open(&mut self, container: Container) {
        self.flush();
        self.stack.push(container);
    }

    /// Closes the innermost container and adds what it read to its parent.
    fn close(&mut self) {
        self.flush();
        if self.stack.len() < 2 {
            return;
        }
        let block = match self.stack.pop() {
            Some(Container::Quote(blocks)) => Block::Quote(blocks),
            Some(Container::List { start, items }) => Block::List { start, items },
            Some(Container::Item(blocks)) => {
                match self.stack.last_mut() {
                    Some(Container::List { items, .. }) => items.push(blocks),
                    Some(other) => blocks.into_iter().for_each(|b| other.push(b)),
                    None => {}
                }
                return;
            }
            Some(Container::Root(blocks)) => Block::Quote(blocks),
            None => return,
        };
        self.push_block(block);
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => {
                if self.html {
                    let text = strip_tags(&text);
                    self.text(&text, Style::default());
                } else if self.code.is_some() {
                    self.text(&text, Style::default());
                } else {
                    // Prose collapses runs of spaces, as the reference's
                    // HTML documentation shows `contexts.  See` as
                    // `contexts. See`.
                    let style = self.style();
                    let after_space = self
                        .inline
                        .as_ref()
                        .is_some_and(|(_, i)| i.text.ends_with(' '));
                    self.text(&collapse_spaces(&text, after_space), style);
                }
            }
            Event::Code(text) => {
                let style = Style {
                    code: true,
                    ..self.style()
                };
                self.text(&text, style);
            }
            Event::InlineMath(text) | Event::DisplayMath(text) => {
                let style = self.style();
                self.text(&text, style);
            }
            Event::SoftBreak => {
                let style = self.style();
                self.text(" ", style);
            }
            Event::HardBreak => self.text("\n", Style::default()),
            Event::Html(html) | Event::InlineHtml(html) => {
                if is_line_break(&html) {
                    self.text("\n", Style::default());
                } else if self.html {
                    let text = strip_tags(&html);
                    // Text starts at the first visible char of the block.
                    let empty = self.inline.as_ref().is_none_or(|(_, i)| i.text.is_empty());
                    let text = if empty { text.trim_start() } else { &text };
                    self.text(text, Style::default());
                }
            }
            Event::Rule => {
                self.flush();
                self.push_block(Block::Rule);
            }
            Event::TaskListMarker(done) => {
                self.text(if done { "[x] " } else { "[ ] " }, Style::default())
            }
            Event::FootnoteReference(name) => {
                let style = self.style();
                self.text(&format!("[{name}]"), style);
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                self.flush();
                self.inline = Some((InlineKind::Paragraph, Inline::default()));
            }
            Tag::Heading { level, .. } => {
                self.flush();
                self.inline = Some((InlineKind::Heading(heading_level(level)), Inline::default()));
            }
            Tag::BlockQuote(_) => self.open(Container::Quote(Vec::new())),
            Tag::CodeBlock(kind) => {
                self.flush();
                let language = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .filter(|l| !l.is_empty())
                        .map(str::to_owned),
                    CodeBlockKind::Indented => None,
                };
                self.code = Some((language, String::new()));
            }
            Tag::HtmlBlock => {
                self.flush();
                self.html = true;
            }
            Tag::List(start) => self.open(Container::List {
                start,
                items: Vec::new(),
            }),
            Tag::Item => self.open(Container::Item(Vec::new())),
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strikethrough += 1,
            Tag::Link { dest_url, .. } => {
                let start = self.inline().text.len();
                self.links.push((dest_url.to_string(), start));
            }
            // Table cells and the rest: their text, in the paragraph being
            // read.
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) => self.flush(),
            TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Item => self.close(),
            TagEnd::CodeBlock => {
                if let Some((language, mut text)) = self.code.take() {
                    if text.ends_with('\n') {
                        text.pop();
                    }
                    self.push_block(Block::Code(Code::new(language, text, self.lexer)));
                }
            }
            TagEnd::HtmlBlock => {
                self.html = false;
                self.flush();
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strikethrough = self.strikethrough.saturating_sub(1),
            TagEnd::Link => {
                if let Some((href, start)) = self.links.pop() {
                    let inline = self.inline();
                    if inline.text.len() > start {
                        let range = start..inline.text.len();
                        inline.links.push((range, href));
                        inline.links.sort_by_key(|(r, _)| r.start);
                    }
                }
            }
            TagEnd::TableRow | TagEnd::TableHead => self.text("\n", Style::default()),
            TagEnd::TableCell => self.text("  ", Style::default()),
            _ => {}
        }
    }

    fn finish(mut self) -> Markdown {
        self.flush();
        while self.stack.len() > 1 {
            self.close();
        }
        match self.stack.pop() {
            Some(Container::Root(blocks)) => Markdown { blocks },
            _ => Markdown::default(),
        }
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    level as u8
}

fn is_line_break(html: &str) -> bool {
    let tag: String = html
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    matches!(tag.as_str(), "<br>" | "<br/>")
}

/// Text of HTML without its tags.
/// Prose with each run of spaces and tabs as one space; a leading one goes
/// when the text before ends with a space.
fn collapse_spaces(text: &str, after_space: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = after_space;
    for c in text.chars() {
        if c == ' ' || c == '\t' {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    out
}

fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

// ---- drawing ---------------------------------------------------------------

/// Room between blocks.
const BLOCK_GAP: f32 = 6.0;
/// Width of the column of list markers.
const BULLET_WIDTH: f32 = 16.0;
const NUMBER_WIDTH: f32 = 24.0;

/// Element ids of interactive parts, unique within one document.
struct Ids {
    prefix: String,
    next: usize,
}

impl Ids {
    fn new(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_owned(),
            next: 0,
        }
    }

    fn next(&mut self) -> ElementId {
        self.next += 1;
        ElementId::Name(format!("{}-{}", self.prefix, self.next).into())
    }
}

fn render_blocks(blocks: &[Block], ids: &mut Ids, cx: &App) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(BLOCK_GAP))
        .children(blocks.iter().map(|b| render_block(b, ids, cx)))
}

fn render_block(block: &Block, ids: &mut Ids, cx: &App) -> AnyElement {
    let theme = cx.theme();
    match block {
        Block::Paragraph(text) => render_inline_with(text, ids.next(), FontWeight::NORMAL, cx),
        Block::Heading { level, text } => {
            let scale = match level {
                1 => 1.3,
                2 => 1.15,
                _ => 1.0,
            };
            div()
                .text_size(theme.ui_size() * scale)
                .font_weight(FontWeight::BOLD)
                .child(render_inline_with(text, ids.next(), FontWeight::BOLD, cx))
                .into_any_element()
        }
        Block::Code(code) => div()
            .px(px(8.0))
            .py(px(6.0))
            .rounded(px(4.0))
            .bg(theme.color(UiRole::SurfaceAlt))
            .child(render_code(code, cx))
            .into_any_element(),
        Block::List { start, items } => {
            let width = if start.is_some() {
                NUMBER_WIDTH
            } else {
                BULLET_WIDTH
            };
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .children(items.iter().enumerate().map(|(i, blocks)| {
                    let marker = match start {
                        Some(first) => format!("{}.", first + i as u64),
                        None => "•".to_owned(),
                    };
                    div()
                        .flex()
                        .flex_row()
                        .child(div().flex_none().w(px(width)).child(marker))
                        .child(render_blocks(blocks, ids, cx).flex_1().min_w(px(0.0)))
                }))
                .into_any_element()
        }
        Block::Quote(blocks) => div()
            .pl(px(8.0))
            .border_l_2()
            .border_color(theme.color(UiRole::Divider))
            .text_color(theme.color(UiRole::TextMuted))
            .child(render_blocks(blocks, ids, cx))
            .into_any_element(),
        Block::Rule => div()
            .w_full()
            .h(px(1.0))
            .bg(theme.color(UiRole::Divider))
            .into_any_element(),
    }
}

/// Inline text: styled runs in the UI font, links clickable (`id` is the
/// element id of the clickable text).
pub fn render_inline(text: &Inline, id: impl Into<ElementId>, cx: &App) -> AnyElement {
    render_inline_with(text, id.into(), FontWeight::NORMAL, cx)
}

fn render_inline_with(text: &Inline, id: ElementId, weight: FontWeight, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let colors = InlineColors {
        text: theme.color(UiRole::Text),
        link: theme.color(UiRole::TextLink),
    };
    let ui = font(theme.ui_family());
    let mono = theme.mono_family();
    let runs = inline_runs(text, &ui, &mono, weight, colors);
    let styled = StyledText::new(SharedString::from(text.text.clone())).with_runs(runs);
    if text.links.is_empty() {
        return div().child(styled).into_any_element();
    }
    let ranges: Vec<Range<usize>> = text.links.iter().map(|(r, _)| r.clone()).collect();
    let targets: Vec<String> = text.links.iter().map(|(_, h)| h.clone()).collect();
    crate::link::linked_text(id, styled, ranges, move |i, _, cx| {
        if let Some(href) = targets.get(i) {
            open_link(href, cx);
        }
    })
    .into_any_element()
}

/// Opens a web link in the browser; other targets are ignored.
pub fn open_link(href: &str, cx: &mut App) {
    if is_web_link(href) {
        cx.open_url(href);
    }
}

/// A link the browser opens (`http`, `https`, `mailto`).
pub fn is_web_link(href: &str) -> bool {
    let lower = href.trim_start().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")
}

#[derive(Clone, Copy)]
struct InlineColors {
    text: Hsla,
    link: Hsla,
}

/// The text runs of inline text: plain parts in `base` at `weight`, styled
/// parts with their weight, slant, font (code) and strikethrough, links in
/// the link colour.
fn inline_runs(
    text: &Inline,
    base: &Font,
    mono: &SharedString,
    weight: FontWeight,
    colors: InlineColors,
) -> Vec<TextRun> {
    let mut bounds = vec![0, text.text.len()];
    let ranges = text.styles.iter().map(|(r, _)| r);
    for r in ranges.chain(text.links.iter().map(|(r, _)| r)) {
        bounds.push(r.start);
        bounds.push(r.end);
    }
    bounds.retain(|&b| b <= text.text.len() && text.text.is_char_boundary(b));
    bounds.sort_unstable();
    bounds.dedup();
    bounds
        .windows(2)
        .filter(|w| w[1] > w[0])
        .map(|w| {
            let style = text
                .styles
                .iter()
                .find(|(r, _)| r.start <= w[0] && w[1] <= r.end)
                .map(|(_, s)| *s)
                .unwrap_or_default();
            let link = text
                .links
                .iter()
                .any(|(r, _)| r.start <= w[0] && w[1] <= r.end);
            let mut font = base.clone();
            if style.code {
                font.family = mono.clone();
            }
            font.weight = if style.bold { FontWeight::BOLD } else { weight };
            if style.italic {
                font.style = FontStyle::Italic;
            }
            TextRun {
                len: w[1] - w[0],
                font,
                color: if link { colors.link } else { colors.text },
                background_color: None,
                underline: None,
                strikethrough: style.strikethrough.then(|| StrikethroughStyle {
                    thickness: px(1.0),
                    color: None,
                }),
            }
        })
        .collect()
}

/// Code in the editor font and size, its tokens in the colours of the
/// theme's text roles.
pub fn render_code(code: &Code, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let runs = code_runs(code, cx);
    div()
        .font_family(theme.mono_family())
        .text_size(theme.editor_size())
        .text_color(theme.color(UiRole::EditorText))
        .child(StyledText::new(SharedString::from(code.text.clone())).with_runs(runs))
        .into_any_element()
}

fn code_runs(code: &Code, cx: &App) -> Vec<TextRun> {
    let theme = cx.theme();
    let base = font(theme.mono_family());
    let plain = theme.color(UiRole::EditorText);
    let run = |len: usize, role: Option<TextRole>| {
        let mut font = base.clone();
        let mut color = plain;
        if let Some(role) = role {
            let style = theme.text_style(role);
            color = theme.text_color(role);
            if style.bold == Some(true) {
                font.weight = FontWeight::BOLD;
            }
            if style.italic == Some(true) {
                font.style = FontStyle::Italic;
            }
        }
        TextRun {
            len,
            font,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }
    };
    let mut runs = Vec::new();
    let mut at = 0;
    for (range, role) in &code.spans {
        if range.start > at {
            runs.push(run(range.start - at, None));
        }
        runs.push(run(range.len(), Some(*role)));
        at = range.end;
    }
    if at < code.text.len() {
        runs.push(run(code.text.len() - at, None));
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prose_collapses_runs_of_spaces() {
        let md = Markdown::parse(
            "Utilities for with-statement contexts.  See PEP 343.\n\n```\na  =  1\n```",
            None,
        );
        let json = md.to_json();
        assert_eq!(
            json[0]["text"],
            "Utilities for with-statement contexts. See PEP 343."
        );
        assert_eq!(json[1]["text"], "a  =  1");
        assert_eq!(collapse_spaces("  a \t b", true), "a b");
        assert_eq!(collapse_spaces(" a", false), " a");
    }

    /// Colours `def` and `return` as keywords and digits as numbers.
    struct Words;

    impl CodeLexer for Words {
        fn lex(&self, language: Option<&str>, code: &str) -> Vec<(Range<usize>, TextRole)> {
            let _ = language;
            let mut spans = Vec::new();
            for word in ["def", "return"] {
                for (i, _) in code.match_indices(word) {
                    spans.push((i..i + word.len(), TextRole::Keyword));
                }
            }
            for (i, c) in code.char_indices() {
                if c.is_ascii_digit() {
                    spans.push((i..i + 1, TextRole::Number));
                }
            }
            spans.sort_by_key(|(r, _)| r.start);
            spans
        }
    }

    fn style(names: &str) -> Style {
        Style {
            bold: names.contains("bold"),
            italic: names.contains("italic"),
            strikethrough: names.contains("strike"),
            code: names.contains("code"),
        }
    }

    #[test]
    fn paragraphs_emphasis_code_and_links() {
        let md = Markdown::parse(
            "Serialize ``obj`` to a *JSON* **formatted**\n`str`, see [the docs](https://docs.python.org/3/).\n\nSecond ~~old~~ paragraph.  \nNext line.",
            None,
        );
        assert_eq!(md.blocks.len(), 2);
        let Block::Paragraph(p) = &md.blocks[0] else {
            panic!("a paragraph: {:?}", md.blocks[0]);
        };
        // A soft break is a space.
        assert_eq!(
            p.text,
            "Serialize obj to a JSON formatted str, see the docs."
        );
        let styled: Vec<(&str, Style)> = p
            .styles
            .iter()
            .map(|(r, s)| (&p.text[r.clone()], *s))
            .collect();
        assert_eq!(
            styled,
            [
                ("obj", style("code")),
                ("JSON", style("italic")),
                ("formatted", style("bold")),
                ("str", style("code")),
            ]
        );
        assert_eq!(p.links.len(), 1);
        assert_eq!(&p.text[p.links[0].0.clone()], "the docs");
        assert_eq!(p.links[0].1, "https://docs.python.org/3/");
        let Block::Paragraph(second) = &md.blocks[1] else {
            panic!("a paragraph");
        };
        // A hard break is a line break.
        assert_eq!(second.text, "Second old paragraph.\nNext line.");
        assert_eq!(second.styles, [(7..10, style("strike"))]);
    }

    #[test]
    fn headings_lists_rules_and_quotes() {
        let md = Markdown::parse(
            "# Title\n\nText\n\n---\n\n- one\n- two *b*\n  - nested\n\n3. three\n4. four\n\n> quoted\n",
            None,
        );
        let json = md.to_json();
        let types: Vec<&str> = json
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["type"].as_str().unwrap())
            .collect();
        assert_eq!(
            types,
            ["heading", "paragraph", "rule", "list", "list", "quote"]
        );
        assert_eq!(json[0]["level"], 1);
        assert_eq!(json[0]["text"], "Title");
        // A tight list: items without paragraphs of their own.
        let Block::List { start, items } = &md.blocks[3] else {
            panic!("a list");
        };
        assert_eq!(*start, None);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], [Block::Paragraph(Inline::plain("one"))]);
        assert_eq!(items[1].len(), 2, "text and a nested list: {:?}", items[1]);
        assert!(matches!(&items[1][1], Block::List { items, .. } if items.len() == 1));
        assert_eq!(json[4]["start"], 3);
        assert_eq!(json[4]["items"][1][0]["text"], "four");
        assert_eq!(json[5]["blocks"][0]["text"], "quoted");
    }

    #[test]
    fn code_blocks_are_coloured_by_the_lexer() {
        let md = Markdown::parse(
            "```python\ndef f(x):\n    return 42\n```\n\n    indented 1\n\n```text\nplain\n```",
            Some(&Words),
        );
        let Block::Code(code) = &md.blocks[0] else {
            panic!("a code block");
        };
        assert_eq!(code.language.as_deref(), Some("python"));
        assert_eq!(code.text, "def f(x):\n    return 42");
        let spans: Vec<(&str, TextRole)> = code
            .spans
            .iter()
            .map(|(r, role)| (&code.text[r.clone()], *role))
            .collect();
        assert_eq!(
            spans,
            [
                ("def", TextRole::Keyword),
                ("return", TextRole::Keyword),
                ("4", TextRole::Number),
                ("2", TextRole::Number),
            ]
        );
        let Block::Code(indented) = &md.blocks[1] else {
            panic!("an indented code block");
        };
        assert_eq!(indented.language, None);
        assert_eq!(indented.text, "indented 1");
        assert_eq!(md.to_json()[0]["spans"][0]["role"], "keyword");
        // Without a lexer the code has no colours.
        let md = Markdown::parse("```\nreturn 1\n```", None);
        assert_eq!(
            md.blocks,
            [Block::Code(Code {
                language: None,
                text: "return 1".into(),
                spans: Vec::new(),
            })]
        );
    }

    #[test]
    fn spans_that_do_not_fit_are_dropped() {
        struct Bad;
        impl CodeLexer for Bad {
            fn lex(&self, _: Option<&str>, _: &str) -> Vec<(Range<usize>, TextRole)> {
                vec![
                    (0..2, TextRole::Keyword),
                    (1..3, TextRole::Number),
                    (3..4, TextRole::Number),
                    (5..99, TextRole::String),
                ]
            }
        }
        let code = Code::new(None, "añbc".into(), Some(&Bad));
        // "ñ" is two bytes: 1..3 is the char, 0..2 and 3..4 are not on
        // boundaries or overlap.
        assert_eq!(
            code.spans,
            [(1..3, TextRole::Number), (3..4, TextRole::Number)]
        );
    }

    #[test]
    fn html_and_plain_text() {
        let md = Markdown::parse(
            "first<br>second <span>kept</span>\n\n<div>\nblock <b>text</b>\n</div>",
            None,
        );
        let json = md.to_json();
        assert_eq!(json[0]["text"], "first\nsecond kept");
        assert_eq!(json[1]["text"], "block text");
        let plain = Markdown::plain("a\nb\n\n\nc  *not markdown*\n");
        assert_eq!(
            plain.blocks,
            [
                Block::Paragraph(Inline::plain("a\nb")),
                Block::Paragraph(Inline::plain("c  *not markdown*")),
            ]
        );
        assert!(Markdown::parse("  \n", None).is_empty());
    }

    #[test]
    fn line_counts() {
        let md = Markdown::parse(
            "A paragraph of twenty.\n\n```\na\nb\n```\n\n- one\n- two\n\n---",
            None,
        );
        // 22 chars at 10 a line: 3 lines; two items; two lines of code.
        assert_eq!(md.line_counts(10), (5, 2));
        assert_eq!(Markdown::default().line_counts(10), (0, 0));
    }

    #[test]
    fn web_links() {
        assert!(is_web_link(
            "https://docs.astral.sh/ruff/rules/unused-import/"
        ));
        assert!(is_web_link("HTTP://example.org"));
        assert!(!is_web_link("file:///p/a.py#L5"));
        assert!(!is_web_link("command:run"));
    }

    #[test]
    fn runs_cover_the_text() {
        let mut text = Inline::default();
        text.push("plain ", Style::default());
        text.push("code", style("code"));
        text.push_link(" link", "https://x.org", Style::default());
        let colors = InlineColors {
            text: gpui::black(),
            link: gpui::blue(),
        };
        let runs = inline_runs(
            &text,
            &font("Inter"),
            &"Mono".into(),
            FontWeight::NORMAL,
            colors,
        );
        let lens: Vec<usize> = runs.iter().map(|r| r.len).collect();
        assert_eq!(lens, [6, 4, 5]);
        assert_eq!(runs[1].font.family.as_ref(), "Mono");
        assert_eq!(runs[2].color, gpui::blue());
        assert_eq!(runs[0].color, gpui::black());
    }
}
