//! 空间分析里的搜索与筛选：在正在看的目录下，按名字、大小、多久没动过找文件和文件夹。

use pc_specs_core::space::{Category, Kind, Node};

/// 「至少这么大」的几档（字节）。
pub(super) const SIZES: [u64; 5] = [0, 1 << 20, 10 << 20, 100 << 20, 1 << 30];
/// 「至少这么多天没修改过」的几档。
pub(super) const AGES: [u32; 4] = [0, 30, 90, 365];

const DAY: u64 = 24 * 60 * 60;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Query {
    /// 名字里要包含的词，空格隔开表示都要包含；不分大小写。
    pub text: String,
    /// 至少多大，0 表示不限。
    pub min_size: u64,
    /// 至少多少天没修改过，0 表示不限。只有文件有修改时间，所以设了它结果里就只有文件。
    pub older_days: u32,
}

impl Query {
    /// 有没有设任何条件；没设就还是平常的「最大文件 / 当前目录」列表。
    pub fn is_active(&self) -> bool {
        !self.text.trim().is_empty() || self.min_size > 0 || self.older_days > 0
    }

    /// 编译成逐个节点判断的样子。`now` 是现在的 Unix 秒，`only` 是图例里选中的类型。
    pub fn matcher(&self, now: u64, only: Option<Category>) -> Matcher {
        Matcher {
            terms: self
                .text
                .split_whitespace()
                .map(str::to_lowercase)
                .collect(),
            min_size: self.min_size,
            cutoff: (self.older_days > 0).then(|| {
                u32::try_from(now.saturating_sub(u64::from(self.older_days) * DAY))
                    .unwrap_or(u32::MAX)
            }),
            only,
        }
    }
}

pub(super) struct Matcher {
    terms: Vec<String>,
    min_size: u64,
    /// 修改时间不晚于它才算「很久没动过」。
    cutoff: Option<u32>,
    only: Option<Category>,
}

impl Matcher {
    /// 符合条件就给出排序用的大小，不符合给 `None`。
    /// 限定了类型时，目录算的是它里面这个类型的字节数，和列表、树图的口径一致。
    pub fn score(&self, node: &Node) -> Option<u64> {
        let measure = match (self.only, node.kind) {
            (Some(category), Kind::File) => (node.category == category).then_some(node.size)?,
            (Some(category), _) => node.category_size(category),
            (None, _) => node.size,
        };
        if measure == 0 || measure < self.min_size {
            return None;
        }
        if let Some(cutoff) = self.cutoff
            && (node.kind != Kind::File || node.modified == 0 || node.modified > cutoff)
        {
            return None;
        }
        self.terms
            .iter()
            .all(|term| contains_ignore_case(&node.name, term))
            .then_some(measure)
    }
}

/// `haystack` 里有没有 `needle`（已转成小写），不分大小写。
/// 绝大多数文件名是 ASCII，直接逐字节比，免得几十万个文件每个都分配一份小写副本。
fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.is_ascii() && needle.is_ascii() {
        return haystack
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()));
    }
    haystack.to_lowercase().contains(needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pc_specs_core::space::{Options, Progress, Tree, scan_with};
    use std::fs;
    use std::sync::atomic::AtomicBool;

    fn tree(tag: &str) -> Tree {
        let dir = std::env::temp_dir().join(format!("pcs-search-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let write = |relative: &str, bytes: usize| {
            let path = dir.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, vec![7u8; bytes]).unwrap();
        };
        write("Holiday/Clip.MP4", 3 << 20);
        write("Holiday/notes.txt", 20 << 10);
        write("work/holiday-plan.pdf", 2 << 20);
        write("文档/报告.pdf", 1 << 20);
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
        .expect("scan");
        let _ = fs::remove_dir_all(&dir);
        tree
    }

    fn found_at(tree: &Tree, query: &Query, only: Option<Category>, now: u64) -> Vec<String> {
        let matcher = query.matcher(now, only);
        tree.root
            .search(50, &|node| matcher.score(node))
            .into_iter()
            .map(|(_, node, _)| node.name.to_string())
            .collect()
    }

    /// 用真实的现在判断：刚写下的文件都算新的。
    fn found(tree: &Tree, query: &Query, only: Option<Category>) -> Vec<String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        found_at(tree, query, only, now)
    }

    #[test]
    fn names_match_every_word_ignoring_case() {
        let tree = tree("names");
        let query = |text: &str| Query {
            text: text.into(),
            ..Query::default()
        };
        // 文件夹和文件都能搜到，大的在前；大小写无关。
        assert_eq!(
            found(&tree, &query("HOLIDAY"), None),
            ["Holiday", "holiday-plan.pdf"]
        );
        // 多个词要同时包含。
        assert_eq!(
            found(&tree, &query("holiday pdf"), None),
            ["holiday-plan.pdf"]
        );
        assert_eq!(found(&tree, &query(".mp4"), None), ["Clip.MP4"]);
        // 中文名也行。
        assert_eq!(found(&tree, &query("报告"), None), ["报告.pdf"]);
        assert!(found(&tree, &query("nothing like this"), None).is_empty());
    }

    #[test]
    fn size_age_and_type_narrow_the_results() {
        let tree = tree("filters");
        let size = Query {
            min_size: 2 << 20,
            ..Query::default()
        };
        // 至少 2 MiB：Holiday 文件夹（3 MiB 多）、Clip.MP4、holiday-plan.pdf 和 work 文件夹。
        let big = found(&tree, &size, None);
        assert!(big.contains(&"Clip.MP4".to_string()) && !big.contains(&"notes.txt".to_string()));
        assert!(!big.contains(&"报告.pdf".to_string()));

        // 刚写下的文件不算「很久没动过」；只有文件有修改时间，所以结果里没有文件夹。
        let old = Query {
            older_days: 30,
            ..Query::default()
        };
        assert!(found(&tree, &old, None).is_empty());
        // 把「现在」拨到十年以后，它们就都很久没动过了。
        let files = found_at(&tree, &old, None, u64::from(u32::MAX));
        assert!(files.contains(&"Clip.MP4".to_string()));
        assert!(!files.contains(&"Holiday".to_string()));

        // 只看视频：文件夹按里面的视频字节数算。
        let video = found(&tree, &Query::default(), Some(Category::Video));
        assert_eq!(video, ["Holiday", "Clip.MP4"]);
    }

    #[test]
    fn an_empty_query_is_not_active() {
        assert!(!Query::default().is_active());
        assert!(
            !Query {
                text: "   ".into(),
                ..Query::default()
            }
            .is_active()
        );
        assert!(
            Query {
                min_size: 1,
                ..Query::default()
            }
            .is_active()
        );
        assert!(
            Query {
                older_days: 30,
                ..Query::default()
            }
            .is_active()
        );
    }

    #[test]
    fn substring_search_handles_ascii_and_unicode() {
        assert!(contains_ignore_case("Quarterly Report.XLSX", "report"));
        assert!(contains_ignore_case("a", ""));
        assert!(!contains_ignore_case("ab", "abc"));
        assert!(contains_ignore_case("ÄPFEL.txt", "äpfel"));
        assert!(!contains_ignore_case("apple", "äpfel"));
    }
}
