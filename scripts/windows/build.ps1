<#
.SYNOPSIS
  编译 Windows 发布版二进制（原生编译）。

.DESCRIPTION
  scripts\windows\build.ps1 [-Arch x64|arm64]     默认本机架构

  需要 Rust 和 Visual Studio Build Tools（"使用 C++ 的桌面开发"工作负载）。
  产物：target\<x86_64|aarch64>-pc-windows-msvc\release\pc-specs.exe，之后用 scripts\windows\package.ps1 打包。

  在 x64 机器上编 arm64（或反过来）也可以：Build Tools 里要装对应架构的 MSVC 生成工具，arm64 还需要 clang（LLVM）。
  GPUI 在 Windows 上要用 Windows SDK 里的 fxc.exe 编着色器，所以只能在 Windows 上编译，没法从 macOS / Linux 交叉编译。
#>
param(
  [ValidateSet('x64', 'arm64')]
  [string]$Arch
)

. "$PSScriptRoot\common.ps1"
trap { Exit-WithError $_ }

if ($env:OS -ne 'Windows_NT') {
  throw 'Windows 版只能在 Windows 上编译（GPUI 要用 Windows SDK 的 fxc.exe 编着色器，没法交叉编译）；没有 Windows 机器的话，用 GitHub Actions 的 Release 工作流'
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
  throw '缺少 cargo：https://rustup.rs'
}

if (-not $Arch) { $Arch = Get-HostArch }
$triple = Get-WindowsTriple $Arch

# ring（HTTPS 依赖）在 Windows ARM64 上必须用 clang 编译 C 代码。
if ($Arch -eq 'arm64' -and -not (Get-Command clang -ErrorAction SilentlyContinue)) {
  throw 'ARM64 版需要 clang（LLVM）：winget install LLVM.LLVM，或 choco install llvm'
}

if (Get-Command rustup -ErrorAction SilentlyContinue) {
  Invoke-Native rustup @('target', 'add', $triple)
}

Write-Log "cargo build --release --target $triple"
Push-Location $Root
try {
  Invoke-Native cargo @('build', '--release', '--locked', '-p', $Package, '--target', $triple)
}
finally {
  Pop-Location
}

Write-Log "完成：$(Get-WindowsBinary $Arch)"
