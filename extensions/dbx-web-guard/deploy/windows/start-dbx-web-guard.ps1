param(
    [Parameter(Mandatory = $true)]
    [string]$GuardRoot
)

$resolvedRoot = (Resolve-Path -LiteralPath $GuardRoot).Path
$executable = Join-Path $resolvedRoot "dbx-web-guard.exe"
$config = Join-Path $resolvedRoot "config\guard.toml"
$logDirectory = Join-Path $resolvedRoot "logs"

if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
    throw "dbx-web-guard.exe was not found under the configured Guard root"
}
if (-not (Test-Path -LiteralPath $config -PathType Leaf)) {
    throw "Guard configuration was not found"
}

New-Item -ItemType Directory -Force -Path $logDirectory | Out-Null
$env:RUST_LOG = "dbx_web_guard=info"
Set-Location -LiteralPath $resolvedRoot
& $executable --config $config serve `
    1>> (Join-Path $logDirectory "stdout.log") `
    2>> (Join-Path $logDirectory "stderr.log")
exit $LASTEXITCODE
