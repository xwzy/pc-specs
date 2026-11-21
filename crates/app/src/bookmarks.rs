//! 沙盒里的文件夹授权（Mac App Store 版）。
//!
//! App Sandbox 只放行用户在「选择文件夹」对话框里亲自选的位置，而且只到应用退出为止。
//! 想让「最近分析过的文件夹」重启之后还能点开，就得在选完的当下把授权存成「安全作用域书签」，
//! 下次启动先用书签恢复访问权限，再去读那个文件夹。
//!
//! 书签在设置里存成十六进制文本（`Settings::storage_bookmarks`）。其它平台没有沙盒，
//! 这里的函数都返回 `None`，调用方照常直接读路径。

use std::path::{Path, PathBuf};

/// 给已经能访问的文件夹存一份书签；存不了（没有权限、不在沙盒里）返回 `None`。
pub fn save(path: &Path) -> Option<String> {
    imp::save(path).map(|bytes| to_hex(&bytes))
}

/// 用书签恢复访问权限，返回文件夹现在的位置（用户可能挪过它）。
/// 书签过期了的话，第二项是换新的书签。恢复出来的访问权限一直保持到应用退出。
pub fn open(text: &str) -> Option<(PathBuf, Option<String>)> {
    let (path, stale) = imp::open(&from_hex(text)?)?;
    let refreshed = if stale { save(&path) } else { None };
    Some((path, refreshed))
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if text.is_empty() || !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2::runtime::Bool;
    use objc2_foundation::{
        NSData, NSString, NSURL, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions,
    };
    use std::path::{Path, PathBuf};

    pub fn save(path: &Path) -> Option<Vec<u8>> {
        let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(path.to_str()?), true);
        let data = url
            .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
                NSURLBookmarkCreationOptions::WithSecurityScope,
                None,
                None,
            )
            .ok()?;
        Some(data.to_vec())
    }

    pub fn open(bytes: &[u8]) -> Option<(PathBuf, bool)> {
        let data = NSData::with_bytes(bytes);
        let mut stale = Bool::NO;
        // SAFETY: `stale` 是一个活着的局部变量，指针在调用期间有效。
        let url = unsafe {
            NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
                &data,
                NSURLBookmarkResolutionOptions::WithSecurityScope,
                None,
                &mut stale,
            )
        }
        .ok()?;
        // SAFETY: `url` 是刚用安全作用域书签解析出来的 URL；访问权限故意不归还，直到进程结束。
        if !unsafe { url.startAccessingSecurityScopedResource() } {
            return None;
        }
        Some((PathBuf::from(url.path()?.to_string()), stale.as_bool()))
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use std::path::{Path, PathBuf};

    pub fn save(_: &Path) -> Option<Vec<u8>> {
        None
    }

    pub fn open(_: &[u8]) -> Option<(PathBuf, bool)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_rejects_garbage() {
        let bytes = [0u8, 1, 0x7f, 0x80, 0xff];
        assert_eq!(to_hex(&bytes), "00017f80ff");
        assert_eq!(from_hex("00017f80ff"), Some(bytes.to_vec()));
        assert_eq!(from_hex("00017F80FF"), Some(bytes.to_vec()));
        for bad in ["", "0", "zz", "0g", "é1"] {
            assert_eq!(from_hex(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_broken_bookmark_is_refused() {
        assert_eq!(open("deadbeef"), None);
        assert_eq!(open("not hex"), None);
    }
}
