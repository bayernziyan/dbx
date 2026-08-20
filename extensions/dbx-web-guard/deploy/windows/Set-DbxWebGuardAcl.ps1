param(
    [Parameter(Mandatory = $true)]
    [string]$GuardRoot,
    [string]$ServiceIdentity = "NT AUTHORITY\LOCAL SERVICE"
)

$resolvedRoot = (Resolve-Path -LiteralPath $GuardRoot).Path
$protectedPaths = @(
    (Join-Path $resolvedRoot "secrets"),
    (Join-Path $resolvedRoot "state")
)

foreach ($path in $protectedPaths) {
    New-Item -ItemType Directory -Force -Path $path | Out-Null
    $acl = Get-Acl -LiteralPath $path
    $acl.SetAccessRuleProtection($true, $false)
    foreach ($rule in @($acl.Access)) {
        $acl.RemoveAccessRuleAll($rule)
    }
    foreach ($identity in @("NT AUTHORITY\SYSTEM", "BUILTIN\Administrators", $ServiceIdentity)) {
        $rule = New-Object System.Security.AccessControl.FileSystemAccessRule(
            $identity,
            "FullControl",
            "ContainerInherit,ObjectInherit",
            "None",
            "Allow"
        )
        $acl.AddAccessRule($rule)
    }
    Set-Acl -LiteralPath $path -AclObject $acl
}
