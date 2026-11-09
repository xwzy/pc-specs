<#
.SYNOPSIS
  Windows 发布打包：签名 exe → 安装程序（Inno Setup）+ 便携版 zip，放进 dist\。

.DESCRIPTION
  scripts\windows\package.ps1 [-Arch x64|arm64] [-Binary <路径>] [-Formats installer,zip] [-RequireSigning]

    -Arch            默认本机架构
    -Binary          用指定的 exe，默认 scripts\windows\build.ps1 的产物
    -Formats         默认 installer,zip
    -RequireSigning  没配置代码签名、或签名没通过校验时报错（发布流程用）

  代码签名用环境变量配置，见 scripts\windows\sign.ps1；没配置就打出未签名的包。
  安装程序用 Inno Setup 6 编译（winget install JRSoftware.InnoSetup）；配置了签名时，
  里面的 exe、安装程序和卸载程序都会签名。

  产物（目录可用 DIST_DIR 改）：
    pc-specs-<版本>-windows-<arch>-setup.exe    安装程序，默认装给当前用户，也可以选所有用户
    pc-specs-<版本>-windows-<arch>.zip          便携版，解压即用
#>
param(
  [ValidateSet('x64', 'arm64')]
  [string]$Arch,
  [string]$Binary,
  [ValidateSet('installer', 'zip')]
  [string[]]$Formats = @('installer', 'zip'),
  [switch]$RequireSigning
)

. "$PSScriptRoot\common.ps1"
trap { Exit-WithError $_ }

if ($env:OS -ne 'Windows_NT') {
  throw 'Windows 版只能在 Windows 上打包；在别的系统上请用 CI（.github/workflows/release.yml）'
}
if (-not $Arch) { $Arch = Get-HostArch }
if (-not $Binary) { $Binary = Get-WindowsBinary $Arch }
if (-not (Test-Path $Binary)) {
  throw "找不到 exe：$Binary（先运行 scripts\windows\build.ps1 -Arch $Arch）"
}
if ($RequireSigning -and -not (Test-SigningConfigured)) {
  throw '指定了 -RequireSigning，但没有配置代码签名：请设置 WINDOWS_SIGN_COMMAND、WINDOWS_CERTIFICATE_THUMBPRINT 或 WINDOWS_CERTIFICATE'
}

# exe 的架构和 -Arch 对不上时，打出来的包装得上、跑不起来，这里提前拦住。读 PE 头里的 Machine 字段。
function Get-PeMachine([string]$File) {
  $stream = [IO.File]::OpenRead($File)
  try {
    $reader = New-Object IO.BinaryReader($stream)
    $null = $stream.Seek(0x3C, 'Begin')
    $peOffset = $reader.ReadInt32()
    $null = $stream.Seek($peOffset + 4, 'Begin')
    return [int]$reader.ReadUInt16()
  }
  finally {
    $stream.Dispose()
  }
}
$expectedMachine = @{ 'x64' = 0x8664; 'arm64' = 0xAA64 }[$Arch]
if ((Get-PeMachine $Binary) -ne $expectedMachine) {
  throw "$Binary 的架构和 -Arch $Arch 不一致"
}

function Invoke-Sign([string[]]$Files) {
  $global:LASTEXITCODE = 0
  $signArguments = @{ Path = $Files }
  if ($RequireSigning) { $signArguments.Strict = $true }
  & "$PSScriptRoot\sign.ps1" @signArguments
  if ($LASTEXITCODE -ne 0) { throw '代码签名失败' }
}

function Find-InnoSetup {
  $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue
  if ($command) { return $command.Source }
  # 安装程序默认装给当前用户时在 %LOCALAPPDATA%\Programs 下，装给所有用户时在 Program Files 下。
  $candidates = @(
    @(${env:ProgramFiles(x86)}, 'Inno Setup 6/ISCC.exe'),
    @($env:ProgramFiles, 'Inno Setup 6/ISCC.exe'),
    @($env:LOCALAPPDATA, 'Programs/Inno Setup 6/ISCC.exe')
  )
  foreach ($candidate in $candidates) {
    if (-not $candidate[0]) { continue }
    $path = Join-Path $candidate[0] $candidate[1]
    if (Test-Path $path) { return $path }
  }
  throw '找不到 Inno Setup 6（ISCC.exe）：winget install JRSoftware.InnoSetup，或 choco install innosetup'
}

$version = Get-AppVersion
$core = Get-VersionCore $version
$dist = Get-DistDir
$stage = Join-Path (Get-TargetDir) "package/windows-$Arch"
New-Item -ItemType Directory -Force $dist | Out-Null
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage | Out-Null

Write-Log "打包 $AppName $version（Windows $Arch）：$($Formats -join ', ')"

# 签名的是 stage 里的副本，target\ 里的原始产物不动。
$exe = Join-Path $stage "$BinName.exe"
Copy-Item $Binary $exe
if (Test-SigningConfigured) {
  Invoke-Sign @($exe)
}
else {
  Write-Warn '没有配置代码签名，打出未签名的包（见 scripts\windows\sign.ps1）'
}

if ($Formats -contains 'zip') {
  $zipDir = Join-Path $stage 'zip'
  New-Item -ItemType Directory -Force $zipDir | Out-Null
  Copy-Item $exe $zipDir
  Copy-Item (Join-Path $Root 'LICENSE') $zipDir
  $zip = Join-Path $dist "pc-specs-$version-windows-$Arch.zip"
  if (Test-Path $zip) { Remove-Item -Force $zip }
  Compress-Archive -Path (Join-Path $zipDir '*') -DestinationPath $zip
  Write-Log "已生成 $(Split-Path $zip -Leaf)"
}

if ($Formats -contains 'installer') {
  $iscc = Find-InnoSetup
  $isccArguments = @(
    '/Qp',
    "/DVersion=$version",
    "/DVersionCore=$core",
    "/DArch=$Arch",
    "/DSourceExe=$exe",
    "/DRoot=$Root",
    "/DOutputDir=$dist"
  )
  if (Test-SigningConfigured) {
    # 让 Inno 在生成安装程序和卸载程序时调用 sign.ps1。命令里 $f 是要签名的文件（Inno 展开时已经带引号），$q 是引号。
    $strict = ''
    if ($RequireSigning) { $strict = ' -Strict' }
    $signScript = Join-Path $PSScriptRoot 'sign.ps1'
    $isccArguments += '/DSign'
    $isccArguments += ('/Spcsign=powershell.exe -NoProfile -ExecutionPolicy Bypass -File $q' + $signScript + '$q' + $strict + ' -Path $f')
  }
  # 传给原生程序的路径用 GetFullPath 规整成反斜杠。
  $isccArguments += [IO.Path]::GetFullPath((Join-Path $Root 'packaging/windows/installer.iss'))

  Invoke-Native $iscc $isccArguments

  $installer = Join-Path $dist "pc-specs-$version-windows-$Arch-setup.exe"
  if (-not (Test-Path $installer)) { throw "Inno Setup 没有产出 $installer" }
  Write-Log "已生成 $(Split-Path $installer -Leaf)（签名状态：$((Get-AuthenticodeSignature $installer).Status)）"
}

Write-Log "完成，文件在 $dist"
Get-ChildItem $dist | Format-Table Name, Length -AutoSize
