param(
    [int]$PublicPort = 82,
    [int]$BackendPort = 4225,
    [int]$GuardPort = 4226
)

$rules = @(
    "DBX Web Nginx LocalSubnet",
    "DBX Web Backend Block",
    "DBX Web Guard Block"
)
foreach ($name in $rules) {
    Get-NetFirewallRule -DisplayName $name -ErrorAction SilentlyContinue | Remove-NetFirewallRule
}

New-NetFirewallRule `
    -DisplayName "DBX Web Nginx LocalSubnet" `
    -Direction Inbound `
    -Action Allow `
    -Protocol TCP `
    -LocalPort $PublicPort `
    -RemoteAddress LocalSubnet `
    -Profile Domain,Private | Out-Null

New-NetFirewallRule `
    -DisplayName "DBX Web Backend Block" `
    -Direction Inbound `
    -Action Block `
    -Protocol TCP `
    -LocalPort $BackendPort `
    -Profile Any | Out-Null

New-NetFirewallRule `
    -DisplayName "DBX Web Guard Block" `
    -Direction Inbound `
    -Action Block `
    -Protocol TCP `
    -LocalPort $GuardPort `
    -Profile Any | Out-Null

Get-NetFirewallRule -DisplayName $rules | Select-Object DisplayName, Enabled, Direction, Action, Profile
