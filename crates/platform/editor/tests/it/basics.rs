//! Editing basics (tests/scenarios/editor/keymap-and-editing-basics.md):
//! bracket and quote pairing, smart indentation, line and statement moves,
//! comments, selection growth and mouse selection, driven through the
//! input handler, commands and simulated mouse events.

use editor::{Editor, EditorConfig};
use gpui::{
    ClipboardItem, Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase, VisualTestContext,
    point, px,
};
use serde_json::{Value, json};
use text::{Buffer, Selection, SelectionSet};

const CARET: char = '|';
const ANCHOR: char = '‹';
const HEAD: char = '›';

fn parse_marked(marked: &str) -> (String, Selection) {
    let mut text = String::new();
    let (mut caret, mut anchor, mut head) = (None, None, None);
    for c in marked.chars() {
        let pos = text.chars().count();
        match c {
            CARET => caret = Some(pos),
            ANCHOR => anchor = Some(pos),
            HEAD => head = Some(pos),
            c => text.push(c),
        }
    }
    let selection = match (caret, anchor, head) {
        (Some(p), None, None) => Selection::caret(p),
        (None, Some(a), Some(h)) => Selection::new(a, h),
        _ => panic!("bad markers in {marked:?}"),
    };
    (text, selection)
}

fn render_marked(text: &str, s: &Selection) -> String {
    let mut marks = if s.is_empty() {
        vec![(s.head, CARET)]
    } else {
        vec![(s.anchor, ANCHOR), (s.head, HEAD)]
    };
    marks.sort_by_key(|m| std::cmp::Reverse(m.0));
    let mut chars: Vec<char> = text.chars().collect();
    for (pos, mark) in marks {
        chars.insert(pos, mark);
    }
    chars.into_iter().collect()
}

fn python_config() -> EditorConfig {
    EditorConfig {
        indent_after: vec![':'],
        dedent_after: ["return", "pass", "break", "continue", "raise"]
            .map(String::from)
            .to_vec(),
        line_comment: Some("# ".into()),
        quotes: vec!['"', '\''],
        triple_quotes: true,
        string_prefixes: "rRbBfFuU".chars().collect(),
        ..EditorConfig::default()
    }
}

struct T<'a> {
    editor: Entity<Editor>,
    cx: &'a mut VisualTestContext,
}

fn open<'a>(cx: &'a mut TestAppContext, marked: &str) -> T<'a> {
    cx.update(|cx| {
        ui::init(cx, themes::ThemeKind::Dark);
        editor::init(cx);
    });
    let (text, selection) = parse_marked(marked);
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut e = Editor::new(Buffer::new(&text), window, cx);
        e.set_config(python_config());
        e.set_selections(SelectionSet::single(selection), cx);
        e
    });
    cx.update(|window, cx| {
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus);
    });
    let mut t = T { editor, cx };
    t.draw();
    t
}

impl T<'_> {
    fn draw(&mut self) {
        self.cx.run_until_parked();
        self.cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
    }

    fn set(&mut self, marked: &str) {
        let (text, selection) = parse_marked(marked);
        self.editor.update(self.cx, |e, cx| {
            *e.buffer_mut() = Buffer::new(&text);
            e.buffer_reloaded(cx);
            e.set_selections(SelectionSet::single(selection), cx);
        });
        self.draw();
    }

    fn marked(&mut self) -> String {
        self.editor.read_with(self.cx, |e, _| {
            render_marked(&e.text(), e.selections().primary())
        })
    }

    fn input(&mut self, text: &str) {
        self.cx.simulate_input(text);
        self.draw();
    }

    fn invoke(&mut self, id: &str, args: Value) {
        self.cx
            .update(|window, cx| ui::command::invoke(id, args, window, cx))
            .unwrap_or_else(|e| panic!("{id}: {e:?}"));
        self.draw();
    }

    fn run(&mut self, id: &str) {
        self.invoke(id, json!(null));
    }

    /// `before`, then `f`, then the marked text must be `after`.
    fn case(&mut self, before: &str, after: &str, f: impl FnOnce(&mut Self)) {
        self.set(before);
        f(self);
        assert_eq!(self.marked(), after, "from {before:?}");
    }

    /// Window position of a line and column (middle of the line, just right
    /// of the column's left edge).
    fn at(&mut self, line: usize, col: usize) -> gpui::Point<gpui::Pixels> {
        let (origin, lh, cw) = self
            .editor
            .read_with(self.cx, |e, _| e.text_geometry().unwrap());
        point(
            origin.x + cw * col as f32 + px(1.0),
            origin.y + lh * line as f32 + lh / 2.0,
        )
    }

    fn click(&mut self, at: gpui::Point<gpui::Pixels>, clicks: usize, modifiers: Modifiers) {
        self.cx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers,
            click_count: clicks,
            first_mouse: false,
        });
        self.cx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers,
            click_count: clicks,
        });
        self.draw();
    }
}

