use std::cmp::Ordering;

/// PEP 503 normalized project name: lowercase, runs of `-`, `_` and `.`
/// collapsed to a single `-`.
pub fn normalize_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut sep = false;
    for c in name.trim().chars() {
        if matches!(c, '-' | '_' | '.') {
            sep = true;
        } else {
            if sep && !out.is_empty() {
                out.push('-');
            }
            sep = false;
            out.push(c.to_ascii_lowercase());
        }
    }
    out
}

/// epoch, release, pre, post, dev, local: the PEP 440 sort key.
type SortKey<'a> = (u64, Vec<u64>, (i8, u64), (i8, u64), (i8, u64), &'a str);

/// A PEP 440 version, enough to order releases and to tell pre-releases
/// from final ones. Local version labels are compared as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    epoch: u64,
    release: Vec<u64>,
    /// 0 = alpha, 1 = beta, 2 = release candidate.
    pre: Option<(u8, u64)>,
    post: Option<u64>,
    dev: Option<u64>,
    local: String,
}

impl Version {
    /// Parse the PEP 440 forms seen on package indexes, including the
    /// alternative spellings (`1.0-alpha.1`, `1.0.rev2`, `1.0-1`).
    pub fn parse(text: &str) -> Option<Version> {
        let s = text.trim().to_ascii_lowercase();
        let s = s.strip_prefix('v').unwrap_or(&s);
        let (s, local) = match s.split_once('+') {
            Some((a, b)) => (a, b.to_owned()),
            None => (s, String::new()),
        };
        let b = s.as_bytes();
        let mut i = 0;

        let mut epoch = 0;
        if let Some(bang) = s.find('!') {
            epoch = s[..bang].parse().ok()?;
            i = bang + 1;
        }

        let mut release = Vec::new();
        loop {
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if start == i {
                return None;
            }
            release.push(s[start..i].parse().ok()?);
            if i < b.len() && b[i] == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit) {
                i += 1;
            } else {
                break;
            }
        }

        let (mut pre, mut post, mut dev) = (None, None, None);
        while i < b.len() {
            if matches!(b[i], b'.' | b'-' | b'_') {
                i += 1;
                // "1.0-1": a bare number after a dash is an implicit post release.
                if b[i - 1] == b'-' && b.get(i).is_some_and(u8::is_ascii_digit) {
                    let (n, next) = number(s, i);
                    post = Some(n);
                    i = next;
                    continue;
                }
            }
            let start = i;
            while i < b.len() && b[i].is_ascii_alphabetic() {
                i += 1;
            }
            if start == i {
                return None;
            }
            let word = &s[start..i];
            if i < b.len()
                && matches!(b[i], b'.' | b'-' | b'_')
                && b.get(i + 1).is_some_and(u8::is_ascii_digit)
            {
                i += 1;
            }
            let (n, next) = number(s, i);
            i = next;
            match word {
                "a" | "alpha" if pre.is_none() => pre = Some((0, n)),
                "b" | "beta" if pre.is_none() => pre = Some((1, n)),
                "c" | "rc" | "pre" | "preview" if pre.is_none() => pre = Some((2, n)),
                "post" | "rev" | "r" if post.is_none() => post = Some(n),
                "dev" if dev.is_none() => dev = Some(n),
                _ => return None,
            }
        }
        Some(Version {
            epoch,
            release,
            pre,
            post,
            dev,
            local,
        })
    }

    pub fn is_prerelease(&self) -> bool {
        self.pre.is_some() || self.dev.is_some()
    }

    fn key(&self) -> SortKey<'_> {
        let mut release = self.release.clone();
        while release.len() > 1 && release.last() == Some(&0) {
            release.pop();
        }
        let pre = match (self.pre, self.post, self.dev) {
            (Some((k, n)), _, _) => (k as i8, n),
            (None, None, Some(_)) => (-1, 0),
            (None, _, _) => (3, 0),
        };
        let post = self.post.map_or((0, 0), |n| (1, n));
        let dev = self.dev.map_or((1, 0), |n| (0, n));
        (self.epoch, release, pre, post, dev, &self.local)
    }
}

fn number(s: &str, from: usize) -> (u64, usize) {
    let b = s.as_bytes();
    let mut i = from;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    (s[from..i].parse().unwrap_or(0), i)
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key().cmp(&other.key())
    }
}

/// Order version strings, newest first; strings that are not PEP 440
/// versions go last in their original order.
pub fn sort_newest_first(versions: &mut [String]) {
    versions.sort_by(|a, b| match (Version::parse(a), Version::parse(b)) {
        (Some(x), Some(y)) => y.cmp(&x),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap_or_else(|| panic!("cannot parse {s}"))
    }

    #[test]
    fn normalizes_names() {
        assert_eq!(normalize_name("Friendly-Bard"), "friendly-bard");
        assert_eq!(normalize_name("Friendly.Bard"), "friendly-bard");
        assert_eq!(normalize_name("FRIENDLY_bard"), "friendly-bard");
        assert_eq!(normalize_name("friendly-.-_bard"), "friendly-bard");
        assert_eq!(normalize_name("  requests "), "requests");
        assert_eq!(normalize_name("zope.interface"), "zope-interface");
    }

    #[test]
    fn orders_like_pep_440() {
        let ordered = [
            "1.0.dev1",
            "1.0a1",
            "1.0a2.dev1",
            "1.0a2",
            "1.0b1",
            "1.0rc1",
            "1.0",
            "1.0.post1",
            "1.0.post2.dev1",
            "1.0.post2",
            "1.1.dev1",
            "1.1",
            "1.10",
            "2!0.1",
        ];
        for pair in ordered.windows(2) {
            assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
        }
    }

    #[test]
    fn equal_after_zero_padding_and_spelling() {
        assert_eq!(v("1.0").cmp(&v("1.0.0")), Ordering::Equal);
        assert_eq!(v("1.0-alpha.1").cmp(&v("1.0a1")), Ordering::Equal);
        assert_eq!(v("1.0.rev3").cmp(&v("1.0.post3")), Ordering::Equal);
        assert_eq!(v("1.0-3").cmp(&v("1.0.post3")), Ordering::Equal);
        assert_eq!(v("V1.0RC1").cmp(&v("1.0rc1")), Ordering::Equal);
        assert_eq!(v("1.0c2").cmp(&v("1.0rc2")), Ordering::Equal);
    }

    #[test]
    fn prerelease_flag() {
        assert!(v("1.0rc1").is_prerelease());
        assert!(v("1.0.dev3").is_prerelease());
        assert!(v("3.15.0b3").is_prerelease());
        assert!(!v("1.0.post1").is_prerelease());
        assert!(!v("2024.8.30").is_prerelease());
    }

    #[test]
    fn rejects_garbage() {
        for bad in ["", "abc", "1..0", "1.0.foo", "1.0.dev1.dev2"] {
            assert!(Version::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn sorts_newest_first_with_unparseable_last() {
        let mut vs: Vec<String> = ["1.0", "weird", "2.0rc1", "1.9", "2.0", "1.10"]
            .into_iter()
            .map(String::from)
            .collect();
        sort_newest_first(&mut vs);
        assert_eq!(vs, ["2.0", "2.0rc1", "1.10", "1.9", "1.0", "weird"]);
    }
}
