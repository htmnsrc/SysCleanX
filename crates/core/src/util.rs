//! Small helpers shared across the crate.

use std::path::Path;

/// Human readable byte size ("1.4 GB").
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value >= 99.95 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Case-insensitive wildcard match supporting `*` and `?`.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let t: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

pub fn has_wildcard(s: &str) -> bool {
    s.contains('*') || s.contains('?')
}

/// Normalized comparison key for a path: lowercase, forward slashes, no verbatim
/// prefix, no trailing separator. Windows paths are case-insensitive.
pub fn norm_key(path: &Path) -> String {
    let mut s = path.to_string_lossy().replace('\\', "/").to_lowercase();
    if let Some(rest) = s.strip_prefix("//?/") {
        s = rest.to_string();
    }
    while s.len() > 1 && s.ends_with('/') && !s.ends_with(":/") {
        s.pop();
    }
    s
}

/// True if `key` equals `base` or lies inside it (both normalized keys).
pub fn key_within(key: &str, base: &str) -> bool {
    key == base
        || (key.len() > base.len()
            && key.starts_with(base)
            && (base.ends_with('/') || key.as_bytes()[base.len()] == b'/'))
}

/// Normalize a display name for fuzzy comparisons: lowercase alphanumerics only.
pub fn norm_name(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Standard base64 (used for PowerShell -EncodedCommand).
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Split a command line into (program, raw argument string).
/// Handles `"C:\x y\a.exe" /S`, `C:\x y\a.exe /S` (unquoted path with spaces) and `prog args`.
pub fn split_command(cmd: &str) -> (String, String) {
    let cmd = cmd.trim();
    if let Some(rest) = cmd.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            return (rest[..end].to_string(), rest[end + 1..].trim().to_string());
        }
        return (rest.to_string(), String::new());
    }
    // ASCII lowercasing keeps byte offsets identical to `cmd` (full Unicode lowercasing can
    // change lengths, e.g. "İ", which would make the slice below panic).
    let lower = cmd.to_ascii_lowercase();
    if let Some(idx) = lower.find(".exe") {
        let end = idx + 4;
        if end == cmd.len() || cmd[end..].starts_with(' ') {
            return (cmd[..end].to_string(), cmd[end..].trim().to_string());
        }
    }
    match cmd.split_once(char::is_whitespace) {
        Some((p, a)) => (p.to_string(), a.trim().to_string()),
        None => (cmd.to_string(), String::new()),
    }
}

/// Expand `%VAR%` references. Returns `None` if any variable is missing or empty,
/// so a path can never silently collapse into a relative one.
pub fn expand_env(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('%')?;
        let name = &after[..end];
        let value = std::env::var(name).ok().filter(|v| !v.trim().is_empty())?;
        out.push_str(value.trim_end_matches(['\\', '/']));
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// Unix seconds -> [year, month, day, hour, minute, second] (UTC, Howard Hinnant's algorithm).
pub fn civil(secs: i64) -> [u16; 6] {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    [y as u16, m as u16, d as u16, (rem / 3600) as u16, (rem % 3600 / 60) as u16, (rem % 60) as u16]
}

/// `YYYY-MM-DD` for a file time.
pub fn date_string(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let [y, m, d, ..] = civil(secs);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards() {
        assert!(wildcard_match("thumbcache_*.db", "ThumbCache_1024.DB"));
        assert!(wildcard_match("*.log", "a.log"));
        assert!(!wildcard_match("*.log", "a.log.txt"));
        assert!(wildcard_match("profile ?", "Profile 1"));
        assert!(wildcard_match("*", ""));
        assert!(!wildcard_match("a*b", "acd"));
    }

    #[test]
    fn keys() {
        assert_eq!(norm_key(Path::new("C:\\Windows\\")), "c:/windows");
        assert_eq!(norm_key(Path::new("\\\\?\\C:\\Temp")), "c:/temp");
        assert_eq!(norm_key(Path::new("C:\\")), "c:/");
        assert!(key_within("c:/windows/temp", "c:/windows"));
        assert!(!key_within("c:/windowsold", "c:/windows"));
        assert!(key_within("c:/x", "c:/"));
    }

    #[test]
    fn commands() {
        assert_eq!(
            split_command(r#""C:\Program Files\A\unins000.exe" /SILENT"#),
            (r"C:\Program Files\A\unins000.exe".into(), "/SILENT".into())
        );
        assert_eq!(
            split_command(r"C:\Program Files\A\u.exe --x"),
            (r"C:\Program Files\A\u.exe".into(), "--x".into())
        );
        assert_eq!(
            split_command("MsiExec.exe /I{ABC}"),
            ("MsiExec.exe".into(), "/I{ABC}".into())
        );
        assert_eq!(
            split_command("C:\\İnstaller\\Ünïcode App\\setup.EXE /x"),
            ("C:\\İnstaller\\Ünïcode App\\setup.EXE".into(), "/x".into())
        );
        assert_eq!(
            split_command("RunDll32 C:\\x.dll,Uninstall"),
            ("RunDll32".into(), "C:\\x.dll,Uninstall".into())
        );
    }

    #[test]
    fn env_expansion() {
        std::env::set_var("SCX_TEST_VAR", "/tmp/base/");
        assert_eq!(expand_env("%SCX_TEST_VAR%/x").as_deref(), Some("/tmp/base/x"));
        assert_eq!(expand_env("%SCX_MISSING_VAR%/x"), None);
        assert_eq!(expand_env("plain"), Some("plain".into()));
    }

    #[test]
    fn byte_formatting() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(412 << 30), "412 GB");
        assert_eq!(format_bytes(6_920_000_000), "6.4 GB");
    }

    #[test]
    fn dates() {
        assert_eq!(civil(0), [1970, 1, 1, 0, 0, 0]);
        assert_eq!(civil(1_709_210_096), [2024, 2, 29, 12, 34, 56]);
    }

    #[test]
    fn b64() {
        assert_eq!(base64(b"hello"), "aGVsbG8=");
        assert_eq!(base64(b"hi"), "aGk=");
        assert_eq!(base64(b"abc"), "YWJj");
    }
}
