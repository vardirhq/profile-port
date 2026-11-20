//! Exclusion rules: caches and temporary data not worth migrating.
//!
//! Patterns are '/'-separated paths relative to the profile root. A `*`
//! component matches any single name (e.g. a browser profile folder).
//! Matching is case-insensitive, like Windows. An excluded folder is not
//! descended into and is reported as excluded, never silently dropped.

#[derive(Debug, Clone)]
pub struct Exclusions {
    patterns: Vec<Vec<String>>,
}

pub const DEFAULT_EXCLUSIONS: &[&str] = &[
    "AppData/Local/Temp",
    "AppData/Local/CrashDumps",
    "AppData/Local/Microsoft/Windows/INetCache",
    "AppData/Local/Microsoft/Windows/WebCache",
    "AppData/Local/Microsoft/Windows/Explorer", // thumbnail caches
    "AppData/Local/Google/Chrome/User Data/*/Cache",
    "AppData/Local/Google/Chrome/User Data/*/Code Cache",
    "AppData/Local/Google/Chrome/User Data/*/GPUCache",
    "AppData/Local/Microsoft/Edge/User Data/*/Cache",
    "AppData/Local/Microsoft/Edge/User Data/*/Code Cache",
    "AppData/Local/Microsoft/Edge/User Data/*/GPUCache",
    "AppData/Local/Mozilla/Firefox/Profiles/*/cache2",
];

impl Exclusions {
    pub fn new<S: AsRef<str>>(patterns: &[S]) -> Self {
        Self {
            patterns: patterns
                .iter()
                .map(|p| p.as_ref().split('/').filter(|s| !s.is_empty()).map(str::to_lowercase).collect())
                .collect(),
        }
    }

    pub fn none() -> Self {
        Self { patterns: Vec::new() }
    }

    /// Does this relative path match a rule exactly (not merely sit inside
    /// an excluded folder — those are never visited)?
    pub fn matches(&self, rel_path: &str) -> bool {
        let parts: Vec<String> = rel_path.split('/').map(str::to_lowercase).collect();
        self.patterns
            .iter()
            .any(|p| p.len() == parts.len() && p.iter().zip(&parts).all(|(pat, name)| pat == "*" || pat == name))
    }
}

impl Default for Exclusions {
    fn default() -> Self {
        Self::new(DEFAULT_EXCLUSIONS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching() {
        let ex = Exclusions::default();
        assert!(ex.matches("AppData/Local/Temp"));
        assert!(ex.matches("appdata/local/TEMP"));
        assert!(ex.matches("AppData/Local/Google/Chrome/User Data/Default/Cache"));
        assert!(ex.matches("AppData/Local/Google/Chrome/User Data/Profile 2/Code Cache"));
        assert!(!ex.matches("AppData/Local"));
        assert!(!ex.matches("AppData/Local/Google/Chrome/User Data/Default/Bookmarks"));
        assert!(!ex.matches("Documents/Temp"));
    }
}
