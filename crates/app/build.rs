//! Windows：把图标嵌进 exe。
//!
//! GPUI 会从 exe 的资源 ID 1 读取窗口 / 任务栏图标，资源管理器里的文件图标也来自这里。
//! 需要 `rc.exe`，所以只在 Windows 主机上生效；在 macOS / Linux 上交叉编译 Windows 目标时会跳过。

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icons/icon.ico");

    let target_is_windows = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    if cfg!(windows) && target_is_windows {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/icons/icon.ico");
        res.set("ProductName", "PC Specs");
        res.set("FileDescription", "PC Specs");
        res.compile()
            .expect("failed to embed the Windows icon resource");
    }
}
