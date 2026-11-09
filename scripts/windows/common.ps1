# 各 Windows 脚本共用的常量和函数，用 `. "$PSScriptRoot\common.ps1"` 引入，不直接运行。
# 常量和 scripts/lib/common.sh 里的保持一致。写法兼容 Windows 自带的 PowerShell 5.1。

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3.0

$Root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$AppName = 'PC Specs'
$AppId = 'com.xwzy.pc-specs'
$BinName = 'pc-specs'
$Package = 'pc-specs'

function Write-Log([string]$Message) {
  Write-Host "==> $Message" -ForegroundColor Blue
}

function Write-Warn([string]$Message) {
  Write-Host "warning: $Message" -ForegroundColor Yellow
}

# 脚本开头写 `trap { Exit-WithError $_ }`：出错时只打一行说明并以 1 退出，不甩一大段 PowerShell 的调用栈。
function Exit-WithError($ErrorRecord) {
  [Console]::Error.WriteLine("error: $($ErrorRecord.Exception.Message)")
  exit 1
}

# 运行外部命令，退出码不是 0 就报错（PowerShell 默认不会因为外部命令失败而停下）。
function Invoke-Native([string]$File, [string[]]$Arguments) {
  $global:LASTEXITCODE = 0
  & $File @Arguments
  if ($LASTEXITCODE -ne 0) {
    throw "$File 失败（退出码 $LASTEXITCODE）"
  }
}

# 版本号的唯一来源是根 Cargo.toml 的 [workspace.package].version，直接读这个文件
# （逻辑和 scripts/lib/common.sh 的 app_version 一致）。
function Get-AppVersion {
  $inSection = $false
  foreach ($line in Get-Content (Join-Path $Root 'Cargo.toml')) {
    if ($line -match '^\[') {
      $inSection = ($line.Trim() -eq '[workspace.package]')
      continue
    }
    if ($inSection -and $line -match '^version\s*=\s*"([^"]+)"') {
      return $Matches[1]
    }
  }
  throw '根 Cargo.toml 的 [workspace.package] 里没有找到 version'
}

# 去掉预发布 / 构建元数据后缀：1.2.3-rc.1+abc → 1.2.3。Windows 的文件版本只认纯数字。
function Get-VersionCore([string]$Version) {
  return ($Version -split '[-+]')[0]
}

# cargo 的输出目录（尊重 CARGO_TARGET_DIR）。
function Get-TargetDir {
  if ($env:CARGO_TARGET_DIR) { return $env:CARGO_TARGET_DIR }
  return (Join-Path $Root 'target')
}

# 最终发布文件的输出目录。
function Get-DistDir {
  if ($env:DIST_DIR) { return $env:DIST_DIR }
  return (Join-Path $Root 'dist')
}

# 本机架构，用统一的名字：x64 / arm64。
function Get-HostArch {
  # 32 位进程里 PROCESSOR_ARCHITECTURE 会被 WOW64 改写，真实架构在 PROCESSOR_ARCHITEW6432 里。
  $arch = $env:PROCESSOR_ARCHITEW6432
  if (-not $arch) { $arch = $env:PROCESSOR_ARCHITECTURE }
  switch ($arch) {
    'AMD64' { return 'x64' }
    'ARM64' { return 'arm64' }
    default { throw "不支持的 CPU 架构：$arch" }
  }
}

function Get-WindowsTriple([string]$Arch) {
  switch ($Arch) {
    'x64' { return 'x86_64-pc-windows-msvc' }
    'arm64' { return 'aarch64-pc-windows-msvc' }
    default { throw "不支持的架构：$Arch（x64 | arm64）" }
  }
}

# 某个架构编译出来的 exe 放在哪。
function Get-WindowsBinary([string]$Arch) {
  return (Join-Path (Get-TargetDir) "$(Get-WindowsTriple $Arch)/release/$BinName.exe")
}

# 有没有配置代码签名（详见 scripts/windows/sign.ps1）。
function Test-SigningConfigured {
  return [bool]($env:WINDOWS_SIGN_COMMAND -or $env:WINDOWS_CERTIFICATE_THUMBPRINT -or $env:WINDOWS_CERTIFICATE)
}
