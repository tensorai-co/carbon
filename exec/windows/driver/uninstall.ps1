# Stops and removes winkvm.sys. Run as Administrator.
$ErrorActionPreference = 'Stop'
if (Get-Service winkvm -ErrorAction SilentlyContinue) {
    sc.exe stop winkvm | Out-Null
    sc.exe delete winkvm | Out-Null
}
Remove-Item (Join-Path $env:SystemRoot 'System32\drivers\winkvm.sys') -ErrorAction SilentlyContinue
Write-Host 'winkvm is gone'
