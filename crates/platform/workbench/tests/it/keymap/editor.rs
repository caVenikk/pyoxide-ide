//! Caret movement, selection, editing and the clipboard: the bindings of `keymaps/*/editor.toml`.

use super::*;

area_tests!("editor");

/// Text and caret before and after commands that only edit or move.
fn editing_case(command: &str) -> Option<(&'static str, &'static str)> {
    Some(match command {
        "editor.move-left" => ("ab c|d", "ab |cd"),
        "editor.move-right" => ("ab |cd", "ab c|d"),
        "editor.move-up" => ("ab\ncd|", "ab|\ncd"),
        "editor.move-down" => ("ab|\ncd", "ab\ncd|"),
        "editor.move-word-left" => ("ab cd|", "ab |cd"),
        "editor.move-word-right" => ("|ab", "ab|"),
        "editor.move-line-start" => ("  ab|", "  |ab"),
        "editor.move-line-end" => ("|ab", "ab|"),
        "editor.move-document-start" => ("ab\ncd|", "|ab\ncd"),
        "editor.move-document-end" => ("|ab\ncd", "ab\ncd|"),
        "editor.select-left" => ("ab|", "a›b‹"),
        "editor.select-right" => ("|ab", "‹a›b"),
        "editor.select-up" => ("ab\ncd|", "ab›\ncd‹"),
        "editor.select-down" => ("|ab\ncd", "‹ab\n›cd"),
        "editor.select-word-left" => ("ab cd|", "ab ›cd‹"),
        "editor.select-word-right" => ("|ab", "‹ab›"),
        "editor.select-line-start" => ("  ab|", "  ›ab‹"),
        "editor.select-line-end" => ("|ab", "‹ab›"),
        "editor.select-document-start" => ("ab\ncd|", "›ab\ncd‹"),
        "editor.select-document-end" => ("|ab\ncd", "‹ab\ncd›"),
        "editor.select-all" => ("a|b\ncd", "‹ab\ncd›"),
        "editor.select-word" => ("ab c|d ef", "ab ‹cd› ef"),
        "editor.escape" => ("‹ab›", "ab|"),
        "editor.newline" => ("def f():|", "def f():\n    |"),
        "editor.start-new-line" => ("a|b", "ab\n|"),
        "editor.tab" => ("|ab", "    |ab"),
        "editor.unindent" => ("    a|b", "a|b"),
        "editor.backspace" => ("ab|", "a|"),
        "editor.delete" => ("|ab", "|b"),
        "editor.delete-word-left" => ("ab cd|", "ab |"),
        "editor.delete-word-right" => ("|ab", "|"),
        "editor.delete-line" => ("ab\nc|d\nef", "ab\ne|f"),
        "editor.duplicate" => ("a|b", "ab\na|b"),
        "editor.move-line-up" => ("ab\nc|d", "c|d\nab"),
        "editor.move-line-down" => ("a|b\ncd", "cd\na|b"),
        "editor.move-statement-up" => ("def f():\n    pass\nx| = 1", "x| = 1\ndef f():\n    pass"),
        "editor.move-statement-down" => {
            ("d|ef f():\n    pass\nx = 1", "x = 1\nd|ef f():\n    pass")
        }
        "editor.join-lines" => ("a|b\n    cd", "ab| cd"),
        "editor.toggle-comment" => ("    a|b\ncd", "    # ab\ncd|"),
        _ => return None,
    })
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    if let Some((before, after)) = editing_case(&b.command) {
        h.set_editor(before);
        h.press(keys);
        assert_eq!(h.editor_marked(), after, "{what}");
        return true;
    }
    match b.command.as_str() {
        "editor.page-up"
        | "editor.page-down"
        | "editor.select-page-up"
        | "editor.select-page-down" => {
            let text: Vec<String> = (0..300).map(|i| format!("line {i}")).collect();
            let mut marked = text.join("\n");
            let at = marked.find("line 150").unwrap();
            marked.insert(at, CARET);
            h.set_editor(&marked);
            h.press(keys);
            let (anchor, head) = h.caret_line();
            let down = b.command.ends_with("down");
            assert!(
                if down { head > 155 } else { head < 145 },
                "{what}: caret on line {head}"
            );
            let select = b.command.contains("select");
            assert_eq!(anchor == 150, select, "{what}: anchor on line {anchor}");
        }
        "editor.shrink-selection" => {
            h.set_editor("ab c|d");
            h.invoke("editor.select-word", json!(null));
            assert_eq!(h.editor_marked(), "ab ‹cd›");
            h.focus_editor();
            h.press(keys);
            assert_eq!(h.editor_marked(), "ab c|d", "{what}");
        }
        "editor.undo" => {
            h.set_editor("ab|");
            h.invoke("editor.type", json!({ "text": "x" }));
            h.focus_editor();
            h.press(keys);
            assert_eq!(h.editor_marked(), "ab|", "{what}");
        }
        "editor.redo" => {
            h.set_editor("ab|");
            h.invoke("editor.type", json!({ "text": "x" }));
            h.invoke("editor.undo", json!(null));
            h.focus_editor();
            h.press(keys);
            assert_eq!(h.editor_marked(), "abx|", "{what}");
        }
        "editor.copy" | "editor.cut" => {
            h.set_editor("a|b\ncd");
            h.press(keys);
            let copied = h.cx.read_from_clipboard().and_then(|c| c.text());
            assert_eq!(copied.as_deref(), Some("ab\n"), "{what}");
            let after = if b.command == "editor.cut" {
                "c|d"
            } else {
                "a|b\ncd"
            };
            assert_eq!(h.editor_marked(), after, "{what}");
        }
        "editor.paste" => {
            h.cx.write_to_clipboard(ClipboardItem::new_string("xy".into()));
            h.set_editor("a|b");
            h.press(keys);
            assert_eq!(h.editor_marked(), "axy|b", "{what}");
        }
        _ => return false,
    }
    true
}
