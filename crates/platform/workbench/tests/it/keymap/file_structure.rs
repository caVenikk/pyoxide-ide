//! File Structure filter shortcuts.

use super::*;

area_tests!("file-structure");

fn check(h: &mut H, b: &Binding) -> bool {
    let what = format!("{} -> {}", b.keys, b.command);
    let keys = &b.keys;
    h.reset();
    match b.command.as_str() {
        command if command.starts_with("structure.toggle-") => {
            h.set_editor("al|pha");
            h.invoke("code.file-structure", json!(null));
            let index = ["structure.toggle-first", "structure.toggle-second", "structure.toggle-third", "structure.toggle-fourth"].iter().position(|c| *c == command).unwrap();
            let before = h.invoke("code.state", json!(null))["list"]["groups"][index]["enabled"].as_bool().unwrap();
            h.press(keys);
            assert_eq!(h.invoke("code.state", json!(null))["list"]["groups"][index]["enabled"], !before, "{what}");
        }
        "menu.confirm" => {
            h.set_editor("al|pha\nx");
            h.invoke("code.file-structure", json!(null));
            h.invoke("menu.down", json!(null));
            h.press(keys);
            assert_eq!(h.editor_marked(), "alpha\n|x", "{what}");
        }
        _ => return false,
    }
    true
}
