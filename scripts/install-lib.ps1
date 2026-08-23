# sb-agent-core — librería de instalación compartida (Windows)
#
# No se ejecuta sola. Cada install.ps1 de un agente la descarga y la dot-sourcea,
# fija sus propias constantes/config, y llama a estas funciones.
# Ver sb-agent-core/TODO.md, sección "Lo que no es Rust".
#
# Uso típico en el install.ps1 de un agente:
#
#   $libUrl = "https://raw.githubusercontent.com/securyblack/sb-agent-core/main/scripts/install-lib.ps1"
#   $libTmp = New-TemporaryFile
#   Invoke-WebRequest -Uri $libUrl -OutFile $libTmp -UseBasicParsing
#   . $libTmp.FullName
#
#   $SbAgentLabel = "oxipulse"
#   Assert-SbAdmin
#   $target  = Get-SbArchTarget
#   $version = Get-SbLatestVersion -GithubRepo "securyblack/oxi-pulse"
#   $zipPath = Get-SbReleaseAsset -GithubRepo "securyblack/oxi-pulse" -Version $version `
#                -AssetName "oxipulse-$target.zip" -TmpDir $tmpDir
#   Install-SbBinaryFromZip -ZipPath $zipPath -BinaryName "oxipulse.exe" -InstallDir $installDir -ServiceName "OxiPulse"
#   Register-SbWindowsService -ServiceName "OxiPulse" -DisplayName "OxiPulse Monitoring Agent" `
#                -BinaryPath "$installDir\oxipulse.exe" -Description "..."

$SbAgentLabel = if ($SbAgentLabel) { $SbAgentLabel } else { "sb-agent" }

# Set TLS 1.2 for PowerShell 5.1 compatibility on Windows Server — necesario en
# todos los agentes, no solo en el que lo tenía escrito.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 -bor [Net.SecurityProtocolType]::Tls11 -bor [Net.SecurityProtocolType]::Tls

# ─── Logging ──────────────────────────────────────────────────────────────────
function Write-SbInfo    { param($msg) Write-Host "[$SbAgentLabel] $msg" -ForegroundColor Cyan }
function Write-SbSuccess { param($msg) Write-Host "[$SbAgentLabel] $msg" -ForegroundColor Green }
function Write-SbWarn    { param($msg) Write-Host "[$SbAgentLabel] $msg" -ForegroundColor Yellow }
function Invoke-SbFail   { param($msg) Write-Host "[$SbAgentLabel] ERROR: $msg" -ForegroundColor Red; exit 1 }

# ─── Requisitos ───────────────────────────────────────────────────────────────
function Assert-SbAdmin {
    $currentPrincipal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
    if (-not $currentPrincipal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        Invoke-SbFail "This script must be run as Administrator. Right-click PowerShell and select 'Run as Administrator'."
    }
}

# ─── Arquitectura ─────────────────────────────────────────────────────────────
function Get-SbArchTarget {
    $procArch = $env:PROCESSOR_ARCHITECTURE
    $target = switch ($procArch) {
        "AMD64" { "x86_64-pc-windows-msvc" }
        "ARM64" { "aarch64-pc-windows-msvc" }
        default { Invoke-SbFail "Unsupported architecture: $procArch" }
    }
    Write-SbInfo "Detected architecture: $procArch ($target)"
    return $target
}

# ─── Última versión publicada en GitHub Releases ─────────────────────────────
function Get-SbLatestVersion {
    param([Parameter(Mandatory)][string]$GithubRepo)
    Write-SbInfo "Fetching latest release from GitHub..."
    $releaseApi = "https://api.github.com/repos/$GithubRepo/releases/latest"
    try {
        $releaseInfo = Invoke-RestMethod -Uri $releaseApi -Headers @{ "User-Agent" = "$SbAgentLabel-installer" }
    } catch {
        Invoke-SbFail "Could not reach GitHub API. Check your internet connection."
    }
    $version = $releaseInfo.tag_name
    if (-not $version) { Invoke-SbFail "Could not determine latest version. Check your internet connection." }
    Write-SbInfo "Latest version: $version"
    return $version
}

