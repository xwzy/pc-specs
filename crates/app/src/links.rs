//! 对外链接：官网、隐私政策、技术支持。
//!
//! 商店审核要求隐私政策在应用里也能点开（App Review Guideline 5.1.1(i)），所以两个版本都放在「设置 → 关于」里。
//! 页面源码在仓库的 `site/` 目录，中文在根目录、英文在 `en/` 下。

/// 官网。
pub const WEBSITE: &str = "https://pc-specs.xwzy.dev";

/// 隐私政策，按界面语言选中文还是英文页面。
pub fn privacy(zh: bool) -> String {
    page(zh, "privacy.html")
}

/// 技术支持页（常见问题和联系方式）。
pub fn support(zh: bool) -> String {
    page(zh, "support.html")
}

fn page(zh: bool, name: &str) -> String {
    if zh {
        format!("{WEBSITE}/{name}")
    } else {
        format!("{WEBSITE}/en/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_follow_the_interface_language() {
        assert_eq!(privacy(true), "https://pc-specs.xwzy.dev/privacy.html");
        assert_eq!(privacy(false), "https://pc-specs.xwzy.dev/en/privacy.html");
        assert_eq!(support(false), "https://pc-specs.xwzy.dev/en/support.html");
    }
}
