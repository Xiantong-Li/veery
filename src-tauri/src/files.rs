use std::collections::HashSet;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const MAX_ENTRIES: usize = 120_000;
const MAX_DEPTH: u8 = 16;
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

/// 只向用户开放这几组常用类型；不收录可执行文件。
pub const GROUPS: &[(&str, &[&str])] = &[
    (
        "document",
        &[
            "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "rtf", "csv", "wps", "et",
            "dps",
        ],
    ),
    (
        "image",
        &["jpg", "jpeg", "png", "gif", "bmp", "webp", "svg", "ico", "tif", "tiff"],
    ),
    (
        "media",
        &[
            "mp3", "flac", "wav", "m4a", "aac", "ogg", "mp4", "mkv", "avi", "mov", "wmv", "webm",
        ],
    ),
    ("archive", &["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "iso"]),
    (
        "code",
        &[
            "json", "xml", "yml", "yaml", "html", "css", "js", "ts", "py", "rs", "go", "java", "c",
            "cpp", "h", "sh", "ps1", "sql", "log",
        ],
    ),
];

pub struct FileEntry {
    pub name: String,
    stem_folded: String,
    pub path: PathBuf,
    pub ext: String,
    modified: u64,
}

/// 过滤未知分组并去重，保留用户选择的顺序。
pub fn normalize_types(types: &[String]) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for ty in types {
        if GROUPS.iter().any(|(id, _)| *id == ty.as_str()) && !kept.iter().any(|item| item == ty) {
            kept.push(ty.clone());
        }
    }
    kept
}

pub fn extensions_for(types: &[String]) -> HashSet<&'static str> {
    let mut set = HashSet::new();
    for (id, extensions) in GROUPS {
        if types.iter().any(|ty| ty == id) {
            set.extend(extensions.iter().copied());
        }
    }
    set
}

/// 桌面、文档、下载。文档可能被重定向到 OneDrive，这里跟随系统设置。
pub fn roots() -> Vec<PathBuf> {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads};

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    [&FOLDERID_Desktop, &FOLDERID_Documents, &FOLDERID_Downloads]
        .into_iter()
        .filter_map(known_folder)
        .collect()
}

fn known_folder(id: &windows::core::GUID) -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{IShellItem, SHGetKnownFolderItem, KF_FLAG_DEFAULT, SIGDN_FILESYSPATH};

    unsafe {
        let item: IShellItem = SHGetKnownFolderItem(id, KF_FLAG_DEFAULT, None).ok()?;
        let ptr = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        if ptr.is_null() {
            return None;
        }
        let text = String::from_utf16_lossy(ptr.as_wide());
        CoTaskMemFree(Some(ptr.as_ptr() as *const core::ffi::c_void));
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        Some(PathBuf::from(text))
    }
}

pub fn scan() -> Result<Vec<FileEntry>, String> {
    let roots = roots();
    if roots.is_empty() {
        return Err("找不到常用目录".to_string());
    }
    Ok(scan_roots(&roots))
}

pub fn scan_roots(roots: &[PathBuf]) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        collect(root, &mut entries, &mut seen, 0);
        if entries.len() >= MAX_ENTRIES {
            break;
        }
    }
    entries.truncate(MAX_ENTRIES);
    entries
}

