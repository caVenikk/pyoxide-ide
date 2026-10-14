//! Documentation: formatted text, hovers and signatures.

use std::ops::Range;

use serde::Serialize;
use text::Point;

/// How [`Markup`] text is formatted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MarkupKind {
    /// Shown as it is.
    PlainText,
    /// CommonMark, with fenced code blocks in the document's language.
    Markdown,
}

/// Documentation text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Markup {
    pub kind: MarkupKind,
    pub text: String,
}

impl Markup {
    pub fn markdown(text: impl Into<String>) -> Self {
        Self {
            kind: MarkupKind::Markdown,
            text: text.into(),
        }
    }

    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            kind: MarkupKind::PlainText,
            text: text.into(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// Documentation of the symbol at a position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hover {
    pub contents: Markup,
    /// The range of the symbol it describes.
    pub range: Option<Range<Point>>,
}

/// One parameter of a [`Signature`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParameterInfo {
    /// Where the parameter is in the signature's label: a byte range of
    /// `label`, on char boundaries (shown bold while it is the current one).
    pub label: Range<usize>,
    pub documentation: Option<Markup>,
}

/// One signature (overload) of the called function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Signature {
    /// The signature as shown (`(x: float, y: float) -> Point`).
    pub label: String,
    pub documentation: Option<Markup>,
    pub parameters: Vec<ParameterInfo>,
    /// The current parameter of this signature, when it differs from the
    /// help's.
    pub active_parameter: Option<usize>,
}

/// The signatures of the call at a position (parameter info).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SignatureHelp {
    pub signatures: Vec<Signature>,
    /// The signature that matches the arguments best.
    pub active_signature: usize,
    /// The parameter the caret is at.
    pub active_parameter: Option<usize>,
}

impl SignatureHelp {
    /// The current parameter of a signature.
    pub fn active_parameter_of(&self, signature: usize) -> Option<usize> {
        self.signatures
            .get(signature)
            .and_then(|s| s.active_parameter)
            .or(self.active_parameter)
    }
}

/// Why parameter info is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SignatureTrigger {
    /// Explicitly (Parameter Info).
    Invoked,
    /// By typing one of the server's trigger characters (`(`, `,`).
    Character(char),
    /// The caret or the text moved while the popup is shown.
    ContentChange,
}

/// The context of a signature help request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureContext {
    pub trigger: SignatureTrigger,
    /// The help shown now when the popup is open, so the server keeps the
    /// chosen signature.
    pub active: Option<SignatureHelp>,
}

impl SignatureContext {
    pub fn invoked() -> Self {
        Self {
            trigger: SignatureTrigger::Invoked,
            active: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_parameters() {
        let signature = |active| Signature {
            label: "(x, y)".into(),
            documentation: None,
            parameters: vec![
                ParameterInfo {
                    label: 1..2,
                    documentation: None,
                },
                ParameterInfo {
                    label: 4..5,
                    documentation: Some(Markup::plain("y")),
                },
            ],
            active_parameter: active,
        };
        let help = SignatureHelp {
            signatures: vec![signature(None), signature(Some(0))],
            active_signature: 0,
            active_parameter: Some(1),
        };
        assert_eq!(help.active_parameter_of(0), Some(1));
        assert_eq!(help.active_parameter_of(1), Some(0));
        assert_eq!(
            &help.signatures[0].label[help.signatures[0].parameters[1].label.clone()],
            "y"
        );
        assert!(Markup::markdown(" \n").is_empty());
        assert_eq!(
            SignatureContext::invoked().trigger,
            SignatureTrigger::Invoked
        );
    }
}
