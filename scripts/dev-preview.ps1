$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$devRoot = Join-Path $repoRoot ".dev-runtime"
$appDataDir = Join-Path $devRoot "app-data"
$codexDir = Join-Path $devRoot "codex"

New-Item -ItemType Directory -Force -Path $appDataDir | Out-Null
New-Item -ItemType Directory -Force -Path $codexDir | Out-Null

Write-Host "Dev preview uses an isolated CodexDeck data directory."

$devAccountsPath = Join-Path $appDataDir "accounts.json"
if (Test-Path -LiteralPath $devAccountsPath) {
    try {
        $store = Get-Content -LiteralPath $devAccountsPath -Raw | ConvertFrom-Json
        if ($store.settings) {
            $store.settings.launchAtStartup = $false
            $store | ConvertTo-Json -Depth 100 | Set-Content -LiteralPath $devAccountsPath -Encoding UTF8
        }
    } catch {
        Write-Warning ("Failed to rewrite dev preview launchAtStartup setting: {0}" -f $_.Exception.Message)
    }
}

$env:CODEXDECK_DEV_DATA_DIR = $appDataDir
$env:CODEXDECK_DEV_CODEX_DIR = $codexDir

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
Write-Host ("  codex dir: {0}" -f $codexDir)

$devTauriConfigPath = Join-Path $devRoot "tauri.dev.conf.json"
$devTauriConfig = @{
    productName = "CodexDeck Dev"
    identifier = "io.github.barbital11111.codexdeck.dev"
    app = @{
        windows = @(
            @{
                title = "CodexDeck Dev"
                width = 1320
                height = 960
                resizable = $true
            }
        )
    }
} | ConvertTo-Json -Depth 8
Set-Content -LiteralPath $devTauriConfigPath -Value $devTauriConfig -Encoding UTF8
Write-Host ("  tauri config: {0}" -f $devTauriConfigPath)

Set-Location $repoRoot
npm run tauri -- dev --config $devTauriConfigPath