#[gpui::test]
fn brackets_and_quotes_pair_and_type_over(cx: &mut TestAppContext) {
    let mut t = open(cx, "|");
    // An opening bracket brings its closing one; typing it steps over.
    t.case("print|", "print(|)", |t| t.input("("));
    t.case("print(|)", "print()|", |t| t.input(")"));
    t.case("x = |", "x = [1, 2]|", |t| t.input("[1, 2]"));
    // No pair before a word character.
    t.case("|abc", "(|abc", |t| t.input("("));
    // Quotes pair, step over, and not after a word or inside a string.
    t.case("x = |", "x = \"a\"|", |t| t.input("\"a\""));
    t.case("don|", "don'|", |t| t.input("'"));
    t.case("x = \"a|", "x = \"a\"|", |t| t.input("\""));
    // A string prefix may precede the quote.
    t.case("x = f|", "x = f\"|\"", |t| t.input("\""));
    // Three quotes open a triple-quoted string.
    t.case("    |", "    \"\"\"|\"\"\"", |t| t.input("\"\"\""));
    // A selection is wrapped.
    t.case("x = ‹abc›", "x = (‹abc›)", |t| t.input("("));
    t.case("x = ‹abc›", "x = '‹abc›'", |t| t.input("'"));
    // Backspace between an empty pair removes both.
    t.case("f(|)", "f|", |t| t.run("editor.backspace"));
    t.case("x = \"|\"", "x = |", |t| t.run("editor.backspace"));
    t.case("f(a|)", "f(|)", |t| t.run("editor.backspace"));
}

#[gpui::test]
fn smart_enter_backspace_and_indentation(cx: &mut TestAppContext) {
    let mut t = open(cx, "|");
    t.case("    x = 1|", "    x = 1\n    |", |t| {
        t.run("editor.newline")
    });
    t.case("def f():|", "def f():\n    |", |t| t.run("editor.newline"));
    t.case(
        "    if x:\n        return 1|",
        "    if x:\n        return 1\n    |",
        |t| t.run("editor.newline"),
    );
    t.case("f(|)", "f(\n    |\n)", |t| t.run("editor.newline"));
    t.case("x = [|", "x = [\n    |", |t| t.run("editor.newline"));
    // Backspace in leading spaces goes back one indentation level.
    t.case("        |x", "    |x", |t| t.run("editor.backspace"));
    t.case("      |x", "    |x", |t| t.run("editor.backspace"));
    // Tab and Shift+Tab on several lines indent and unindent them.
    t.case("‹a\nb›", "‹    a\n    b›", |t| t.run("editor.tab"));
    t.case("    ‹a\n    b›", "‹a\nb›", |t| {
        t.run("editor.unindent")
    });
    // Tab at a column fills up to the next indentation stop.
    t.case("  |a", "    |a", |t| t.run("editor.tab"));
}

#[gpui::test]
fn deleting_to_line_start_and_end(cx: &mut TestAppContext) {
    let mut t = open(cx, "|");
    t.case("abc d|ef", "|ef", |t| t.run("editor.delete-to-line-start"));
    t.case("ab\n|cd", "ab|cd", |t| t.run("editor.delete-to-line-start"));
    t.case("ab|c def", "ab|", |t| t.run("editor.delete-to-line-end"));
    t.case("ab|\ncd", "ab|cd", |t| t.run("editor.delete-to-line-end"));
}

