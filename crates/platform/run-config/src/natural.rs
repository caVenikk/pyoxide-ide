//! Natural order of names, as the reference IDE sorts run configurations,
//! their folders and their types (`NaturalComparator`, see NOTICE):
//! numbers compare by value, letters ignore case first, and equal names
//! then compare with case.

use std::cmp::Ordering;

/// Compares two names in natural order.
pub fn compare(a: &str, b: &str) -> Ordering {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    compare_chars(&a, &b, true)
}

fn compare_chars(s1: &[char], s2: &[char], ignore_case: bool) -> Ordering {
    let (len1, len2) = (s1.len(), s2.len());
    let (mut i, mut j) = (0, 0);
    while i < len1 && j < len2 {
        let (ch1, ch2) = (s1[i], s2[j]);
        let number = |c: char| c.is_ascii_digit() || c == ' ';
        if number(ch1) && number(ch2) {
            let start1 = skip(s1, skip(s1, i, ' '), '0');
            let start2 = skip(s2, skip(s2, j, ' '), '0');
            let end1 = skip_digits(s1, start1);
            let end2 = skip_digits(s2, start2);
            // More digits: a greater number.
            let by_length = (end1 - start1).cmp(&(end2 - start2));
            if by_length != Ordering::Equal {
                return by_length;
            }
            let by_digits = s1[start1..end1].cmp(&s2[start2..end2]);
            if by_digits != Ordering::Equal {
                return by_digits;
            }
            // The same number: the one with fewer leading spaces and
            // zeros first.
            let by_full = (end1 - i).cmp(&(end2 - j));
            if by_full != Ordering::Equal {
                return by_full;
            }
            let by_leading = s1[i..start1].cmp(&s2[j..start2]);
            if by_leading != Ordering::Equal {
                return by_leading;
            }
            i = end1;
            j = end2;
            continue;
        }
        let diff = compare_char(ch1, ch2, ignore_case);
        if diff != Ordering::Equal {
            return diff;
        }
        i += 1;
        j += 1;
    }
    if i < len1 {
        return Ordering::Greater;
    }
    if j < len2 {
        return Ordering::Less;
    }
    match len1.cmp(&len2) {
        Ordering::Equal if ignore_case => compare_chars(s1, s2, false),
        other => other,
    }
}

fn compare_char(ch1: char, ch2: char, ignore_case: bool) -> Ordering {
    // A space sorts after the characters between it and '0'.
    if ch1 == ' ' && ch2 > ' ' && ch2 < '0' {
        return Ordering::Greater;
    }
    if ch2 == ' ' && ch1 > ' ' && ch1 < '0' {
        return Ordering::Less;
    }
    if ignore_case {
        let a = ch1.to_uppercase().next().unwrap_or(ch1);
        let b = ch2.to_uppercase().next().unwrap_or(ch2);
        if a != b {
            let a = a.to_lowercase().next().unwrap_or(a);
            let b = b.to_lowercase().next().unwrap_or(b);
            return a.cmp(&b);
        }
        Ordering::Equal
    } else {
        ch1.cmp(&ch2)
    }
}

fn skip(s: &[char], mut at: usize, c: char) -> usize {
    while at < s.len() && s[at] == c {
        at += 1;
    }
    at
}

fn skip_digits(s: &[char], mut at: usize) -> usize {
    while at < s.len() && s[at].is_ascii_digit() {
        at += 1;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(names: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        v.sort_by(|a, b| compare(a, b));
        v
    }

    #[test]
    fn natural_order() {
        assert_eq!(
            sorted(&["task10", "Task2", "task1", "alpha", "Beta"]),
            ["alpha", "Beta", "task1", "Task2", "task10"]
        );
        assert_eq!(sorted(&["b", "B", "a"]), ["a", "B", "b"]);
        assert_eq!(sorted(&["x 2", "x 10", "x 1"]), ["x 1", "x 2", "x 10"]);
        assert_eq!(compare("Python", "Python tests"), Ordering::Less);
        assert_eq!(compare("Python tests", "Shell Script"), Ordering::Less);
        assert_eq!(compare("a01", "a1"), Ordering::Greater);
        assert_eq!(compare("same", "same"), Ordering::Equal);
    }
}
