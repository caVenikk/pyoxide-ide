//! Find and Replace in Files: the bindings of `keymaps/*/find-in-files.toml`.

use super::*;

area_tests!("find-in-files");

impl H<'_> {
    fn popup(&mut self) -> Value {
        self.invoke("find-in-files.state", json!(null))
    }

    /// Searches the project with the Find in Files popup (plain, without a
    /// mask); the focus is in its search field.
    /// Waits for the popup's preview and puts the focus and the caret in
    /// it.
    fn focus_preview(&mut self, caret: usize) {
        self.cx
            .executor()
            .advance_clock(workbench::find_in_files::PREVIEW_DELAY * 2);
        self.draw();
        let popup = self.workbench(|w, _| w.find_in_files().clone());
        let preview = self.cx.update(|cx| popup.read(cx).preview_editor().clone());
        self.cx.update(|cx| {
            preview.update(cx, |e, cx| e.set_selections(SelectionSet::caret(caret), cx))
        });
        let handle = self.cx.update(|cx| preview.focus_handle(cx));
        self.focus(handle);
    }

    /// The preview's selection, as text.
    fn preview_selection(&mut self) -> String {
        let popup = self.workbench(|w, _| w.find_in_files().clone());
        self.cx.update(|cx| {
            let e = popup.read(cx).preview_editor().read(cx);
            e.buffer().slice(e.selections().primary().range())
        })
    }

    fn search_in_files(&mut self, text: &str) {
        self.invoke(
            "find-in-files.search",
            json!({ "text": text, "case_sensitive": false, "whole_words": false, "regex": false, "mask": "" }),
        );
        let popup = self.workbench(|w, _| w.find_in_files().clone());
        let field = self.cx.update(|cx| popup.read(cx).query_field().clone());
        let focused = self
            .cx
            .update_window(self.window.into(), |_, window, cx| {
                field.focus_handle(cx).is_focused(window)
            })
            .unwrap();
        assert!(focused, "the popup's search field has the focus");
    }
}