#[gpui::test]
fn lines_and_statements_move(cx: &mut TestAppContext) {
    let mut t = open(cx, "|");
    // Selected lines move together and stay selected.
    t.case("a\n‹b\nc›\nd", "‹b\nc›\na\nd", |t| {
        t.run("editor.move-line-up")
    });
    t.case("a\n‹b\nc›\nd", "a\nd\n‹b\nc›", |t| {
        t.run("editor.move-line-down")
    });
    // The first line does not move up, the last does not move down.
    t.case("a|\nb", "a|\nb", |t| t.run("editor.move-line-up"));
    t.case("a\nb|", "a\nb|", |t| t.run("editor.move-line-down"));
    // A statement moves with its block; blank lines between stay put.
    let before = "def f():\n    return 1\n\n\ndef g|():\n    return 2\n";
    let after = "def g|():\n    return 2\n\n\ndef f():\n    return 1\n";
    t.case(before, after, |t| t.run("editor.move-statement-up"));
    let before = "def f|():\n    return 1\n\n\ndef g():\n    return 2";
    let after = "def g():\n    return 2\n\n\ndef f|():\n    return 1";
    t.case(before, after, |t| t.run("editor.move-statement-down"));
    // Bracketed continuation lines belong to the statement.
    let before = "x = f(\n    1,\n)\ny| = 2";
    let after = "y| = 2\nx = f(\n    1,\n)";
    t.case(before, after, |t| t.run("editor.move-statement-up"));
    // The first statement of a block stays inside it.
    let same = "if a:\n    b| = 1\nc = 2";
    t.case(same, same, |t| t.run("editor.move-statement-up"));
}

#[gpui::test]
fn join_duplicate_and_delete_lines(cx: &mut TestAppContext) {
    let mut t = open(cx, "|");
    t.case("a = (|\n    1\n)", "a = (|1\n)", |t| {
        t.run("editor.join-lines")
    });
    t.case("‹a\n  b\n  c›", "a| b c", |t| {
        t.run("editor.join-lines")
    });
    t.case("x ‹ab› y", "x ab‹ab› y", |t| {
        t.run("editor.duplicate")
    });
    t.case("a\n‹b\nc›\nd", "a\nd|", |t| t.run("editor.delete-line"));
}

#[gpui::test]
fn line_comments_toggle(cx: &mut TestAppContext) {
    let mut t = open(cx, "|");
    // Commenting moves the caret down; uncommenting accepts a missing space.
    t.case("    a|\nb", "    # a\nb|", |t| {
        t.run("editor.toggle-comment")
    });
    t.case("    #a|\nb", "    a\nb|", |t| {
        t.run("editor.toggle-comment")
    });
    // Selected lines are commented at their smallest indentation; blank
    // lines are left alone.
    t.case("‹if x:\n\n    y›", "‹# if x:\n\n#     y›", |t| {
        t.run("editor.toggle-comment")
    });
    t.case("‹# if x:\n\n#     y›", "‹if x:\n\n    y›", |t| {
        t.run("editor.toggle-comment")
    });
    // Mixed lines are commented, not uncommented.
    t.case("‹# a\nb›", "‹# # a\n# b›", |t| {
        t.run("editor.toggle-comment")
    });
}

#[gpui::test]
fn selection_grows_and_shrinks(cx: &mut TestAppContext) {
    let mut t = open(cx, "|");
    // Without a syntax tree: word, line text, whole line, file.
    t.set("x = a + fo|o\ny");
    let steps = [
        "x = a + ‹foo›\ny",
        "‹x = a + foo›\ny",
        "‹x = a + foo\n›y",
        "‹x = a + foo\ny›",
    ];
    for step in steps {
        t.run("editor.select-word");
        assert_eq!(t.marked(), step);
    }
    for step in steps.iter().rev().skip(1) {
        t.run("editor.shrink-selection");
        assert_eq!(t.marked(), *step);
    }
    t.run("editor.shrink-selection");
    assert_eq!(t.marked(), "x = a + fo|o\ny");
    // Escape drops the selection.
    t.run("editor.select-word");
    t.run("editor.escape");
    assert_eq!(t.marked(), "x = a + foo|\ny");
}

