param(
    [Parameter(Mandatory = $true)]
    [string]$OutputPath,
    [Parameter(Mandatory = $true)]
    [string]$StaticDirectory,
    [Parameter(Mandatory = $true)]
    [string[]]$AllowedOrigins,
    [int]$BackendPort = 4225,
    [int]$GuardPort = 4226,
    [string]$PublicBasePath = "/",
    [switch]$Force
)

$template = Join-Path $PSScriptRoot "..\..\config\guard.toml.example"
$template = (Resolve-Path -LiteralPath $template).Path
$outputFullPath = [System.IO.Path]::GetFullPath($OutputPath)
$staticFullPath = [System.IO.Path]::GetFullPath($StaticDirectory)

if ((Test-Path -LiteralPath $outputFullPath) -and -not $Force) {
    throw "Guard configuration already exists; use -Force only after backing it up"
}
if ($AllowedOrigins.Count -eq 0) {
    throw "At least one browser origin is required"
}
if ($staticFullPath.Contains("'")) {
    throw "Static directory cannot contain a single quote"
}

$originValues = $AllowedOrigins | ForEach-Object {
    if ($_ -notmatch '^https?://[^/]+$') {
        throw "Invalid allowed origin: $_"
    }
    '"' + $_.Replace('"', '\"') + '"'
}

$content = Get-Content -LiteralPath $template -Raw
$normalizedBasePath = if ($PublicBasePath -eq "/") { "/" } else { "/" + $PublicBasePath.Trim("/") }
$content = [regex]::Replace($content, '(?m)^public_base_path = .+$', 'public_base_path = "' + $normalizedBasePath + '"', 1)
$policyPrefix = $normalizedBasePath.TrimEnd('/')
$content = $content.Replace('/dbx/api/', ($policyPrefix + '/api/'))
$content = [regex]::Replace($content, '(?m)^listen = .+$', 'listen = "127.0.0.1:' + $GuardPort + '"', 1)
$content = [regex]::Replace($content, '(?m)^base_url = .+$', 'base_url = "http://127.0.0.1:' + $BackendPort + '"', 1)
$content = [regex]::Replace($content, '(?m)^directory = .+$', "directory = '$staticFullPath'", 1)
$content = [regex]::Replace($content, '(?m)^allowed_origins = .+$', 'allowed_origins = [' + ($originValues -join ', ') + ']', 1)

$parent = Split-Path -Parent $outputFullPath
New-Item -ItemType Directory -Force -Path $parent | Out-Null
[System.IO.File]::WriteAllText($outputFullPath, $content, [System.Text.UTF8Encoding]::new($false))
Write-Output $outputFullPath