# ─── Descarga + verificación de checksum ─────────────────────────────────────
# Descarga $AssetName al $TmpDir y verifica su .sha256 si existe (warning, no
# error, si no existe — igual que en Linux). Devuelve la ruta al zip descargado.
function Get-SbReleaseAsset {
    param(
        [Parameter(Mandatory)][string]$GithubRepo,
        [Parameter(Mandatory)][string]$Version,
        [Parameter(Mandatory)][string]$AssetName,
        [Parameter(Mandatory)][string]$TmpDir
    )
    $downloadUrl = "https://github.com/$GithubRepo/releases/download/$Version/$AssetName"
    $checksumUrl = "$downloadUrl.sha256"
    $zipPath = Join-Path $TmpDir $AssetName

    Write-SbInfo "Downloading $AssetName..."
    Invoke-WebRequest -Uri $downloadUrl -OutFile $zipPath -UseBasicParsing

    try {
        $checksumFile = "$zipPath.sha256"
        Invoke-WebRequest -Uri $checksumUrl -OutFile $checksumFile -UseBasicParsing
        $expected = (Get-Content $checksumFile).Split(" ")[0].Trim().ToLower()
        $actual   = (Get-FileHash -Algorithm SHA256 $zipPath).Hash.ToLower()
        if ($expected -ne $actual) { Invoke-SbFail "Checksum mismatch. Download may be corrupted." }
        Write-SbSuccess "Checksum OK"
    } catch {
        Write-SbWarn "No checksum file found, skipping verification"
    }

    return $zipPath
}

# ─── Instalación del binario desde un .zip ───────────────────────────────────
# Para de forma segura el servicio existente (el binario puede estar bloqueado),
# extrae el zip y copia el binario al directorio de instalación.
function Install-SbBinaryFromZip {
    param(
        [Parameter(Mandatory)][string]$ZipPath,
        [Parameter(Mandatory)][string]$BinaryName,
        [Parameter(Mandatory)][string]$InstallDir,
        [Parameter(Mandatory)][string]$ServiceName
    )
    if (Get-Service -Name $ServiceName -ErrorAction SilentlyContinue) {
        Write-SbInfo "Stopping existing service '$ServiceName'..."
        Stop-Service -Name $ServiceName -Force -ErrorAction SilentlyContinue
        & sc.exe delete $ServiceName | Out-Null
        Start-Sleep -Seconds 2
    }

    Write-SbInfo "Installing binary to $InstallDir..."
    $extractDir = Join-Path (Split-Path $ZipPath -Parent) "extracted"
    Expand-Archive -Path $ZipPath -DestinationPath $extractDir -Force
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    Copy-Item (Join-Path $extractDir $BinaryName) (Join-Path $InstallDir $BinaryName) -Force
    Write-SbSuccess "Binary installed"
}

# ─── Registro del servicio Windows ───────────────────────────────────────────
# New-Service + política de reinicio ante fallos, incluyendo exit limpio
# (failureflag 1) — necesario para que el auto-update reinicie el servicio
# tras el self_update, que sale con código 0 después de reemplazar el binario.
function Register-SbWindowsService {
    param(
        [Parameter(Mandatory)][string]$ServiceName,
        [Parameter(Mandatory)][string]$DisplayName,
        [Parameter(Mandatory)][string]$BinaryPath,
        [Parameter(Mandatory)][string]$Description
    )
    Write-SbInfo "Registering Windows Service '$ServiceName'..."
    New-Service -Name $ServiceName `
                -BinaryPathName $BinaryPath `
                -DisplayName $DisplayName `
                -Description $Description `
                -StartupType Automatic | Out-Null

    & sc.exe failure $ServiceName reset= 86400 actions= restart/10000/restart/30000/restart/60000 | Out-Null
    & sc.exe failureflag $ServiceName 1 | Out-Null

    Start-Service -Name $ServiceName
    Write-SbSuccess "Service registered and started"
}

# ─── Restricción de ACL de un fichero de config a Administrators + SYSTEM ────
# Usa SIDs conocidos (locale-independent) para no fallar en Windows no-inglés.
function Protect-SbConfigFile {
    param([Parameter(Mandatory)][string]$Path)
    $acl = Get-Acl $Path
    $acl.SetAccessRuleProtection($true, $false)
    $adminSid  = New-Object System.Security.Principal.SecurityIdentifier(
        [System.Security.Principal.WellKnownSidType]::BuiltinAdministratorsSid, $null)
    $systemSid = New-Object System.Security.Principal.SecurityIdentifier(
        [System.Security.Principal.WellKnownSidType]::LocalSystemSid, $null)
    $adminRule  = New-Object System.Security.AccessControl.FileSystemAccessRule($adminSid, "FullControl", "Allow")
    $systemRule = New-Object System.Security.AccessControl.FileSystemAccessRule($systemSid, "FullControl", "Allow")
    $acl.AddAccessRule($adminRule)
    $acl.AddAccessRule($systemRule)
    Set-Acl -Path $Path -AclObject $acl
}
