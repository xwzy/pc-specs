<#
.SYNOPSIS
  给 Windows 的 exe / 安装包做 Authenticode 签名。

.DESCRIPTION
  scripts\windows\sign.ps1 -Path <文件>[,<文件>...] [-Strict]

  用环境变量选择签名方式，按顺序取第一个配置了的：

    WINDOWS_SIGN_COMMAND            自定义命令，由 PowerShell 执行，{file} 会换成文件路径（不要再给它加引号）。
                                    Azure Trusted Signing、SignPath、DigiCert KeyLocker（smctl）、云 HSM 这类
                                    导不出私钥的方案都靠它接入，例如 Azure Trusted Signing：
                                      signtool sign /fd SHA256 /tr http://timestamp.acs.microsoft.com /td SHA256 `
                                        /dlib C:\azure\Azure.CodeSigning.Dlib.dll /dmdf C:\azure\metadata.json {file}
    WINDOWS_CERTIFICATE_THUMBPRINT  证书在本机证书库里（硬件令牌、本机开发时用），按 SHA-1 指纹查找
    WINDOWS_CERTIFICATE             .pfx 证书的 base64（CI 用），密码放 WINDOWS_CERTIFICATE_PASSWORD

  可选：
    WINDOWS_TIMESTAMP_URL           RFC 3161 时间戳服务，默认 http://timestamp.digicert.com
    WINDOWS_SIGNTOOL                signtool.exe 的路径，默认到 PATH 和 Windows SDK 里找

  没配置任何签名方式时只提示一句，文件保持未签名；加 -Strict（发布流程用）则改成报错。
  签完用 Get-AuthenticodeSignature 校验：-Strict 下没通过算错误，否则只提示（自签名的测试证书过不了校验）。
#>
param(
  [Parameter(Mandatory = $true, Position = 0)]
  [string[]]$Path,
  [switch]$Strict
)

. "$PSScriptRoot\common.ps1"
trap { Exit-WithError $_ }

$files = @($Path | ForEach-Object { (Resolve-Path $_).Path })

if (-not (Test-SigningConfigured)) {
  if ($Strict) {
    throw '没有配置代码签名：请设置 WINDOWS_SIGN_COMMAND、WINDOWS_CERTIFICATE_THUMBPRINT 或 WINDOWS_CERTIFICATE（见 Get-Help）'
  }
  Write-Warn '没有配置代码签名，文件保持未签名（用户安装时 SmartScreen 会提示“未知发布者”）'
  return
}

$timestamp = 'http://timestamp.digicert.com'
if ($env:WINDOWS_TIMESTAMP_URL) { $timestamp = $env:WINDOWS_TIMESTAMP_URL }

function Find-SignTool {
  if ($env:WINDOWS_SIGNTOOL) { return $env:WINDOWS_SIGNTOOL }
  $command = Get-Command signtool.exe -ErrorAction SilentlyContinue
  if ($command) { return $command.Source }

  # 没在 PATH 里就到 Windows SDK 里找，取版本号最大的那份（目录名里的版本号位数一致，按名字排序就行）。
  $hostDir = 'x64'
  if ((Get-HostArch) -eq 'arm64') { $hostDir = 'arm64' }
  $programFiles = ${env:ProgramFiles(x86)}
  if (-not $programFiles) { $programFiles = $env:ProgramFiles }
  $kits = Join-Path $programFiles 'Windows Kits\10\bin'
  $found = Get-ChildItem -Path $kits -Filter signtool.exe -Recurse -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -like "*\$hostDir\signtool.exe" } |
    Sort-Object FullName -Descending |
    Select-Object -First 1
  if (-not $found) {
    throw '找不到 signtool.exe：安装 Windows SDK（winget install Microsoft.WindowsSDK.10.0.26100），或用 WINDOWS_SIGNTOOL 指定路径'
  }
  return $found.FullName
}

# 时间戳服务偶尔会超时，失败了等几秒再试，最多三次。
function Invoke-WithRetry([scriptblock]$Action, [string]$What) {
  for ($attempt = 1; $attempt -le 3; $attempt++) {
    try {
      & $Action
      return
    }
    catch {
      if ($attempt -eq 3) { throw }
      Write-Warn "$What 失败（第 $attempt 次），5 秒后重试：$($_.Exception.Message)"
      Start-Sleep -Seconds 5
    }
  }
}

function Invoke-CustomCommand([string]$File) {
  $quoted = "'" + $File.Replace("'", "''") + "'"
  $command = $env:WINDOWS_SIGN_COMMAND.Replace('"{file}"', $quoted).Replace('{file}', $quoted)
  $global:LASTEXITCODE = 0
  Invoke-Expression $command
  if ($LASTEXITCODE -ne 0) {
    throw "WINDOWS_SIGN_COMMAND 失败（退出码 $LASTEXITCODE）"
  }
}

$signtool = $null
$certArguments = @()
$pfxPath = $null
try {
  if (-not $env:WINDOWS_SIGN_COMMAND) {
    $signtool = Find-SignTool
    if ($env:WINDOWS_CERTIFICATE_THUMBPRINT) {
      $certArguments = @('/sha1', $env:WINDOWS_CERTIFICATE_THUMBPRINT)
    }
    else {
      # .pfx 只在签名期间落盘，签完就删。
      $pfxPath = Join-Path ([IO.Path]::GetTempPath()) ("pcs-sign-" + [guid]::NewGuid().ToString('N') + '.pfx')
      [IO.File]::WriteAllBytes($pfxPath, [Convert]::FromBase64String($env:WINDOWS_CERTIFICATE))
      $certArguments = @('/f', $pfxPath)
      # PowerShell 5.1 不会把空字符串传给外部命令，所以没有密码时干脆不带 /p。
      if ($env:WINDOWS_CERTIFICATE_PASSWORD) {
        $certArguments += @('/p', $env:WINDOWS_CERTIFICATE_PASSWORD)
      }
    }
  }

  foreach ($file in $files) {
    $name = [IO.Path]::GetFileName($file)
    Write-Log "签名 $name"
    if ($env:WINDOWS_SIGN_COMMAND) {
      Invoke-WithRetry { Invoke-CustomCommand $file } "签名 $name"
    }
    else {
      $arguments = @('sign') + $certArguments + @('/fd', 'sha256', '/tr', $timestamp, '/td', 'sha256', '/d', $AppName, $file)
      Invoke-WithRetry { Invoke-Native $signtool $arguments } "签名 $name"
    }

    $signature = Get-AuthenticodeSignature -FilePath $file
    if ($signature.Status -eq 'Valid') {
      Write-Log "签名有效：$name（$($signature.SignerCertificate.Subject)）"
    }
    elseif ($Strict) {
      throw "签名校验没有通过：$name（$($signature.Status)：$($signature.StatusMessage)）"
    }
    else {
      Write-Warn "签名没有通过校验：$name（$($signature.Status)）；自签名的测试证书就是这样"
    }
  }
}
finally {
  if ($pfxPath) { Remove-Item -Force $pfxPath -ErrorAction SilentlyContinue }
}
