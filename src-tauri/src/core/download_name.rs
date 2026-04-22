//! 下载文件名：由上传名派生，与输出格式扩展名一致（如 `报告.docx` → `报告.md`）。
use std::fmt::Write;
use std::path::Path;

/// 常用输出格式在下载时使用更常见的扩展名。
pub fn download_file_extension(output_format: &str) -> String {
    match output_format {
        "markdown" => "md".to_string(),
        other => other.to_string(),
    }
}

/// 从浏览器上传的 `file_name` 与 `output_format` 生成建议的下载文件名（仅 basename，无路径）。
pub fn derive_result_download_filename(
    original_upload_name: Option<&str>,
    output_format: &str,
) -> String {
    let ext = download_file_extension(output_format);
    let stem = original_upload_name
        .and_then(|name| {
            let path = Path::new(name);
            path.file_name().and_then(|p| p.to_str()).map(|base| {
                Path::new(base)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(base)
            })
        })
        .unwrap_or("converted");
    let stem = sanitize_filename_stem(stem);
    let stem = if stem.is_empty() {
        "converted".to_string()
    } else {
        stem
    };
    let stem = stem_avoid_windows_device_name(&stem);
    format!("{stem}.{ext}")
}

/// 任务列表/卡片展示：原始上传名的 **basename**（去掉路径），不含敏感目录信息；过长截断。
pub fn input_file_label_for_task(original_upload_name: Option<&str>) -> Option<String> {
    let raw = original_upload_name?.trim();
    if raw.is_empty() {
        return None;
    }
    let base = Path::new(raw)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(raw);
    const MAX: usize = 200;
    let n = base.chars().count();
    if n <= MAX {
        Some(base.to_string())
    } else {
        Some(format!(
            "{}…",
            base.chars().take(MAX.saturating_sub(1)).collect::<String>()
        ))
    }
}

/// 在目录 `dir` 下为 `desired`（仅 basename）找一个不冲突的文件名：若已存在则使用 `stem (1).ext`、`stem (2).ext`…
pub fn unique_filename_in_dir(dir: &Path, desired: &str) -> String {
    let candidate = dir.join(desired);
    if !candidate.exists() {
        return desired.to_string();
    }
    let path = Path::new(desired);
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("converted");
    let mut n = 1u32;
    loop {
        let name = if ext.is_empty() {
            format!("{stem} ({n})")
        } else {
            format!("{stem} ({n}).{ext}")
        };
        if !dir.join(&name).exists() {
            return name;
        }
        n = n.saturating_add(1);
        if n > 10_000 {
            use std::time::{SystemTime, UNIX_EPOCH};
            let t = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            return if ext.is_empty() {
                format!("{stem}_{t}")
            } else {
                format!("{stem}_{t}.{ext}")
            };
        }
    }
}

/// Windows 设备保留名（如 `CON`、`COM1`）不能作为文件名主体，加前缀避免创建失败。
fn stem_avoid_windows_device_name(stem: &str) -> String {
    let u = stem.trim().to_ascii_uppercase();
    if is_windows_reserved_base(&u) {
        format!("_{stem}")
    } else {
        stem.to_string()
    }
}

fn is_windows_reserved_base(u: &str) -> bool {
    matches!(u, "CON" | "PRN" | "AUX" | "NUL")
        || u.strip_prefix("COM")
            .and_then(|s| s.parse::<u8>().ok())
            .is_some_and(|n| n <= 9)
        || u.strip_prefix("LPT")
            .and_then(|s| s.parse::<u8>().ok())
            .is_some_and(|n| n <= 9)
}

const MAX_STEM_CHARS: usize = 180;

fn sanitize_filename_stem(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars().take(MAX_STEM_CHARS) {
        let rep = match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        };
        out.push(rep);
    }
    out.trim_matches(|c: char| c.is_whitespace()).to_string()
}

/// `Content-Disposition: attachment`，同时带 ASCII `filename` 与 RFC 5987 `filename*`（UTF-8）。
pub fn content_disposition_attachment(filename: &str) -> String {
    let ext = filename
        .rsplit_once('.')
        .map(|(_, e)| e)
        .filter(|e| !e.is_empty() && !e.contains('/'))
        .unwrap_or("bin");
    let legacy = if filename.is_ascii() {
        escape_legacy_disposition_filename(filename)
    } else {
        format!("download.{ext}")
    };
    let star = encode_rfc5987_value(filename);
    format!(r#"attachment; filename="{legacy}"; filename*=UTF-8''{star}"#)
}

fn escape_legacy_disposition_filename(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        match c {
            '\\' => o.push_str(r"\\"),
            '"' => o.push_str(r#"\""#),
            c if c.is_control() || (c as u32) == 0x7f => o.push('_'),
            c => o.push(c),
        }
    }
    o
}

fn is_rfc5987_attr_char(b: u8) -> bool {
    matches!(
        b,
        b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'!'
            | b'#'
            | b'$'
            | b'&'
            | b'+'
            | b'-'
            | b'.'
            | b'^'
            | b'_'
            | b'`'
            | b'|'
            | b'~'
    )
}

fn encode_rfc5987_value(s: &str) -> String {
    let mut out = String::new();
    for &b in s.as_bytes() {
        if is_rfc5987_attr_char(b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{:02X}", b);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_simple() {
        assert_eq!(
            derive_result_download_filename(Some("Notes.docx"), "markdown"),
            "Notes.md"
        );
    }

    #[test]
    fn derive_nested_path() {
        assert_eq!(
            derive_result_download_filename(Some("folder/报告.docx"), "markdown"),
            "报告.md"
        );
    }

    #[test]
    fn input_label_basename_only() {
        assert_eq!(
            input_file_label_for_task(Some("folder/sub/报告.pdf")).as_deref(),
            Some("报告.pdf")
        );
    }

    #[test]
    fn input_label_none_for_empty() {
        assert_eq!(input_file_label_for_task(Some("   ")), None);
        assert_eq!(input_file_label_for_task(None), None);
    }

    #[test]
    fn derive_no_name() {
        assert_eq!(
            derive_result_download_filename(None, "markdown"),
            "converted.md"
        );
    }

    #[test]
    fn derive_windows_reserved_stem() {
        assert_eq!(
            derive_result_download_filename(Some("CON.docx"), "markdown"),
            "_CON.md"
        );
    }

    #[test]
    fn unique_when_free_returns_same() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(unique_filename_in_dir(dir.path(), "out.md"), "out.md");
    }

    #[test]
    fn unique_when_taken_appends_counter() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("out.md"), b"x").unwrap();
        assert_eq!(unique_filename_in_dir(dir.path(), "out.md"), "out (1).md");
        std::fs::write(dir.path().join("out (1).md"), b"x").unwrap();
        assert_eq!(unique_filename_in_dir(dir.path(), "out.md"), "out (2).md");
    }

    #[test]
    fn disposition_has_star_param() {
        let h = content_disposition_attachment("报告.md");
        assert!(h.starts_with("attachment;"));
        assert!(h.contains("filename*=UTF-8''"));
    }
}
