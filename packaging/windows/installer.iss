; Inno Setup 6 安装程序脚本，由 scripts/windows/package.ps1 调用。下面这些值都从命令行传进来：
;   /DVersion=0.3.0-rc.1   完整版本号（根 Cargo.toml 里的）
;   /DVersionCore=0.3.0    去掉预发布后缀的纯数字版本，给文件属性用
;   /DArch=x64|arm64       目标架构
;   /DSourceExe=...        要打进安装包的（已签名的）pc-specs.exe
;   /DRoot=...             仓库根目录
;   /DOutputDir=...        安装程序输出目录
;   /DSign                 要给安装程序和卸载程序签名；同时要用 /Spcsign=... 提供签名命令

#ifndef Version
  #error 缺少 /DVersion：请用 scripts\windows\package.ps1 打包
#endif
#ifndef Arch
  #error 缺少 /DArch：请用 scripts\windows\package.ps1 打包
#endif

[Setup]
; AppId 一旦发布就不能再改：升级和卸载都靠它认出「这是同一个程序」。
AppId={{1DC4EEDE-CEDA-4F6E-AA7F-E0E846F3BE1A}
AppName=PC Specs
AppVersion={#Version}
AppVerName=PC Specs {#Version}
AppPublisher=pc-specs contributors
AppPublisherURL=https://github.com/xwzy/pc-specs
AppSupportURL=https://github.com/xwzy/pc-specs/issues
AppUpdatesURL=https://github.com/xwzy/pc-specs/releases
AppCopyright=Copyright (C) 2026 pc-specs contributors
VersionInfoVersion={#VersionCore}.0
VersionInfoProductName=PC Specs
VersionInfoDescription=PC Specs Setup

; 默认装给当前用户（不需要管理员权限），安装时可以改成所有用户。
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
DefaultDirName={autopf}\PC Specs
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\pc-specs.exe
SetupIconFile={#Root}\assets\icons\icon.ico
LicenseFile={#Root}\LICENSE
WizardStyle=modern

; 升级时让安装程序自己关掉正在运行的 PC Specs，装完不再自动重启它。
CloseApplications=yes
RestartApplications=no

Compression=lzma2/max
SolidCompression=yes
OutputDir={#OutputDir}
OutputBaseFilename=pc-specs-{#Version}-windows-{#Arch}-setup

#if Arch == "arm64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
#else
; x64compatible 同时包含 x64 和能运行 x64 程序的 Windows on ARM。
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
#endif

#ifdef Sign
SignTool=pcsign
SignedUninstaller=yes
#endif

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "pc-specs.exe"; Flags: ignoreversion
Source: "{#Root}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\PC Specs"; Filename: "{app}\pc-specs.exe"
Name: "{autodesktop}\PC Specs"; Filename: "{app}\pc-specs.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\pc-specs.exe"; Description: "{cm:LaunchProgram,PC Specs}"; Flags: nowait postinstall skipifsilent

[Code]
// 应用里打开「开机自启」会在当前用户的 Run 键里写一个值，卸载时一并清掉，免得留下指向已删除文件的启动项。
// 名字要和 crates/app/src/autostart.rs 里的一致。
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'PC Specs');
end;