/// Checks one binding; returns `false` when there is no check for its
/// command.
fn check(h: &mut H, b: &Binding) -> bool {
    let keys = b.keys.as_str();
    let what = format!("`{keys}` -> {}", b.command);
    h.reset();
    let in_popup = b
        .context
        .as_deref()
        .is_some_and(|c| c.starts_with("FindInFiles"));
    match b.command.as_str() {
        "find.toggle-case" | "find.toggle-words" | "find.toggle-regex" if in_popup => {
            h.search_in_files(" = ");
            let option = match b.command.as_str() {
                "find.toggle-case" => "case_sensitive",
                "find.toggle-words" => "whole_words",
                _ => "regex",
            };
            h.press(keys);
            assert_eq!(h.popup()[option], json!(true), "{what}");
        }
        "find.show-history" if in_popup => {
            for text in ["alpha", "beta"] {
                h.search_in_files(text);
                h.invoke("find-in-files.close", json!(null));
            }
            h.search_in_files("gamma");
            h.press(keys);
            let d = h.popup()["dropdown"].clone();
            assert_eq!(d["kind"], json!("search-history"), "{what}");
            assert_eq!(d["items"][0], json!("beta"), "{what}");
            h.invoke("find.history-close", json!(null));
        }
        "find-in-files.open-replace" => {
            h.set_editor("x ‹foo› foo");
            h.press(keys);
            let s = h.popup();
            assert_eq!(s["open"], json!(true), "{what}");
            assert_eq!(s["replace_mode"], json!(true), "{what}");
            assert_eq!(s["text"], json!("foo"), "{what}");
        }
        "find-in-files.scope-project"
        | "find-in-files.scope-module"
        | "find-in-files.scope-directory"
        | "find-in-files.scope-named" => {
            h.search_in_files(" = ");
            if b.command.ends_with("project") {
                h.invoke("find-in-files.scope-directory", json!(null));
                h.invoke("find-in-files.open", json!(null));
            }
            h.press(keys);
            let expected = match b.command.as_str() {
                "find-in-files.scope-project" => "project",
                "find-in-files.scope-module" => "module",
                "find-in-files.scope-directory" => "directory",
                _ => "Project Files",
            };
            assert_eq!(h.popup()["scope"], json!(expected), "{what}");
            h.invoke("find-in-files.scope-project", json!(null));
        }
        "find-in-files.toggle-recursive" => {
            h.search_in_files(" = ");
            let before = h.popup()["recursive"].as_bool().unwrap();
            h.press(keys);
            assert_eq!(h.popup()["recursive"], json!(!before), "{what}");
            h.invoke("find-in-files.toggle-recursive", json!(null));
        }
        "find-in-files.next-field" | "find-in-files.previous-field" => {
            // Search and replace fields: either key goes to the other one.
            h.invoke(
                "find-in-files.search",
                json!({ "text": " = ", "replace": "x", "mask": "" }),
            );
            h.press(keys);
            let popup = h.workbench(|w, _| w.find_in_files().clone());
            let field = h.cx.update(|cx| popup.read(cx).replace_field().clone());
            let focused =
                h.cx.update_window(h.window.into(), |_, window, cx| {
                    field.focus_handle(cx).is_focused(window)
                })
                .unwrap();
            assert!(focused, "{what}: the replace field has the focus");
        }
        "find-in-files.replace-selected" | "find-in-files.replace-all" => {
            for file in ["rep1.txt", "rep2.txt"] {
                std::fs::write(h.root.join(file), "qq_token qq_token\n").unwrap();
            }
            h.invoke(
                "find-in-files.search",
                json!({ "text": "qq_token", "replace": "done", "mask": "", "case_sensitive": false, "whole_words": false, "regex": false }),
            );
            assert_eq!(h.popup()["matches"], json!(4));
            h.press(keys);
            if b.command.ends_with("replace-all") {
                h.cx.simulate_prompt_answer("Replace");
                h.draw();
                for file in ["rep1.txt", "rep2.txt"] {
                    let disk = std::fs::read_to_string(h.root.join(file)).unwrap();
                    assert_eq!(disk, "done done\n", "{what}");
                }
            } else {
                let changed = ["rep1.txt", "rep2.txt"]
                    .iter()
                    .filter(|f| {
                        std::fs::read_to_string(h.root.join(f)).unwrap() == "done qq_token\n"
                    })
                    .count();
                assert_eq!(changed, 1, "{what}: the selected match");
                assert_eq!(h.popup()["rows"].as_array().unwrap().len(), 3, "{what}");
            }
            for file in ["rep1.txt", "rep2.txt"] {
                std::fs::remove_file(h.root.join(file)).unwrap();
            }
            if h.popup()["open"] == json!(true) {
                h.invoke("find-in-files.close", json!(null));
            }
        }
        "find-in-files.browse-directory" => {
            h.search_in_files(" = ");
            h.invoke("find-in-files.scope-directory", json!(null));
            let before = h.popup()["browse_requests"].as_u64().unwrap();
            h.press(keys);
            assert_eq!(
                h.popup()["browse_requests"],
                json!(before + 1),
                "{what}: the folder chooser"
            );
            h.invoke("find-in-files.scope-project", json!(null));
        }
        "find.new-line" if in_popup => {
            h.search_in_files("a");
            h.press("end");
            h.press(keys);
            h.cx.simulate_input(h.window.into(), "b");
            h.draw();
            assert_eq!(h.popup()["text"], json!("a\nb"), "{what}");
        }
        "find-in-files.open" => {
            h.set_editor("x ‹foo› foo");
            h.press(keys);
            let s = h.popup();
            assert_eq!(s["open"], json!(true), "{what}");
            assert_eq!(s["text"], json!("foo"), "{what}");
        }
        "find-in-files.close" => {
            h.search_in_files(" = ");
            h.press(keys);
            assert_eq!(h.popup()["open"], json!(false), "{what}");
            assert!(h.editor_focused(), "{what}: the focus goes back");
        }
        "find-in-files.next-result"
        | "find-in-files.previous-result"
        | "find-in-files.down"
        | "find-in-files.up"
        | "find-in-files.page-down"
        | "find-in-files.page-up" => {
            // The `x = n` lines of the project's files (some may be open
            // with other text).
            h.search_in_files(" = ");
            let n = h.popup()["rows"].as_array().unwrap().len();
            assert!(n >= 3, "{what}: {n} rows");
            let forward = b.command.ends_with("next-result") || b.command.ends_with("down");
            if !forward {
                h.invoke("find-in-files.page-down", json!(null));
                assert_eq!(h.popup()["selected"], json!(n - 1));
            }
            h.press(keys);
            let expected = match b.command.as_str() {
                "find-in-files.page-down" => n - 1,
                "find-in-files.page-up" => 0,
                _ if forward => 1,
                _ => n - 2,
            };
            assert_eq!(h.popup()["selected"], json!(expected), "{what}");
        }
        "find-in-files.open-result" => {
            h.search_in_files(" = ");
            let row = h.popup()["rows"][1].clone();
            h.invoke("find-in-files.next-result", json!(null));
            h.press(keys);
            assert_eq!(h.popup()["open"], json!(false), "{what}");
            let s = h.state();
            let active = s["active_tab"].as_u64().unwrap() as usize;
            assert_eq!(s["tabs"][active]["path"], row["path"], "{what}");
            assert!(h.editor_focused(), "{what}");
        }
        "find-in-files.edit-source" => {
            h.search_in_files(" = ");
            let row = h.popup()["rows"][1].clone();
            h.invoke("find-in-files.next-result", json!(null));
            h.press(keys);
            assert_eq!(h.popup()["open"], json!(false), "{what}");
            let s = h.state();
            let active = s["active_tab"].as_u64().unwrap() as usize;
            assert_eq!(s["tabs"][active]["path"], row["path"], "{what}");
            assert!(h.editor_focused(), "{what}");
        }
        "find.next" | "find.previous"
            if b.context.as_deref() == Some("FindInFilesPreview > Editor") =>
        {
            // The editor's Find Next in the preview: the next `=` of the
            // last search in a file ("x = 1"); the rows stay.
            h.cx.update(|cx| {
                cx.set_global(::editor::find::LastSearch {
                    text: "=".into(),
                    performed: true,
                    ..Default::default()
                })
            });
            h.search_in_files(" = ");
            let forward = b.command == "find.next";
            h.focus_preview(if forward { 0 } else { 5 });
            let selected = h.popup()["selected"].clone();
            h.press(keys);
            assert_eq!(h.preview_selection(), "=", "{what}");
            assert_eq!(h.popup()["selected"], selected, "{what}");
            h.cx.update(|cx| cx.set_global(::editor::find::LastSearch::default()));
        }
        "find-in-files.toggle-mask" => {
            h.search_in_files(" = ");
            h.press(keys);
            assert_eq!(h.popup()["mask"]["on"], json!(true), "{what}");
            h.invoke("find-in-files.toggle-mask", json!(null));
        }
        _ => return false,
    }
    true
}
