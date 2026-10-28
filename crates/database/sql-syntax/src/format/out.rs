use super::style::CodeStyle;

/// Whether a token is separated from the one before it by a space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lead {
    Space,
    /// A space even after a token that wants none (operators that would
    /// run together).
    Force,
    No,
}

/// How many blank lines a break may carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Blank {
    None,
    /// The blank lines of the text, at most this many.
    UpTo(usize),
    /// Exactly this many (between statements).
    Exactly(usize),
}

/// A line break that waits for the next token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Brk {
    pub indent: usize,
    pub blank: Blank,
}

impl Brk {
    pub fn at(indent: usize) -> Brk {
        Brk {
            indent,
            blank: Blank::None,
        }
    }
}

/// The state the output can be returned to (a failed one-line attempt).
pub(crate) struct Snap {
    len: usize,
    line_start: usize,
    indent_end: usize,
    content_end: usize,
    col: usize,
    has_content: bool,
    opps: Vec<(usize, usize, bool)>,
    opp_next: Option<usize>,
    prev_open: bool,
    pending: Option<Brk>,
    hold: Option<usize>,
    nl_seen: usize,
    line_comment: bool,
    last_comment: bool,
}

/// The text being written: tokens are appended with their spacing, breaks
/// wait for the next token, long lines are wrapped at the opportunities the
/// layout marked. It knows nothing about SQL.
pub(crate) struct Out<'s> {
    style: &'s CodeStyle,
    buf: String,
    line_start: usize,
    indent_end: usize,
    /// The end of the last token written: spaces after it may be removed.
    content_end: usize,
    col: usize,
    has_content: bool,
    /// Places where the current line may be wrapped: the byte offset of the
    /// separator, the indent of the new line and whether a space is there.
    opps: Vec<(usize, usize, bool)>,
    /// The next separator is an opportunity with this indent.
    pub opp_next: Option<usize>,
    /// No space before the next token.
    prev_open: bool,
    pub pending: Option<Brk>,
    /// A break to make as soon as the text has a line break.
    pub hold: Option<usize>,
    /// Line breaks in the text since the last token.
    pub nl_seen: usize,
    /// A line comment ended the line: the next token goes to a new line.
    pub line_comment: bool,
    last_comment: bool,
    /// Above zero: everything goes on one line.
    pub flat: u32,
    /// A one-line attempt met something that needs a line break.
    pub forced: bool,
    /// The indent of lines that continue an element.
    pub cont: usize,
    /// Between statements, outside any of them.
    pub top: bool,
}

pub(crate) fn text_width(s: &str) -> usize {
    s.chars().count()
}

