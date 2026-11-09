//! 把正在看的文件夹导出成 CSV：每行一个文件夹或文件，拿到表格里排序、筛选、做图都方便。

use super::{Report, StoragePage, user_path};
use pc_specs_core::space::{Category, Kind, Node};
use std::path::PathBuf;

/// 比这小的不导出：整块磁盘有几十万项，导出来既用不上，文件也太大。
const MIN_BYTES: u64 = 1 << 20;
/// 最多导出多少行。
const MAX_ROWS: usize = 200_000;

impl StoragePage {
    /// 让用户选个位置，再把 CSV 存过去。
    pub(super) fn export_csv(&mut self, cx: &mut gpui_kit::Context<Self>) {
        let Some(report) = self.report() else { return };
        let (name, bytes) = (file_name(report), csv(report).into_bytes());
        self.app.update(cx, |state, cx| {
            state.save_with_dialog(name, false, move || Ok(bytes), cx)
        });
    }
}

/// 建议的文件名：`位置名-space-日期.csv`，位置名里不能当文件名的字符换成下划线。
fn file_name(report: &Report) -> String {
    let label: String = report
        .target
        .label
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    format!(
        "{label}-space-{}.csv",
        chrono::Local::now().format("%Y-%m-%d")
    )
}

fn category_key(category: Category) -> &'static str {
    match category {
        Category::Video => "video",
        Category::Audio => "audio",
        Category::Image => "image",
        Category::Document => "document",
        Category::Archive => "archive",
        Category::Program => "program",
        Category::Code => "code",
        Category::Other => "other",
    }
}

/// CSV 里的一个字段：含逗号、引号、换行的用引号括起来，里面的引号写两遍。
fn field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

/// 当前视图里的文件夹和文件（不小于 [`MIN_BYTES`]），一行一个，从上到下按目录树的顺序。
/// 开头带 UTF-8 标记，Excel 才不会把中文路径读成乱码。
fn csv(report: &Report) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("\u{feff}path,type,size_bytes,files,modified,category\n");
    let Some(view) = report.view_node() else {
        return out;
    };

    fn row(out: &mut String, path: &std::path::Path, node: &Node) {
        let kind = if node.kind == Kind::Dir {
            "folder"
        } else {
            "file"
        };
        let modified = if node.modified == 0 {
            String::new()
        } else {
            use chrono::TimeZone;
            chrono::Local
                .timestamp_opt(i64::from(node.modified), 0)
                .single()
                .map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_default()
        };
        let _ = writeln!(
            out,
            "{},{kind},{},{},{modified},{}",
            field(&path.display().to_string()),
            node.size,
            node.files,
            category_key(node.category),
        );
    }

    fn walk(out: &mut String, rows: &mut usize, path: &mut PathBuf, node: &Node) {
        for child in &node.children {
            if *rows >= MAX_ROWS {
                return;
            }
            // 「小项目」没有名字，大小已经算在上一层里了。
            if child.kind == Kind::Rest || child.size < MIN_BYTES {
                continue;
            }
            path.push(&*child.name);
            row(out, path, child);
            *rows += 1;
            if child.kind == Kind::Dir {
                walk(out, rows, path, child);
            }
            path.pop();
        }
    }

    let mut path = user_path(report.tree.path_of(&report.view));
    row(&mut out, &path, view);
    let mut rows = 1;
    walk(&mut out, &mut rows, &mut path, view);
    out
}

#[cfg(test)]
mod tests {
    use super::super::{Target, hidden};
    use super::*;
    use pc_specs_core::space::{Options, Progress, scan_with};
    use std::sync::atomic::AtomicBool;

    #[test]
    fn the_csv_lists_big_items_in_tree_order_and_quotes_awkward_paths() {
        let dir = std::env::temp_dir().join(format!("pcs-csv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, bytes) in [
            ("movies/a.mp4", 2 << 20),
            ("docs, old/c.txt", 3 << 19),
            ("tiny.bin", 100 << 10),
        ] {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, vec![1u8; bytes]).unwrap();
        }
        let options = Options {
            base_keep: 0,
            ..Options::default()
        };
        let tree = scan_with(
            &dir,
            &options,
            &Progress::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        let target = Target {
            label: "a/b".into(),
            path: dir.clone(),
            volume: false,
        };
        let report = Report::new(1, target, tree, hidden::Extra::default(), None);

        let text = csv(&report);
        let lines: Vec<&str> = text.trim_start_matches('\u{feff}').lines().collect();
        assert_eq!(lines[0], "path,type,size_bytes,files,modified,category");
        // 根、两个文件夹、两个文件；tiny.bin 不到 1 MiB，不导出。
        assert_eq!(lines.len(), 1 + 5);
        assert!(lines[1].starts_with(&dir.display().to_string()) && lines[1].contains(",folder,"));
        assert!(lines[3].contains("a.mp4") && lines[3].contains(",file,"));
        let quoted = format!("\"{}\",folder,", dir.join("docs, old").display());
        assert!(text.contains(&quoted), "{text}");
        assert!(!text.contains("tiny.bin"));
        // 位置名里的斜杠不能进文件名。
        assert!(file_name(&report).starts_with("a_b-space-"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fields_with_commas_quotes_or_line_breaks_are_quoted() {
        assert_eq!(field("/a/b.txt"), "/a/b.txt");
        assert_eq!(field("/a/b, c.txt"), "\"/a/b, c.txt\"");
        assert_eq!(field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(field("two\nlines"), "\"two\nlines\"");
    }

    #[test]
    fn every_category_has_a_stable_english_key() {
        let keys: Vec<_> = Category::ALL.iter().map(|&c| category_key(c)).collect();
        assert_eq!(
            keys,
            [
                "video", "audio", "image", "document", "archive", "program", "code", "other"
            ]
        );
    }
}
