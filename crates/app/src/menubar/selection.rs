//! macOS 菜单栏项被选中时的样子：一块没有圆角的矩形底色。
//!
//! 状态项被按下、弹出菜单或者 `highlight:` 的时候，系统会在它后面画一个圆角药丸。
//! 药丸不是应用画的，也没有任何公开接口能改它的形状：AppKit 把「选中区域」交给菜单栏渲染器，
//! 由系统画出来，入口只有 `-[NSStatusItem _setSelectedMenuContentFrame:]` 这一个。
//! 所以这里换掉它：永远告诉系统「没有选中区域」（药丸就不会出现），
//! 同样的状态改成按钮图层上的一块矩形底色。其它显示器上的菜单栏是按钮内容的快照，底色会一起带过去。
//!
//! 这是私有方法。方法不存在、签名对不上都会原样放弃，退回系统自己的药丸，不会有别的影响。
use objc2::{
    ClassType, MainThreadMarker,
    runtime::{Imp, Sel},
    sel,
};
use objc2_app_kit::{NSAppearanceCustomization, NSStatusItem, NSView};
use objc2_core_graphics::CGColor;
use objc2_foundation::NSRect;
use std::sync::{Once, OnceLock};

type SetSelectedFrame = unsafe extern "C-unwind" fn(&NSStatusItem, Sel, NSRect);

/// 被换掉的原方法，用来把「没有选中区域」转交给系统。
static ORIGINAL: OnceLock<SetSelectedFrame> = OnceLock::new();

/// 深色菜单栏上的底色：白色的一层薄雾。
const ON_DARK: (f64, f64) = (1., 0.22);
/// 浅色菜单栏上的底色：黑色的一层薄雾。
const ON_LIGHT: (f64, f64) = (0., 0.14);

/// 换掉系统的选中效果。整个进程只做一次，要在创建第一个状态项之前调用。
pub fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Some(method) =
            NSStatusItem::class().instance_method(sel!(_setSelectedMenuContentFrame:))
        else {
            tracing::debug!("the status item selection hook is not available on this macOS");
            return;
        };
        // 签名必须是 `void (id, SEL, CGRect)`，别的一概不碰。
        let rect = method.argument_type(2);
        let matches = method.arguments_count() == 3
            && method.return_type().to_bytes() == b"v"
            && rect
                .as_deref()
                .is_some_and(|ty| ty.to_bytes().starts_with(b"{CGRect="));
        if !matches {
            tracing::debug!("the status item selection hook has an unexpected signature");
            return;
        }
        // SAFETY: 新函数的签名和原方法一致（self、_cmd、一个 CGRect，无返回值），上面已经核对过。
        let hook = unsafe { std::mem::transmute::<SetSelectedFrame, Imp>(selected_frame) };
        let original = unsafe { method.set_implementation(hook) };
        // SAFETY: 原实现就是这个签名。
        let original = unsafe { std::mem::transmute::<Imp, SetSelectedFrame>(original) };
        let _ = ORIGINAL.set(original);
    });
}

/// 替换后的 `-[NSStatusItem _setSelectedMenuContentFrame:]`。空矩形表示没有选中。
unsafe extern "C-unwind" fn selected_frame(this: &NSStatusItem, cmd: Sel, frame: NSRect) {
    if let Some(original) = ORIGINAL.get() {
        // SAFETY: 参数和原方法一致；永远交给系统一个空矩形，它就不会画药丸。
        unsafe { original(this, cmd, NSRect::ZERO) };
    }
    if let Some(mtm) = MainThreadMarker::new() {
        paint(this, mtm, frame.size.width > 0. && frame.size.height > 0.);
    }
}

fn paint(item: &NSStatusItem, mtm: MainThreadMarker, selected: bool) {
    let Some(button) = item.button(mtm) else {
        return;
    };
    // 系统每画一次按钮都会重新问一遍选中区域。状态没变就什么都别碰：
    // 改图层会触发重画，重画又回到这里，会一直转下去白白烧 CPU。
    let painted = button
        .layer()
        .is_some_and(|layer| layer.backgroundColor().is_some());
    if painted == selected {
        return;
    }
    button.setWantsLayer(true);
    let Some(layer) = button.layer() else {
        return;
    };
    if selected {
        // 按钮的外观跟着菜单栏的明暗走（壁纸深的时候菜单栏文字是白的）。
        let dark = button
            .effectiveAppearance()
            .name()
            .to_string()
            .contains("Dark");
        let (gray, alpha) = if dark { ON_DARK } else { ON_LIGHT };
        layer.setBackgroundColor(Some(&CGColor::new_generic_gray(gray, alpha)));
    } else {
        layer.setBackgroundColor(None);
    }
    // 其它显示器上的菜单栏项是快照，要通知它们重画。
    let view: &NSView = &button;
    view.setNeedsDisplay(true);
}
