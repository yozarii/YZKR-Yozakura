//! What a download's file name says: whether it is a macOS installable, how good a match for this Mac, which release,
//! which product. Nothing here reads a page or the network; it is all arithmetic on names.

/// A version without its leading `v`: tags say `v1.2.3`, feeds say `1.2.3`.
pub fn norm_version(v: &str) -> String {
    v.trim().trim_start_matches(['v', 'V']).to_string()
}

/// Simple glob: `*` matches anything; case-insensitive.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.to_lowercase(), text.to_lowercase());
    let parts: Vec<&str> = p.split('*').collect();
    let mut pos = 0usize;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !t.starts_with(part) {
                return false;
            }
            pos = part.len();
        } else if i == parts.len() - 1 {
            return t.len() >= pos + part.len() && t.ends_with(part);
        } else {
            match t[pos..].find(part) {
                Some(f) => pos += f + part.len(),
                None => return false,
            }
        }
    }
    true
}

/// Score a download name for this Mac. `None` means not a macOS installable at all.
pub fn asset_score(name: &str) -> Option<i32> {
    let n = name.to_lowercase();
    let ext = if n.ends_with(".dmg") {
        30
    } else if n.ends_with(".zip") {
        20
    } else if n.ends_with(".pkg") {
        10
    } else if n.ends_with(".tar.gz") || n.ends_with(".tgz") {
        5
    } else {
        return None;
    };
    // Other operating systems, source, and files that are not the app itself (debug symbols, builds, helpers).
    if ["uninstall", "updater", "agent", "helper"]
        .iter()
        .any(|w| n.contains(w))
    {
        return None;
    }
    if [
        "linux", "windows", "win32", "win64", ".exe", "freebsd", "-src", "source", "dsym",
        "symbols", "-debug", "_debug", ".debug", "-dbg",
    ]
    .iter()
    .any(|w| n.contains(w))
    {
        return None;
    }
    // Whole words in the name that mean Windows ("winmerge-2.16-exe.zip", "app-win.zip"), unlike substrings of other words.
    if n.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| ["exe", "msi", "win", "win32", "win64"].contains(&w))
    {
        return None;
    }
    let mut s = ext;
    // Not the stable release: still installable when it is all there is, but never preferred.
    if [
        "nightly", "-beta", ".beta", "-alpha", ".alpha", "-rc", "-dev", "preview", "canary",
    ]
    .iter()
    .any(|w| n.contains(w))
    {
        s -= 12;
    }
    if ["arm64", "aarch64", "apple-silicon", "applesilicon"]
        .iter()
        .any(|w| n.contains(w))
    {
        s += 8;
    }
    if n.contains("universal") {
        s += 16; // runs natively on both; preferred over a single-architecture build of the same release
    } else if ["macos", "mac", "darwin", "osx"]
        .iter()
        .any(|w| n.contains(w))
    {
        s += 4;
    }
    if ["x86_64", "x64", "intel", "amd64", "i386"]
        .iter()
        .any(|w| n.contains(w))
        && !n.contains("universal")
    {
        s -= 50;
    }
    Some(s)
}

/// A link whose own words say it is for another system: "Download for Windows", "Linux (x64)". File names often carry
/// no system at all (`putty-0.81.zip`), so the words around the link are the evidence. A text that also names the
/// Mac ("Windows, macOS, Linux") is kept.
pub fn not_for_mac(text: &str) -> bool {
    let t = text.to_lowercase();
    let other = [
        "windows", "win32", "win64", "for win", "linux", "ubuntu", "debian", "fedora", "android",
        "chromeos",
    ]
    .iter()
    .any(|w| t.contains(w));
    let mac = ["mac", "osx", "os x", "darwin", "apple silicon", "universal"]
        .iter()
        .any(|w| t.contains(w));
    other && !mac
}

