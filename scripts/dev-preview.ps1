$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$devRoot = Join-Path $repoRoot ".dev-runtime"
$repoParent = Split-Path -Parent $repoRoot
$externalDevRoot = Join-Path $repoParent "CodexDeck-dev-runtime"
$appDataDir = Join-Path $devRoot "app-data"
$copyProdData = $env:CODEX_SWITCH_DEV_COPY_PROD -eq "1"

New-Item -ItemType Directory -Force -Path $appDataDir | Out-Null

function Copy-IfMissing {
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$Destination
    )

    if (!(Test-Path -LiteralPath $Source) -or (Test-Path -LiteralPath $Destination)) {
        return
    }

    $parent = Split-Path -Parent $Destination
    if ($parent) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }

    Copy-Item -LiteralPath $Source -Destination $Destination -Recurse -Force
}

function Set-JsonProperty {
    param(
        [Parameter(Mandatory = $true)]$Object,
        [Parameter(Mandatory = $true)][string]$Name,
        $Value
    )

    $Object | Add-Member -NotePropertyName $Name -NotePropertyValue $Value -Force
}

$prodAppDataDir = Join-Path $env:APPDATA "com.carry.codex-tools"
$futureProdAppDataDir = Join-Path $env:APPDATA "io.github.barbital11111.codex-switch"
if ($copyProdData) {
    Copy-IfMissing -Source (Join-Path $prodAppDataDir "accounts.json") -Destination (Join-Path $appDataDir "accounts.json")
    Copy-IfMissing -Source (Join-Path $prodAppDataDir "profiles") -Destination (Join-Path $appDataDir "profiles")
    Copy-IfMissing -Source (Join-Path $futureProdAppDataDir "accounts.json") -Destination (Join-Path $appDataDir "accounts.json")
    Copy-IfMissing -Source (Join-Path $futureProdAppDataDir "profiles") -Destination (Join-Path $appDataDir "profiles")
} else {
    Write-Host "Dev preview will NOT copy production account cache by default."
    Write-Host "Set CODEX_SWITCH_DEV_COPY_PROD=1 before launch if you intentionally need an isolated production-data copy."
}

$devAccountsPath = Join-Path $appDataDir "accounts.json"
if (Test-Path -LiteralPath $devAccountsPath) {
    try {
        $store = Get-Content -LiteralPath $devAccountsPath -Raw | ConvertFrom-Json
        if ($store.settings) {
            Set-JsonProperty -Object $store.settings -Name "launchAtStartup" -Value $false
            Set-JsonProperty -Object $store.settings -Name "codexLaunchPath" -Value $null
            Set-JsonProperty -Object $store.settings -Name "apiEnhancedLaunchEnabled" -Value $false
            Set-JsonProperty -Object $store.settings -Name "codexMultiModelModeEnabled" -Value $false
            Set-JsonProperty -Object $store.settings -Name "codexMultiModelStatus" -Value $null
            Set-JsonProperty -Object $store.settings -Name "codexMultiModelWorkspace" -Value $null
            Set-JsonProperty -Object $store.settings -Name "codexMultiModelRestorePoint" -Value $null
            Set-JsonProperty -Object $store.settings -Name "codexMultiModelControlledAppRoot" -Value $null
            Set-JsonProperty -Object $store.settings -Name "codexMultiModelControlledExePath" -Value $null
            Set-JsonProperty -Object $store.settings -Name "codexMultiModelControlledAppAsarPath" -Value $null
            Set-JsonProperty -Object $store.settings -Name "codexMultiModelSourceAppRoot" -Value $null
            Set-JsonProperty -Object $store.settings -Name "codexMultiModelPatchStatePath" -Value $null
            $store | ConvertTo-Json -Depth 100 | Set-Content -LiteralPath $devAccountsPath -Encoding UTF8
        }
    } catch {
        Write-Warning ("Failed to rewrite dev preview launchAtStartup setting: {0}" -f $_.Exception.Message)
    }
}

$env:CODEX_SWITCH_DEV_DATA_DIR = $appDataDir
Remove-Item Env:CODEX_SWITCH_DEV_CODEX_DIR -ErrorAction SilentlyContinue
Remove-Item Env:CODEX_TOOLS_DEV_CODEX_DIR -ErrorAction SilentlyContinue

$cargoBin = Join-Path $env:USERPROFILE ".cargo\\bin"
if (Test-Path -LiteralPath $cargoBin) {
    $env:PATH = "$cargoBin;$env:PATH"
}

$rustToolchainBin = Join-Path $env:USERPROFILE ".rustup\\toolchains\\stable-x86_64-pc-windows-msvc\\bin"
if (Test-Path -LiteralPath $rustToolchainBin) {
    $env:PATH = "$rustToolchainBin;$env:PATH"
    $rustcBin = Join-Path $rustToolchainBin "rustc.exe"
    if (Test-Path -LiteralPath $rustcBin) {
        $env:RUSTC = $rustcBin
    }
}

Write-Host "Dev preview will use isolated directories:"
Write-Host ("  app data: {0}" -f $appDataDir)
Write-Host ("  codex dir: {0}" -f (Join-Path $env:USERPROFILE ".codex"))

$devTauriConfigPath = Join-Path $devRoot "tauri.dev.conf.json"
$devTauriConfig = @{
    productName = "CodexDeck Dev"
    identifier = "io.github.barbital11111.codex-switch.dev"
    app = @{
        windows = @(
            @{
                title = "CodexDeck Dev"
                width = 1320
                height = 960
                resizable = $true
                decorations = $false
                shadow = $true
            }
        )
    }
} | ConvertTo-Json -Depth 8
Set-Content -LiteralPath $devTauriConfigPath -Value $devTauriConfig -Encoding UTF8
Write-Host ("  tauri config: {0}" -f $devTauriConfigPath)

Set-Location $repoRoot
npm run tauri -- dev --config $devTauriConfigPath
