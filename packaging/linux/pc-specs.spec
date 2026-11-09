# 预编译二进制的 rpm：不在 rpmbuild 里编译，只把 scripts/linux/package.sh 准备好的文件装进包里。
# 用到的宏都由 package.sh 通过 --define 传入：
#   pcs_version   纯数字版本，如 0.2.0（预发布后缀已拆到 pcs_release 里）
#   pcs_release   Release 字段，正式版是 1，预发布是 0.rc.1.1，保证预发布排在正式版前面
#   pcs_root      仓库根目录
#   pcs_binary    要打包的二进制
#   pcs_metainfo  已经替换好版本号和日期的 AppStream 元数据
#   pcs_license   LICENSE 文件
%global debug_package %{nil}
# 二进制已经在编译时 strip 过，不让 rpmbuild 再去动它（也避免在没有 debuginfo 工具的机器上报错）。
%global __os_install_post %{nil}
%global app_id com.xwzy.pc-specs

Name:           pc-specs
Version:        %{pcs_version}
Release:        %{pcs_release}
Summary:        PC configuration viewer and live monitor
License:        MIT
URL:            https://github.com/xwzy/pc-specs

# GPUI 在运行时 dlopen 下面这些库，rpmbuild 扫 ELF 的 NEEDED 看不到，所以手写。
# 用 soname 而不是包名：Fedora / RHEL / openSUSE 的包名各不相同，soname 是通用的。
Requires:       libvulkan.so.1()(64bit)
Requires:       libxkbcommon-x11.so.0()(64bit)
Requires:       libwayland-client.so.0()(64bit)
Requires:       libX11-xcb.so.1()(64bit)
Requires:       libfontconfig.so.1()(64bit)
Recommends:     mesa-vulkan-drivers

%description
Shows CPU, GPU, memory, storage, network and sensor details of this
computer, with live charts, a tray icon and a local JSON API.

%prep
%build

%install
install -Dm755 "%{pcs_binary}" "%{buildroot}%{_bindir}/pc-specs"
install -Dm644 "%{pcs_root}/packaging/linux/%{app_id}.desktop" "%{buildroot}%{_datadir}/applications/%{app_id}.desktop"
install -Dm644 "%{pcs_root}/assets/icons/32x32.png" "%{buildroot}%{_datadir}/icons/hicolor/32x32/apps/%{app_id}.png"
install -Dm644 "%{pcs_root}/assets/icons/128x128.png" "%{buildroot}%{_datadir}/icons/hicolor/128x128/apps/%{app_id}.png"
install -Dm644 "%{pcs_root}/assets/icons/128x128@2x.png" "%{buildroot}%{_datadir}/icons/hicolor/256x256/apps/%{app_id}.png"
install -Dm644 "%{pcs_metainfo}" "%{buildroot}%{_datadir}/metainfo/%{app_id}.metainfo.xml"
install -Dm644 "%{pcs_license}" "%{buildroot}%{_licensedir}/pc-specs/LICENSE"

%files
%{_bindir}/pc-specs
%{_datadir}/applications/%{app_id}.desktop
%{_datadir}/icons/hicolor/*/apps/%{app_id}.png
%{_datadir}/metainfo/%{app_id}.metainfo.xml
%license %{_licensedir}/pc-specs/LICENSE