/// The first version-like number in a file name (`Foo-4.6.10.dmg` is [4, 6, 10]): digits separated by dots, at least
/// one dot, not glued to letters (`x86.64`, `arm64` and `macos11` are not versions).
pub fn version_of(name: &str) -> Option<Vec<u32>> {
    let b = name.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < b.len()
            && (b[j].is_ascii_digit()
                || (b[j] == b'.' && j + 1 < b.len() && b[j + 1].is_ascii_digit()))
        {
            j += 1;
        }
        let run = &name[i..j];
        let dots = run.matches('.').count();
        let glued = i > 0 && b[i - 1].is_ascii_alphabetic();
        // "Rectangle2.0.5" is a version; "x86.64" and "macos11" are not (glued to letters with fewer than two dots).
        if dots >= 1 && (!glued || dots >= 2) && (i == 0 || !b[i - 1].is_ascii_digit()) {
            return run.split('.').map(|p| p.parse().ok()).collect();
        }
        i = j;
    }
    None
}

/// What product a file is, with the release and the platform words taken away, so `Foo 4.6.dmg` and
/// `Foo 4.4 Arm 64.dmg` are the same product while `Foo Pro 3.1.dmg` is another one.
pub fn product_stem(name: &str) -> String {
    let lower = name.to_lowercase();
    let no_ext = [".tar.gz", ".dmg", ".zip", ".pkg", ".tgz"]
        .iter()
        .find_map(|e| lower.strip_suffix(e))
        .unwrap_or(&lower);
    let mut text = no_ext.to_string();
    if let Some(v) = version_of(name) {
        let run: String = v.iter().map(u32::to_string).collect::<Vec<_>>().join(".");
        text = text.replacen(&run, " ", 1);
    }
    const PLATFORM: [&str; 16] = [
        "arm64",
        "arm",
        "aarch64",
        "x86",
        "x64",
        "64",
        "amd64",
        "intel",
        "silicon",
        "apple",
        "universal",
        "mac",
        "macos",
        "osx",
        "darwin",
        "x86_64",
    ];
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !PLATFORM.contains(w))
        .collect::<Vec<_>>()
        .join("-")
}

/// Index of the best download among `names`, honouring an optional glob. First wins ties.
pub fn pick_asset(names: &[String], pattern: Option<&str>) -> Option<usize> {
    let mut best: Option<(usize, i32)> = None;
    for (i, n) in names.iter().enumerate() {
        if pattern.is_some_and(|p| !glob_match(p, n)) {
            continue;
        }
        let Some(s) = asset_score(n) else { continue };
        if best.is_none_or(|(_, b)| s > b) {
            best = Some((i, s));
        }
    }
    best.map(|(i, _)| i)
}

/// Generalise a download's file name so the next version still matches: `Ghost-1.2-arm64.dmg` becomes `Ghost-*.dmg`.
pub fn link_pattern(filename: &str) -> String {
    let lower = filename.to_lowercase();
    let ext = [".tar.gz", ".dmg", ".zip", ".pkg", ".tgz"]
        .into_iter()
        .find(|e| lower.ends_with(e))
        .unwrap_or("");
    let stem = &filename[..filename.len() - ext.len()];
    let head: String = stem.chars().take_while(|c| !c.is_ascii_digit()).collect();
    format!("{head}*{ext}")
}

