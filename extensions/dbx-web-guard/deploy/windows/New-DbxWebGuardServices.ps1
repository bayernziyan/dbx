param(
    [Parameter(Mandatory = $true)]
    [string]$WinSwExecutable,
    [Parameter(Mandatory = $true)]
    [string]$ServiceDirectory,
    [Parameter(Mandatory = $true)]
    [string]$ServerRoot,
    [Parameter(Mandatory = $true)]
    [string]$GuardRoot,
    [Parameter(Mandatory = $true)]
    [string]$NginxRoot,
    [switch]$Install
)

function ConvertTo-XmlText([string]$Value) {
    return [System.Security.SecurityElement]::Escape($Value)
}

function Write-Utf8NoBom([string]$Path, [string]$Content) {
    [System.IO.File]::WriteAllText($Path, $Content, [System.Text.UTF8Encoding]::new($false))
}

$winSw = (Resolve-Path -LiteralPath $WinSwExecutable).Path
$serviceRoot = [System.IO.Path]::GetFullPath($ServiceDirectory)
$server = (Resolve-Path -LiteralPath $ServerRoot).Path
$guard = (Resolve-Path -LiteralPath $GuardRoot).Path
$nginx = (Resolve-Path -LiteralPath $NginxRoot).Path
New-Item -ItemType Directory -Force -Path $serviceRoot | Out-Null

$backendWrapper = Join-Path $serviceRoot "DBXWebBackend.exe"
$guardWrapper = Join-Path $serviceRoot "DBXWebGuard.exe"
$nginxWrapper = Join-Path $serviceRoot "DBXNginx.exe"
Copy-Item -LiteralPath $winSw -Destination $backendWrapper -Force
Copy-Item -LiteralPath $winSw -Destination $guardWrapper -Force
Copy-Item -LiteralPath $winSw -Destination $nginxWrapper -Force

$backendScript = ConvertTo-XmlText (Join-Path $server "start-dbx-web.ps1")
$guardScript = ConvertTo-XmlText (Join-Path $guard "start-dbx-web-guard.ps1")
$serverXml = ConvertTo-XmlText $server
$guardXml = ConvertTo-XmlText $guard
$nginxExe = ConvertTo-XmlText (Join-Path $nginx "nginx.exe")
$nginxXml = ConvertTo-XmlText $nginx

$backendConfig = @"
<service>
  <id>DBXWebBackend</id>
  <name>DBX Web Backend</name>
  <description>Internal DBX Web backend for the guarded LAN deployment.</description>
  <executable>powershell.exe</executable>
  <arguments>-NoProfile -NonInteractive -ExecutionPolicy Bypass -File &quot;$backendScript&quot; -ServerRoot &quot;$serverXml&quot; -Port 4225 -PublicBasePath /dbx</arguments>
  <workingdirectory>$serverXml</workingdirectory>
  <startmode>Automatic</startmode>
  <onfailure action="restart" delay="5 sec" />
  <stoptimeout>20 sec</stoptimeout>
</service>
"@

$guardConfig = @"
<service>
  <id>DBXWebGuard</id>
  <name>DBX Web Guard</name>
  <description>Dual-role access guard for DBX Web.</description>
  <executable>powershell.exe</executable>
  <arguments>-NoProfile -NonInteractive -ExecutionPolicy Bypass -File &quot;$guardScript&quot; -GuardRoot &quot;$guardXml&quot;</arguments>
  <workingdirectory>$guardXml</workingdirectory>
  <startmode>Automatic</startmode>
  <delayedAutoStart>true</delayedAutoStart>
  <depend>DBXWebBackend</depend>
  <onfailure action="restart" delay="5 sec" />
  <stoptimeout>20 sec</stoptimeout>
</service>
"@

$nginxConfig = @"
<service>
  <id>DBXNginx</id>
  <name>DBX Nginx</name>
  <description>Nginx entry point for guarded DBX Web.</description>
  <executable>$nginxExe</executable>
  <arguments>-p &quot;$nginxXml&quot; -c conf/nginx.conf</arguments>
  <stopexecutable>$nginxExe</stopexecutable>
  <stoparguments>-p &quot;$nginxXml&quot; -s quit</stoparguments>
  <workingdirectory>$nginxXml</workingdirectory>
  <startmode>Automatic</startmode>
  <delayedAutoStart>true</delayedAutoStart>
  <depend>DBXWebGuard</depend>
  <onfailure action="restart" delay="5 sec" />
  <stoptimeout>20 sec</stoptimeout>
</service>
"@

Write-Utf8NoBom (Join-Path $serviceRoot "DBXWebBackend.xml") $backendConfig
Write-Utf8NoBom (Join-Path $serviceRoot "DBXWebGuard.xml") $guardConfig
Write-Utf8NoBom (Join-Path $serviceRoot "DBXNginx.xml") $nginxConfig

if ($Install) {
    $servicePairs = @(
        @{ Id = "DBXWebBackend"; Wrapper = $backendWrapper },
        @{ Id = "DBXWebGuard"; Wrapper = $guardWrapper },
        @{ Id = "DBXNginx"; Wrapper = $nginxWrapper }
    )
    foreach ($pair in $servicePairs) {
        $wrapper = $pair.Wrapper
        & $wrapper install
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to install Windows service wrapper: $wrapper"
        }
        & sc.exe config $pair.Id obj= "NT AUTHORITY\LocalService" password= ""
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to configure LocalService identity for $($pair.Id)"
        }
    }
}

Write-Output $serviceRoot
