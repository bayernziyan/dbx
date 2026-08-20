param(
    [Parameter(Mandatory = $true)]
    [string]$GuardBinary,
    [Parameter(Mandatory = $true)]
    [string]$GuardRoot,
    [Parameter(Mandatory = $true)]
    [string]$FrontendSource,
    [Parameter(Mandatory = $true)]
    [string]$FrontendTarget
)

$binary = (Resolve-Path -LiteralPath $GuardBinary).Path
$frontend = (Resolve-Path -LiteralPath $FrontendSource).Path
$guard = [System.IO.Path]::GetFullPath($GuardRoot)
$frontendDestination = [System.IO.Path]::GetFullPath($FrontendTarget)
$targetBinary = Join-Path $guard "dbx-web-guard.exe"

foreach ($path in @(
    $guard,
    (Join-Path $guard "config"),
    (Join-Path $guard "secrets"),
    (Join-Path $guard "state"),
    (Join-Path $guard "logs"),
    $frontendDestination
)) {
    New-Item -ItemType Directory -Force -Path $path | Out-Null
}

$stagedBinary = Join-Path $guard "dbx-web-guard.exe.new"
Copy-Item -LiteralPath $binary -Destination $stagedBinary -Force
if ((Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash -ne (Get-FileHash -LiteralPath $stagedBinary -Algorithm SHA256).Hash) {
    throw "Staged Guard binary hash mismatch"
}
$runningTarget = Get-Process -Name "dbx-web-guard" -ErrorAction SilentlyContinue | Where-Object {
    try { [System.IO.Path]::GetFullPath($_.Path) -eq $targetBinary } catch { $false }
}
if ($runningTarget) {
    throw "The deployed Guard process is still running; stop its Windows service before staging"
}
if (Test-Path -LiteralPath $targetBinary -PathType Leaf) {
    Copy-Item -LiteralPath $targetBinary -Destination "$targetBinary.previous" -Force
}
Move-Item -LiteralPath $stagedBinary -Destination $targetBinary -Force

$startScript = Join-Path $PSScriptRoot "start-dbx-web-guard.ps1"
Copy-Item -LiteralPath $startScript -Destination (Join-Path $guard "start-dbx-web-guard.ps1") -Force
$configTemplate = Join-Path $PSScriptRoot "..\..\config\guard.toml.example"
$deployedConfig = Join-Path $guard "config\guard.toml"
if (-not (Test-Path -LiteralPath $deployedConfig)) {
    Copy-Item -LiteralPath $configTemplate -Destination $deployedConfig
}

Get-ChildItem -LiteralPath $frontend -Force | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination $frontendDestination -Recurse -Force
}

Get-FileHash -LiteralPath $targetBinary -Algorithm SHA256
