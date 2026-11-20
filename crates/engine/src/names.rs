//! File-name rules of the system being restored to.
//!
//! Package paths are structurally safe (no `..`, no absolute paths; see
//! `split_rel_path`) but may contain names that are legal on the source OS
//! and not on the target: a Linux file may be called `notes: draft?.txt`, or
//! `CON`, or differ from a neighbour only by letter case. Each component is
//! adapted here **before** it is joined to a path on disk. On Windows this
//! is also what makes `\` and `:` inert: they never reach the filesystem as
//! a separator or a stream/drive designator.

use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameRules {
    /// Linux and other Unix-likes: any name without `/` or NUL, and names
    /// differing only by letter case are different files.
    Posix,
    /// macOS (APFS / HFS+ default): POSIX names, but case-insensitive, so
    /// `Report.txt` and `report.txt` are the same file.
    Mac,
    /// Windows (NTFS, exFAT, FAT32 as seen through Win32).
    Windows,
}

impl NameRules {
    /// Rules for the system this code is running on.
    pub fn native() -> Self {
        if cfg!(windows) {
            NameRules::Windows
        } else if cfg!(target_os = "macos") {
            NameRules::Mac
        } else {
            NameRules::Posix
        }
    }

    /// Do names differing only by letter case collide?
    pub fn case_insensitive(self) -> bool {
        matches!(self, NameRules::Windows | NameRules::Mac)
    }

    /// The name to use on the target for one path component.
    pub fn adapt(self, name: &str) -> Cow<'_, str> {
        match self {
            NameRules::Posix | NameRules::Mac => Cow::Borrowed(name),
            NameRules::Windows => windows_name(name),
        }
    }
}

const WINDOWS_FORBIDDEN: &[char] = &['<', '>', ':', '"', '|', '?', '*', '\\', '/'];

fn windows_name(name: &str) -> Cow<'_, str> {
    let mut out: Cow<str> = Cow::Borrowed(name);
    if name.chars().any(|c| WINDOWS_FORBIDDEN.contains(&c) || (c as u32) < 0x20) {
        out = Cow::Owned(
            name.chars().map(|c| if WINDOWS_FORBIDDEN.contains(&c) || (c as u32) < 0x20 { '_' } else { c }).collect(),
        );
    }
    // Windows silently strips trailing dots and spaces, which would make
    // "file." and "file" the same file. Keep them by appending '_'.
    if out.ends_with('.') || out.ends_with(' ') {
        out = Cow::Owned(format!("{out}_"));
    }
    // Device names are reserved with any extension: CON, CON.txt, com1.log.
    if is_reserved_device_name(&out) {
        out = Cow::Owned(format!("_{out}"));
    }
    out
}

fn is_reserved_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").trim_end_matches(' ').to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.chars().count() == 4
            && stem.chars().nth(3).is_some_and(|c| c.is_ascii_digit() || matches!(c, '¹' | '²' | '³')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_and_mac_keep_everything() {
        for n in ["notes: draft?.txt", "CON", "a\\b", "file."] {
            assert_eq!(NameRules::Posix.adapt(n), n);
            assert_eq!(NameRules::Mac.adapt(n), n);
        }
        assert!(!NameRules::Posix.case_insensitive());
        assert!(NameRules::Mac.case_insensitive() && NameRules::Windows.case_insensitive());
    }

    #[test]
    fn windows_rules() {
        let w = |n| NameRules::Windows.adapt(n).into_owned();
        assert_eq!(w("report.docx"), "report.docx");
        assert_eq!(w("Ærø – 日本.txt"), "Ærø – 日本.txt");
        assert_eq!(w("notes: draft?.txt"), "notes_ draft_.txt");
        assert_eq!(w("a<b>c|d*e\"f"), "a_b_c_d_e_f");
        assert_eq!(w("file."), "file._");
        assert_eq!(w("trailing "), "trailing _");
        assert_eq!(w("..."), "..._");
        assert_eq!(w("CON"), "_CON");
        assert_eq!(w("con.txt"), "_con.txt");
        assert_eq!(w("COM1.log"), "_COM1.log");
        assert_eq!(w("lpt¹"), "_lpt¹");
        assert_eq!(w("COM10"), "COM10");
        assert_eq!(w("CONSOLE.txt"), "CONSOLE.txt");
        assert_eq!(w("nul."), "_nul._");
    }

    #[test]
    fn windows_rules_neutralise_separators_and_streams() {
        // These would be traversal or an alternate data stream on Windows.
        for hostile in ["..\\..\\Windows", "C:", "evil:$DATA", "a\\b", "x/y"] {
            let adapted = NameRules::Windows.adapt(hostile);
            assert!(!adapted.contains(['\\', ':', '/']), "{hostile:?} → {adapted:?}");
        }
    }
}