impl<'s> Out<'s> {
    pub fn new(style: &'s CodeStyle) -> Out<'s> {
        Out {
            style,
            buf: String::new(),
            line_start: 0,
            indent_end: 0,
            content_end: 0,
            col: 0,
            has_content: false,
            opps: Vec::new(),
            opp_next: None,
            prev_open: true,
            pending: None,
            hold: None,
            nl_seen: 0,
            line_comment: false,
            last_comment: false,
            flat: 0,
            forced: false,
            cont: 0,
            top: true,
        }
    }

    pub fn finish(self) -> String {
        self.buf
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn ends_with_newline(&self) -> bool {
        self.buf.ends_with('\n')
    }

    pub fn push_newline(&mut self) {
        self.buf.push('\n');
    }

    pub fn col(&self) -> usize {
        self.col
    }

    pub fn prev_open(&self) -> bool {
        self.prev_open
    }

    /// Makes the break that waits for the next token, so that a one-line
    /// attempt starts on the line the token would have been on.
    pub fn flush(&mut self) {
        if self.flat > 0 {
            return;
        }
        let brk = self
            .pending
            .take()
            .or_else(|| self.line_comment.then(|| Brk::at(self.cont)));
        if let Some(b) = brk {
            self.newline(b);
            self.line_comment = false;
        }
    }

    pub fn clear_opps(&mut self) {
        self.opps.clear();
    }

    /// Asks for a line break before the next token (not in a one-line run).
    pub fn request(&mut self, brk: Brk) {
        if self.flat == 0 {
            self.pending = Some(brk);
        }
    }

    pub fn snapshot(&self) -> Snap {
        Snap {
            len: self.buf.len(),
            line_start: self.line_start,
            indent_end: self.indent_end,
            content_end: self.content_end,
            col: self.col,
            has_content: self.has_content,
            opps: self.opps.clone(),
            opp_next: self.opp_next,
            prev_open: self.prev_open,
            pending: self.pending,
            hold: self.hold,
            nl_seen: self.nl_seen,
            line_comment: self.line_comment,
            last_comment: self.last_comment,
        }
    }

    /// The text written since `snap`.
    pub fn since(&self, snap: &Snap) -> &str {
        &self.buf[snap.len..]
    }

    pub fn restore(&mut self, snap: Snap) {
        self.buf.truncate(snap.len);
        self.line_start = snap.line_start;
        self.indent_end = snap.indent_end;
        self.content_end = snap.content_end;
        self.col = snap.col;
        self.has_content = snap.has_content;
        self.opps = snap.opps;
        self.opp_next = snap.opp_next;
        self.prev_open = snap.prev_open;
        self.pending = snap.pending;
        self.hold = snap.hold;
        self.nl_seen = snap.nl_seen;
        self.line_comment = snap.line_comment;
        self.last_comment = snap.last_comment;
        self.forced = false;
    }

    fn push(&mut self, s: &str) {
        self.buf.push_str(s);
        self.content_end = self.buf.len();
        match s.rfind('\n') {
            Some(i) => {
                self.line_start = self.buf.len() - (s.len() - i - 1);
                self.indent_end = self.line_start;
                self.col = text_width(&s[i + 1..]);
            }
            None => self.col += text_width(s),
        }
    }

    fn indent_text(&self, cols: usize) -> String {
        if self.style.use_tab {
            let tab = self.style.tab_size.max(1);
            let mut s = "\t".repeat(cols / tab);
            s.push_str(&" ".repeat(cols % tab));
            s
        } else {
            " ".repeat(cols)
        }
    }

    fn start_line(&mut self, indent: usize) {
        self.line_start = self.buf.len();
        let text = self.indent_text(indent);
        self.buf.push_str(&text);
        self.indent_end = self.buf.len();
        self.content_end = self.buf.len();
        self.col = indent;
        self.has_content = false;
        self.opps.clear();
        self.prev_open = true;
    }

    fn newline(&mut self, brk: Brk) {
        while self.buf.len() > self.content_end && self.buf.ends_with(' ') {
            self.buf.pop();
        }
        if !self.buf.is_empty() {
            let blank = match brk.blank {
                Blank::None => 0,
                Blank::UpTo(max) => self.nl_seen.saturating_sub(1).min(max),
                Blank::Exactly(_) if self.last_comment => self
                    .nl_seen
                    .saturating_sub(1)
                    .min(self.style.keep_blank_lines),
                Blank::Exactly(n) => n,
            };
            for _ in 0..=blank {
                self.buf.push('\n');
            }
        }
        self.start_line(brk.indent);
    }

    /// Puts the spaces that bring the line to column `col` (alignment).
    pub fn pad_to(&mut self, col: usize) {
        if self.flat > 0 || !self.has_content {
            return;
        }
        if self.col >= col {
            return;
        }
        while self.col < col {
            self.buf.push(' ');
            self.col += 1;
        }
        self.prev_open = true;
    }

    /// The next token goes to a new line (after text that ends with a line
    /// comment).
    pub fn end_line(&mut self) {
        self.line_comment = true;
    }

    /// Appends a token. `open_after`: no space before the next one.
    pub fn token(&mut self, text: &str, lead: Lead, open_after: bool, wrap: bool) {
        let mut brk = None;
        if self.flat == 0 {
            brk = self.pending.take();
            let kept = lead != Lead::No
                && self.style.keep_line_breaks
                && self.nl_seen > 0
                && self.has_content;
            if brk.is_none() && (self.line_comment || kept) {
                brk = Some(Brk::at(self.cont));
            }
        } else {
            self.pending = None;
        }
        if let Some(b) = brk {
            self.newline(b);
        } else {
            let space = self.has_content
                && (lead == Lead::Force || (lead == Lead::Space && !self.prev_open));
            if self.has_content
                && self.flat == 0
                && let Some(indent) = self.opp_next
            {
                self.opps.push((self.buf.len(), indent, space));
            }
            if space {
                self.buf.push(' ');
                self.col += 1;
            }
        }
        self.push(text);
        self.has_content = true;
        self.prev_open = open_after;
        self.nl_seen = 0;
        self.line_comment = false;
        self.last_comment = false;
        self.hold = None;
        self.opp_next = None;
        if wrap {
            self.wrap();
        }
    }

    fn wrap(&mut self) {
        let margin = self.style.right_margin;
        if self.flat > 0 || margin == 0 || self.col <= margin {
            return;
        }
        let Some(&(p, indent, space)) = self
            .opps
            .iter()
            .rev()
            .find(|(p, _, _)| *p > self.indent_end)
        else {
            return;
        };
        let rest = self.buf[p + usize::from(space)..].to_string();
        let open = self.prev_open;
        self.buf.truncate(p);
        let opps = std::mem::take(&mut self.opps);
        self.buf.push('\n');
        self.start_line(indent);
        self.push(&rest);
        self.has_content = true;
        self.prev_open = open;
        // Opportunities after the split stay, shifted to the new line.
        let shift = self.indent_end as isize - (p as isize + isize::from(space));
        self.opps = opps
            .into_iter()
            .filter(|(q, _, _)| *q > p)
            .map(|(q, i, sp)| ((q as isize + shift) as usize, i, sp))
            .collect();
    }

    /// A line break in the text between tokens.
    pub fn whitespace(&mut self, text: &str) {
        let n = text.bytes().filter(|b| *b == b'\n').count();
        if n > 0 {
            self.nl_seen += n;
            if self.flat == 0
                && let Some(indent) = self.hold.take()
            {
                self.pending = Some(Brk::at(indent));
            }
        }
    }

    /// A comment between tokens.
    pub fn comment(&mut self, text: &str, line: bool) {
        if self.flat > 0 {
            if line || text.contains('\n') {
                self.forced = true;
                return;
            }
            self.trailing_comment(text, false);
            return;
        }
        if self.nl_seen == 0 && self.has_content {
            self.trailing_comment(text, line);
            return;
        }
        let (indent, blank) = match self.pending {
            Some(b) => (b.indent, b.blank),
            None if self.top => (0, Blank::UpTo(self.style.keep_blank_lines)),
            None => (self.cont, Blank::None),
        };
        if self.buf.is_empty() {
            self.start_line(indent);
        } else {
            self.newline(Brk { indent, blank });
        }
        self.push(text);
        self.has_content = true;
        self.prev_open = false;
        self.nl_seen = 0;
        // A line comment ends the line; a block comment on a line of its own
        // leaves the next token where the text had it.
        self.line_comment = line;
        self.last_comment = true;
        self.hold = None;
        self.opp_next = None;
    }

    fn trailing_comment(&mut self, text: &str, line: bool) {
        self.buf.push(' ');
        self.col += 1;
        self.push(text);
        self.prev_open = false;
        self.line_comment = line;
        self.last_comment = true;
        self.opp_next = None;
    }
}
