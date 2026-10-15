//! Parameter strings ("Parameters", "Interpreter options") split into
//! arguments the way the reference IDE splits them
//! (`ParametersListUtil.parse` with its defaults; see NOTICE): whitespace
//! separates arguments, double quotes group them and are removed, `\"`
//! is a literal quote.

/// Splits a parameter string into arguments.
pub fn parse(text: &str) -> Vec<String> {
    let text = text.trim();
    let mut params = Vec::new();
    if text.is_empty() {
        return params;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut token = String::new();
    let mut in_quotes = false;
    let mut escaped_quote = false;
    let mut non_empty = false;
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '"' {
            if !escaped_quote {
                in_quotes = !in_quotes;
                non_empty = true;
                i += 1;
                continue;
            }
            escaped_quote = false;
        } else if ch.is_whitespace() {
            if !in_quotes {
                if !token.is_empty() || non_empty {
                    params.push(std::mem::take(&mut token));
                    non_empty = false;
                }
                i += 1;
                continue;
            }
        } else if ch == '\\' && i + 1 < chars.len() && chars[i + 1] == '"' {
            escaped_quote = true;
            i += 1;
            continue;
        }
        token.push(ch);
        i += 1;
    }
    if !token.is_empty() || non_empty {
        params.push(token);
    }
    params
}

/// Joins arguments into a parameter string that [`parse`] splits back
/// into them (`ParametersListUtil.join`, `escape`).
pub fn join<S: AsRef<str>>(params: &[S]) -> String {
    params
        .iter()
        .map(|p| escape(p.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn escape(arg: &str) -> String {
    let escaped = arg.replace('"', "\\\"");
    if escaped.is_empty() || escaped.chars().any(char::is_whitespace) || escaped.contains('|') {
        // A trailing backslash would escape the closing quote: it goes
        // after it.
        let trimmed = escaped.trim_end_matches('\\');
        let trailing = escaped.len() - trimmed.len();
        format!("\"{trimmed}\"{}", "\\".repeat(trailing))
    } else {
        escaped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_the_reference() {
        assert!(parse("   ").is_empty());
        assert_eq!(parse("--port 8000  -v"), ["--port", "8000", "-v"]);
        assert_eq!(parse("\"a b\" c"), ["a b", "c"]);
        assert_eq!(parse("--name=\"x y\""), ["--name=x y"]);
        assert_eq!(parse("\"\""), [""]);
        assert_eq!(parse("say \\\"hi\\\""), ["say", "\"hi\""]);
        assert_eq!(
            parse("'single quotes' stay"),
            ["'single", "quotes'", "stay"]
        );
        assert_eq!(parse("a\\b"), ["a\\b"]);
    }

    #[test]
    fn join_round_trips() {
        for args in [
            vec!["plain", "two words", ""],
            vec!["quote\"inside", "pipe|x"],
            vec!["--flag"],
        ] {
            let joined = join(&args);
            assert_eq!(parse(&joined), args, "{joined}");
        }
        assert_eq!(join(&["a b", "c"]), "\"a b\" c");
    }
}
