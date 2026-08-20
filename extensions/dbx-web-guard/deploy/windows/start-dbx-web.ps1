param(
    [Parameter(Mandatory = $true)]
    [string]$ServerRoot,
    [int]$Port = 4225,
    [string]$PublicBasePath = "/"
)

$resolvedRoot = (Resolve-Path -LiteralPath $ServerRoot).Path
$executable = Join-Path $resolvedRoot "dbx-web.exe"
$dataDirectory = Join-Path $resolvedRoot "data"
$logDirectory = Join-Path $resolvedRoot "logs"

if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
    throw "dbx-web.exe was not found under the configured server root"
}

New-Item -ItemType Directory -Force -Path $dataDirectory, $logDirectory | Out-Null
$env:DBX_DATA_DIR = $dataDirectory
$env:DBX_PORT = [string]$Port
$env:DBX_PUBLIC_BASE_PATH = $PublicBasePath
$env:RUST_LOG = "dbx_web=info,tower_http=info"

Set-Location -LiteralPath $resolvedRoot
& $executable `
    1>> (Join-Path $logDirectory "stdout.log") `
    2>> (Join-Path $logDirectory "stderr.log")
exit $LASTEXITCODE
