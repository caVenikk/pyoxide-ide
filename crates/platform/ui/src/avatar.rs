//! A coloured square with initials, as the reference draws a project that
//! has no icon of its own (`AvatarIcon`, `AvatarUtils` of IntelliJ
//! Platform Community, Apache-2.0): a gradient from the bottom left corner
//! (the end colour) to the top right one (the start colour), corners of
//! 0.4 of the size (`arcRatio`, a diameter), white initials in JetBrains
//! Mono of 13 at 20 (`13 * size / 20`), centred. The reference asks for
//! a semibold weight but draws the regular face's strokes; so do we.

use gpui::{
    AnyElement, FontWeight, Hsla, IntoElement, SharedString, div, linear_color_stop,
    linear_gradient, prelude::*, px,
};

/// The initials and the gradient of an avatar.
#[derive(Debug, Clone, PartialEq)]
pub struct Avatar {
    pub text: SharedString,
    /// The top right colour (`color1`, `Avatar.Start`).
    pub start: Hsla,
    /// The bottom left colour (`color2`, `Avatar.End`).
    pub end: Hsla,
}

impl Avatar {
    /// The avatar of `name`: its [`initials`] on the gradient.
    pub fn new(name: &str, start: Hsla, end: Hsla) -> Self {
        Self {
            text: initials(name).into(),
            start,
            end,
        }
    }

    /// The avatar `size` wide and high.
    pub fn render(&self, size: f32) -> AnyElement {
        let font = (13.0 * size / 20.0).floor();
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(size))
            .rounded(px(0.4 * size / 2.0))
            .bg(linear_gradient(
                45.0,
                linear_color_stop(self.end, 0.0),
                linear_color_stop(self.start, 1.0),
            ))
            .text_color(gpui::white())
            .font_family(crate::fonts::MONO_FAMILY)
            .font_weight(FontWeight::NORMAL)
            .text_size(px(font))
            .line_height(px(size))
            .whitespace_nowrap()
            .child(self.text.clone())
            .into_any_element()
    }
}

/// The initials of a name, upper case (`AvatarUtils.initials`): two
/// capitals of a camel case word ("MyProject" is "MP"); else the first
/// and the last word, split at spaces, else at commas, hyphens,
/// underscores, dots or quotes ("my-sample-app" is "MA"); else the first
/// letter ("demo" is "D"). Only letters and digits count in a word.
pub fn initials(name: &str) -> String {
    let camel = camel_case_initials(name);
    if camel.chars().count() == 2 {
        return camel;
    }
    let trimmed = name.trim();
    let words = [
        &[' '][..],
        &[','],
        &['-'],
        &['_'],
        &['.'],
        &['`', '\'', '"'],
    ]
    .iter()
    .find_map(|delimiters| words(trimmed, delimiters));
    match words {
        Some(words) => {
            let first = words.first().and_then(|w| w.chars().next());
            let last = words.last().and_then(|w| w.chars().next());
            first
                .into_iter()
                .chain(last)
                .flat_map(char::to_uppercase)
                .collect()
        }
        None => camel,
    }
}

/// The first char and the upper case letters after it of the first word
/// (letters and digits, from the first letter on), at most two, upper case
/// (`generateFromCamelCase`).
fn camel_case_initials(text: &str) -> String {
    text.chars()
        .skip_while(|c| !c.is_alphabetic())
        .take_while(|c| c.is_alphanumeric())
        .enumerate()
        .filter(|(ix, c)| *ix == 0 || c.is_uppercase())
        .map(|(_, c)| c)
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}

/// The parts of `text` between `delimiters`, each kept to its letters and
/// digits, the empty ones left out; none unless there are two or more
/// (`splitAtLeast2NonEmpty`).
fn words(text: &str, delimiters: &[char]) -> Option<Vec<String>> {
    let words: Vec<String> = text
        .split(|c| delimiters.contains(&c))
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect();
    (words.len() >= 2).then_some(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_as_the_reference_makes_them() {
        // One word: its first letter.
        assert_eq!(initials("demo"), "D");
        assert_eq!(initials("Welcome"), "W");
        // Camel case: two capitals.
        assert_eq!(initials("MyProject"), "MP");
        assert_eq!(initials("sampleApp"), "SA");
        // Words: the first and the last.
        assert_eq!(initials("my-sample-app"), "MA");
        assert_eq!(initials("test-proj"), "TP");
        assert_eq!(initials("sample_at"), "SA");
        assert_eq!(initials("my_project.app"), "MP");
        assert_eq!(initials("First Project, Second"), "FS");
        assert_eq!(initials("ёлка дом"), "ЁД");
        // Spaces split before hyphens.
        assert_eq!(initials("big-data tools"), "BT");
        // Nothing to take.
        assert_eq!(initials(""), "");
        assert_eq!(initials("--"), "");
        assert_eq!(initials("42"), "");
        assert_eq!(initials("4-2"), "42");
    }
}
