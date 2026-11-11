# Builds winkvm.sys, test-signed, and the smoke test. Run from a "Developer PowerShell for VS" with the WDK installed.
#   .\build.ps1                 Release
#   .\build.ps1 -Configuration Debug
param([ValidateSet('Debug', 'Release')] [string] $Configuration = 'Release')
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

msbuild winkvm.vcxproj /nologo /m /p:Configuration=$Configuration /p:Platform=x64 /p:SolutionDir="$PSScriptRoot\"
if ($LASTEXITCODE -ne 0) { throw "the driver didn't build" }

New-Item -ItemType Directory -Force "build\$Configuration" | Out-Null
cl /nologo /W4 /WX /I . tests\smoke.c /Fe"build\$Configuration\smoke.exe" /Fo"build\obj\$Configuration\\"
if ($LASTEXITCODE -ne 0) { throw "the smoke test didn't build" }

Write-Host "built build\$Configuration\winkvm.sys and build\$Configuration\smoke.exe"