fn collect(dir: &Path, out: &mut Vec<FileEntry>, seen: &mut HashSet<PathBuf>, depth: u8) {
    if depth > MAX_DEPTH || out.len() >= MAX_ENTRIES {
        return;
    }
    // 目录联接和符号链接可能成环，canonicalize + 去重可以挡住。
    let Ok(identity) = std::fs::canonicalize(dir) else {
        return;
    };
    if !seen.insert(identity) {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        if out.len() >= MAX_ENTRIES {
            return;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let attrs = meta.file_attributes();
        if attrs & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM | FILE_ATTRIBUTE_REPARSE_POINT) != 0
        {
            continue;
        }
        let path = entry.path();
        if meta.is_dir() {
            collect(&path, out, seen, depth + 1);
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        let Some(ext) = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
        else {
            continue;
        };
        if ext.is_empty() {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        out.push(FileEntry {
            name: name.to_string(),
            stem_folded: fold(stem),
            path,
            ext,
            modified: modified_secs(&meta),
        });
    }
}

fn modified_secs(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn fold(value: &str) -> String {
    let mut folded = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_whitespace() {
            continue;
        }
        let ch = match ch as u32 {
            code @ 0xFF01..=0xFF5E => char::from_u32(code - 0xFEE0).unwrap_or(ch),
            _ => ch,
        };
        for lower in ch.to_lowercase() {
            folded.push(lower);
        }
    }
    folded
}

/// 文件名按勾选的类型过滤；每个词都要在文件名里，前缀命中优先于包含命中。
pub fn search<'a>(
    entries: &'a [FileEntry],
    query: &str,
    types: &[String],
    limit: usize,
) -> Vec<&'a FileEntry> {
    let allowed = extensions_for(types);
    if allowed.is_empty() || limit == 0 {
        return Vec::new();
    }
    let raw = query.trim();
    let folded_query = fold(raw);
    if folded_query.is_empty() {
        return Vec::new();
    }
    let tokens: Vec<String> = raw
        .split_whitespace()
        .map(fold)
        .filter(|token| !token.is_empty())
        .collect();

    let mut best: Vec<(i32, u64, &FileEntry)> = Vec::with_capacity(limit);
    for entry in entries {
        if !allowed.contains(entry.ext.as_str()) {
            continue;
        }
        if !tokens.iter().all(|token| entry.stem_folded.contains(token)) {
            continue;
        }
        let score = if entry.stem_folded == folded_query {
            1000
        } else if entry.stem_folded.starts_with(&folded_query) {
            800
        } else if entry.stem_folded.contains(&folded_query) {
            500
        } else {
            400
        };
        let ahead = best.iter().position(|(kept_score, kept_modified, kept)| {
            score > *kept_score
                || (score == *kept_score && entry.modified > *kept_modified)
                || (score == *kept_score
                    && entry.modified == *kept_modified
                    && entry.name < kept.name)
        });
        if let Some(index) = ahead {
            best.insert(index, (score, entry.modified, entry));
            if best.len() > limit {
                best.pop();
            }
        } else if best.len() < limit {
            best.push((score, entry.modified, entry));
        }
    }
    best.into_iter().map(|(_, _, entry)| entry).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, modified: u64) -> FileEntry {
        let path = PathBuf::from(format!(r"C:\test\{name}"));
        let ext = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let stem = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default();
        FileEntry {
            name: name.to_string(),
            stem_folded: fold(stem),
            path,
            ext,
            modified,
        }
    }

    #[test]
    fn unknown_types_are_dropped_and_duplicates_collapse() {
        let types = vec![
            "document".to_string(),
            "nope".to_string(),
            "document".to_string(),
            "image".to_string(),
        ];
        assert_eq!(normalize_types(&types), vec!["document", "image"]);
        assert!(normalize_types(&[]).is_empty());
    }

    #[test]
    fn only_checked_types_are_searched() {
        let entries = vec![entry("report.pdf", 1), entry("tool.exe", 2), entry("photo.png", 3)];
        let types = vec!["document".to_string()];
        let hits = search(&entries, "o", &types, 8);
        assert!(hits.iter().all(|hit| hit.ext == "pdf" || hit.ext == "doc"));
        assert!(hits.iter().all(|hit| hit.name != "tool.exe"));
        assert!(hits.iter().all(|hit| hit.name != "photo.png"));
        assert!(search(&entries, "o", &[], 8).is_empty());
    }

    #[test]
    fn matches_chinese_and_multiple_words() {
        let entries = vec![
            entry("项目计划.docx", 1),
            entry("年度报告.docx", 2),
            entry("my report.pdf", 3),
        ];
        let types = vec!["document".to_string()];
        assert_eq!(search(&entries, "计划", &types, 8)[0].name, "项目计划.docx");
        assert_eq!(search(&entries, "report", &types, 8)[0].name, "my report.pdf");
        assert_eq!(search(&entries, "my rep", &types, 8)[0].name, "my report.pdf");
        assert!(search(&entries, "my 报告", &types, 8).is_empty());
    }

    #[test]
    fn prefix_ranks_before_contains_and_newer_wins_ties() {
        let entries = vec![
            entry("notes.txt", 10),
            entry("my notes.txt", 20),
            entry("notes 2026.txt", 30),
        ];
        let types = vec!["document".to_string()];
        let hits = search(&entries, "notes", &types, 8);
        assert_eq!(hits[0].name, "notes.txt");
        assert_eq!(hits[1].name, "notes 2026.txt");
        assert_eq!(hits[2].name, "my notes.txt");
    }

    #[test]
    fn limit_is_applied() {
        let entries = vec![
            entry("note a.txt", 1),
            entry("note b.txt", 2),
            entry("note c.txt", 3),
        ];
        let hits = search(&entries, "note", &["document".to_string()], 2);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn scan_roots_collects_supported_files_recursively() {
        let root = std::env::temp_dir().join(format!("veery-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join("alpha.txt"), b"a").unwrap();
        std::fs::write(nested.join("beta.pdf"), b"b").unwrap();
        std::fs::write(nested.join("skip.exe"), b"c").unwrap();

        let entries = scan_roots(&[root.clone()]);
        let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert!(names.contains(&"alpha.txt"));
        assert!(names.contains(&"beta.pdf"));
        assert!(names.contains(&"skip.exe"));

        let types = vec!["document".to_string()];
        let hits = search(&entries, "beta", &types, 8);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].ext, "pdf");

        let _ = std::fs::remove_dir_all(&root);
    }
}
