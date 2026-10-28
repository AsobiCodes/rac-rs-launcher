//! Version numbers of mods and of their releases: `MAJOR.MINOR.PATCH` with an optional `-suffix`, the rules of the game's mod loader
//! (numbers without leading zeros, suffix of letters, digits, `.` and `-`; a suffixed version is a pre-release and sorts below the plain one).
//! A release tag may carry a leading `v` (`v0.3.3`), which is dropped. Anything else is "not a version": `parse` gives None and no
//! comparison says "newer", so a strange version text never starts an update by accident.
//! Pre-release suffixes are compared the semver way (dot parts; numbers by value, so `rc.10` is above `rc.2`; numbers below words).
use std::cmp::Ordering;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ver {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Option<String>,
}

/// `0` or a number without leading zeros.
fn num(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) || (s.len() > 1 && s.starts_with('0')) {
        return None;
    }
    s.parse().ok()
}

/// "0.3.3", "v0.3.3", "1.0.0-rc.1" -> a version; None for everything else (`1.2`, `1.2.3.4`, `01.2.3`, `1.2.3-`, `latest`, empty).
pub fn parse(text: &str) -> Option<Ver> {
    let t = text.trim();
    let t = t.strip_prefix(['v', 'V']).unwrap_or(t);
    let (core, pre) = match t.split_once('-') {
        Some((c, s)) => (c, Some(s)),
        None => (t, None),
    };
    let mut it = core.split('.');
    let (a, b, c) = (it.next()?, it.next()?, it.next()?);
    if it.next().is_some() {
        return None;
    }
    let pre = match pre {
        Some(s) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-') => Some(s.to_string()),
        Some(_) => return None,
        None => None,
    };
    Some(Ver { major: num(a)?, minor: num(b)?, patch: num(c)?, pre })
}

fn cmp_pre(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.split('.'), b.split('.'));
    loop {
        match (x.next(), y.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) => {
                let o = match (num(p), num(q)) {
                    (Some(m), Some(n)) => m.cmp(&n),
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => p.cmp(q),
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
        }
    }
}

impl Ord for Ver {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch)).then_with(|| match (&self.pre, &other.pre) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(a), Some(b)) => cmp_pre(a, b),
        })
    }
}

impl PartialOrd for Ver {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Ver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(p) = &self.pre {
            write!(f, "-{p}")?;
        }
        Ok(())
    }
}

/// Compares two version texts; None when either is not a version.
pub fn cmp_text(a: &str, b: &str) -> Option<Ordering> {
    Some(parse(a)?.cmp(&parse(b)?))
}

/// True only when both are versions and `candidate` is higher than `installed`.
pub fn is_newer(candidate: &str, installed: &str) -> bool {
    cmp_text(candidate, installed) == Some(Ordering::Greater)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_follow_the_loaders_rules() {
        for ok in ["0.3.3", "v0.3.3", "V1.0.0", "2.0.0-beta", "2.0.0-beta.1-x", " 1.2.3 "] {
            assert!(parse(ok).is_some(), "{ok}");
        }
        for bad in ["1.2", "1.2.3.4", "01.2.3", "1.02.3", "1.2.3-", "1.2.3-b@d", "a.b.c", "", "v", "1..3", "latest", "1.2.3+build", "vv1.2.3", "-1.2.3"] {
            assert!(parse(bad).is_none(), "{bad}");
        }
        assert_eq!(parse("v0.3.3").unwrap().to_string(), "0.3.3");
        assert_eq!(parse("1.0.0-rc.1").unwrap().to_string(), "1.0.0-rc.1");
    }

    #[test]
    fn order_is_numeric_and_pre_releases_sort_below_the_plain_version() {
        let lt = |a: &str, b: &str| assert!(parse(a).unwrap() < parse(b).unwrap(), "{a} < {b}");
        lt("0.3.2", "0.3.3");
        lt("0.9.9", "0.10.0");
        lt("1.9.9", "2.0.0");
        lt("1.2.0-beta", "1.2.0");
        lt("1.2.0-rc.2", "1.2.0-rc.10");
        lt("1.2.0-rc", "1.2.0-rc.1");
        lt("1.2.0-1", "1.2.0-alpha");
        lt("1.2.0-alpha", "1.2.0-beta");
        assert_eq!(cmp_text("v0.3.3", "0.3.3"), Some(Ordering::Equal));
    }

    #[test]
    fn newer_needs_two_real_versions() {
        assert!(is_newer("0.3.3", "0.3.2") && is_newer("v0.3.3", "0.3.2") && is_newer("1.0.0", "1.0.0-rc.1"));
        assert!(!is_newer("0.3.2", "0.3.2") && !is_newer("0.3.1", "0.3.2") && !is_newer("1.0.0-rc.1", "1.0.0"));
        assert!(!is_newer("nonsense", "0.3.2") && !is_newer("0.3.3", "nonsense") && !is_newer("", ""));
    }
}