/// The last path segment of a URL, without query or fragment: the name a download is saved under.
pub fn file_name(url: &str) -> String {
    let no_query = url.split(['?', '#']).next().unwrap_or(url);
    no_query
        .rsplit('/')
        .next()
        .unwrap_or("download")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn globs() {
        assert!(glob_match("*.dmg", "Zen-1.0.dmg"));
        assert!(glob_match("zen-*-mac.zip", "Zen-1.0-mac.zip"));
        assert!(!glob_match("*.dmg", "Zen.zip"));
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn prefers_arm64_dmg_over_intel_and_other_oses() {
        let names = s(&[
            "app-linux-x64.tar.gz",
            "App-x86_64.dmg",
            "App-arm64.dmg",
            "App-arm64.zip",
            "App.exe",
            "checksums.txt",
        ]);
        assert_eq!(pick_asset(&names, None), Some(2));
    }

    #[test]
    fn debug_symbols_and_debug_builds_are_not_the_app() {
        for n in [
            "Ghost-3.0-arm64-dSYM.zip",
            "Ghost-3.0-symbols.zip",
            "Ghost-3.0-arm64-debug.zip",
            "ghost_debug.dmg",
            "Ghost-3.0.debug.tar.gz",
        ] {
            assert_eq!(asset_score(n), None, "{n}");
        }
        assert!(asset_score("Ghost-3.0-arm64.zip").is_some());
    }

    #[test]
    fn nightly_and_beta_builds_never_beat_the_stable_one_but_are_usable_alone() {
        let stable = asset_score("Ghost-3.0.2-arm64.dmg").unwrap();
        for n in [
            "Ghost-nightly-arm64.dmg",
            "Ghost-3.1.0-beta1-arm64.dmg",
            "Ghost-3.1-rc1-arm64.dmg",
            "Ghost-canary-arm64.dmg",
            "Ghost-preview-arm64.dmg",
        ] {
            let s = asset_score(n).unwrap();
            assert!(stable - s >= 8, "{n}: {s} vs {stable}");
        }
        assert_eq!(
            pick_asset(&["Ghost-nightly-arm64.dmg".to_string()], None),
            Some(0)
        );
    }

    #[test]
    fn universal_beats_intel_only() {
        let names = s(&["App-intel.dmg", "App-universal.zip"]);
        assert_eq!(pick_asset(&names, None), Some(1));
    }

    #[test]
    fn pattern_filters_and_nothing_matching_is_none() {
        let names = s(&["App-arm64.dmg", "App-arm64.zip"]);
        assert_eq!(pick_asset(&names, Some("*.zip")), Some(1));
        assert_eq!(pick_asset(&names, Some("*.pkg")), None);
        assert_eq!(pick_asset(&s(&["notes.txt"]), None), None);
    }

    #[test]
    fn link_patterns_generalise_a_file_name() {
        assert_eq!(link_pattern("Ghost-1.2.3-arm64.dmg"), "Ghost-*.dmg");
        assert_eq!(link_pattern("Ghost.zip"), "Ghost*.zip");
        assert_eq!(link_pattern("tool-1.0.tar.gz"), "tool-*.tar.gz");
        assert!(glob_match(
            &link_pattern("Ghost-1.2.3-arm64.dmg"),
            "Ghost-9.9-universal.dmg"
        ));
    }

    #[test]
    fn versions_lose_their_v() {
        assert_eq!(norm_version("v1.2.3"), "1.2.3");
    }

    #[test]
    fn versions_are_read_from_file_names_but_not_from_architectures() {
        assert_eq!(version_of("Wireshark 4.6.10.dmg"), Some(vec![4, 6, 10]));
        assert_eq!(
            version_of("audacity-macOS-4.0.2-arm64.dmg"),
            Some(vec![4, 0, 2])
        );
        assert_eq!(version_of("AppCleaner_3.6.8.zip"), Some(vec![3, 6, 8]));
        assert_eq!(version_of("Ghost-arm64.dmg"), None);
        assert_eq!(version_of("Tool-x86.64.zip"), None);
        assert_eq!(version_of("Itsycal.zip"), None);
    }

    #[test]
    fn helpers_updaters_and_uninstallers_are_not_the_app() {
        for n in [
            "InstallFigmaAgent.dmg",
            "Ghost-Updater.zip",
            "Uninstall-Ghost.dmg",
            "Ghost-Helper.pkg",
        ] {
            assert_eq!(asset_score(n), None, "{n}");
        }
        assert!(asset_score("Ghost-1.0-arm64.dmg").is_some());
    }
}