#[gpui::test]
fn whole_lines_paste_above_the_caret_line(cx: &mut TestAppContext) {
    let mut t = open(cx, "|");
    t.set("a|b\ncd");
    t.run("editor.copy");
    let copied = t.cx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(copied.as_deref(), Some("ab\n"));
    t.case("x\nc|d", "x\nab\nc|d", |t| t.run("editor.paste"));
    // Text without the whole-line mark goes in at the caret.
    t.cx.write_to_clipboard(ClipboardItem::new_string("ab\n".into()));
    t.case("c|d", "cab\n|d", |t| t.run("editor.paste"));
}

#[gpui::test]
fn go_to_line_and_column(cx: &mut TestAppContext) {
    let lines: Vec<String> = (0..500).map(|i| format!("line {i}")).collect();
    let mut t = open(cx, &format!("|{}", lines.join("\n")));
    t.invoke("editor.go-to-line", json!({ "line": 300, "column": 3 }));
    let (caret, visible) = t
        .editor
        .read_with(t.cx, |e, _| (e.caret_point(), e.visible_lines().unwrap()));
    assert_eq!((caret.line, caret.column), (299, 2));
    // The line is in the middle of the view.
    let middle = (visible.start + visible.end) / 2;
    assert!(middle.abs_diff(299) <= 2, "{visible:?}");
    // Out of range values are clamped.
    t.invoke("editor.go-to-line", json!({ "line": 9999 }));
    let caret = t.editor.read_with(t.cx, |e, _| e.caret_point());
    assert_eq!((caret.line, caret.column), (499, 0));
}

#[gpui::test]
fn mouse_clicks_drags_and_scrolls(cx: &mut TestAppContext) {
    let lines: Vec<String> = (0..200).map(|i| format!("word{i} more text")).collect();
    let mut t = open(cx, &format!("|{}", lines.join("\n")));
    let line_start =
        |t: &mut T, line: usize| t.editor.read_with(t.cx, |e, _| e.buffer().line_start(line));
    // Triple click selects the line with its line break.
    let at = t.at(2, 3);
    t.click(at, 3, Modifiers::none());
    let s = t
        .editor
        .read_with(t.cx, |e, _| e.selections().primary().range());
    assert_eq!(s, line_start(&mut t, 2)..line_start(&mut t, 3));
    // Shift+click extends from the caret.
    let at = t.at(1, 0);
    t.click(at, 1, Modifiers::none());
    let at = t.at(3, 4);
    t.click(at, 1, Modifiers::shift());
    let s = t.editor.read_with(t.cx, |e, _| *e.selections().primary());
    assert_eq!(s.anchor, line_start(&mut t, 1));
    assert_eq!(s.head, line_start(&mut t, 3) + 4);
    // Dragging selects from the press to the mouse.
    let from = t.at(4, 1);
    let to = t.at(5, 2);
    t.cx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: from,
        modifiers: Modifiers::none(),
        click_count: 1,
        first_mouse: false,
    });
    t.cx.simulate_event(MouseMoveEvent {
        position: to,
        pressed_button: Some(MouseButton::Left),
        modifiers: Modifiers::none(),
    });
    t.cx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: to,
        modifiers: Modifiers::none(),
        click_count: 1,
    });
    t.draw();
    let s = t.editor.read_with(t.cx, |e, _| *e.selections().primary());
    assert_eq!(s.anchor, line_start(&mut t, 4) + 1);
    assert_eq!(s.head, line_start(&mut t, 5) + 2);
    // A trackpad scrolls by pixels.
    let at = t.at(5, 2);
    t.cx.simulate_event(ScrollWheelEvent {
        position: at,
        delta: ScrollDelta::Pixels(point(px(0.0), px(-37.0))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    t.draw();
    let top = t.editor.read_with(t.cx, |e, _| e.scroll_top());
    assert_eq!(top, px(37.0));
}
