//! 应用图标：向系统要，不自己去解 `.icns`——新一些的应用把图标放在资源目录里，根本没有 `.icns`。

use std::path::Path;

/// 应用（或任何文件、文件夹）的图标，渲染成 PNG。macOS 以外没有「应用图标」这回事，返回 `None`。
#[cfg(target_os = "macos")]
pub fn png(path: &Path) -> Option<Vec<u8>> {
    use objc2::AllocAnyThread as _;
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
    use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

    let path = NSString::from_str(path.to_str()?);
    // 系统给的是一组大小不同的图；要 64 点的，Retina 下就是 128 像素那张，列表里用刚好够清楚。
    // 自动释放的临时对象在这个池子里收掉，后台线程上没有别的池子。
    autoreleasepool(|_| {
        let icon = NSWorkspace::sharedWorkspace().iconForFile(&path);
        let mut rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(64.0, 64.0));
        // SAFETY: `rect` 是一个有效的、可写的矩形；上下文和提示传空，用默认值。
        let image = unsafe { icon.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
        let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &image);
        let data = unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }?;
        Some(data.to_vec())
    })
}

#[cfg(not(target_os = "macos"))]
pub fn png(_: &Path) -> Option<Vec<u8>> {
    None
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn the_icon_of_a_system_app_is_a_png() {
        let png = png(Path::new("/System/Applications/Calculator.app")).expect("an icon");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "not a PNG");
        assert!(png.len() > 500);
    }

    #[test]
    fn a_missing_path_still_gets_the_generic_icon_or_nothing() {
        // 不能崩；有没有图都行。
        let _ = png(Path::new("/definitely/not/here.app"));
    }
}
