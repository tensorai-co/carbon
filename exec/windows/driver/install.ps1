# Installs and starts winkvm.sys as a kernel service. Run as Administrator.
# A test-signed driver loads only with test signing on: `bcdedit /set testsigning on`, then reboot.
param([ValidateSet('Debug', 'Release')] [string] $Configuration = 'Release')
$ErrorActionPreference = 'Stop'

$source = Join-Path $PSScriptRoot "build\$Configuration\winkvm.sys"
$target = Join-Path $env:SystemRoot 'System32\drivers\winkvm.sys'
if (-not (Test-Path $source)) { throw "no ${source}: run build.ps1 first" }

if (Get-Service winkvm -ErrorAction SilentlyContinue) {
    sc.exe stop winkvm | Out-Null
    sc.exe delete winkvm | Out-Null
}
Copy-Item $source $target -Force
sc.exe create winkvm type= kernel start= demand binPath= $target | Out-Null
sc.exe start winkvm
if ($LASTEXITCODE -ne 0) { throw "winkvm didn't start (is test signing on?)" }
Write-Host 'winkvm is running: \\.\WinKvm'
