param(
  [string]$SourceRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path,
  [string]$ReleaseRoot,
  [string]$SnapshotsRoot,
  [string]$RemoteUrl = "https://github.com/Barbital11111/CodexDeck.git",
  [switch]$NoRemote,
  [switch]$SkipSnapshot,
  [switch]$NoCommit
)

$ErrorActionPreference = "Stop"

function Resolve-FullPath([string]$Path) {
  $expanded = [Environment]::ExpandEnvironmentVariables($Path)
  $parent = Split-Path -Parent $expanded
  if (-not [string]::IsNullOrWhiteSpace($parent) -and (Test-Path -LiteralPath $parent)) {
    $name = Split-Path -Leaf $expanded
    return (Join-Path (Resolve-Path -LiteralPath $parent).Path $name)
  }
  return [System.IO.Path]::GetFullPath($expanded)
}

function Get-WorkspaceRoot([string]$Root) {
  $current = Resolve-FullPath $Root
  while (-not [string]::IsNullOrWhiteSpace($current)) {
    if ((Split-Path -Leaf $current) -eq "codex-switch-worktrees") {
      return (Split-Path -Parent $current)
    }
    $parent = Split-Path -Parent $current
    if ($parent -eq $current) {
      break
    }
    $current = $parent
  }
  return (Split-Path -Parent $Root)
}

function Assert-SafeReleaseTarget([string]$Path, [string]$WorkspaceRoot) {
  $fullPath = Resolve-FullPath $Path
  $fullWorkspace = Resolve-FullPath $WorkspaceRoot
  if ((Split-Path -Leaf $fullPath) -ne "CodexDeck-release") {
    throw "Refuse to clean non CodexDeck-release directory: $fullPath"
  }
  if (-not $fullPath.StartsWith($fullWorkspace, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Refuse to clean directory outside workspace: $fullPath"
  }
}

function Copy-Tree([string]$From, [string]$To) {
  if (-not (Test-Path -LiteralPath $From)) {
    return
  }
  $excludeDirs = @(
    ".git",
    ".planning",
    ".dev-runtime",
    ".codex",
    ".vite",
    "node_modules",
    "dist",
    "dist-ssr",
    "release",
    "target",
    "debug",
    "gen",
    "local-reference-repos",
    "local-release-staging",
    "local-review-logs"
  )
  $excludeFiles = @(
    "*.log",
    "*.local",
    ".env",
    ".env.*",
    "*.key",
    "*.pem",
    "*.p12",
    "*.pfx",
    "*.sig",
    "accounts.json",
    "accounts.*.json",
    "auth.json",
    "config.toml",
    "export-codexdeck-release.ps1",
    "ReferenceAccountCard.tsx",
    "test-edge-shot.png"
  )

  New-Item -ItemType Directory -Force -Path $To | Out-Null
  Get-ChildItem -LiteralPath $From -Force | ForEach-Object {
    if ($_.PSIsContainer) {
      if ($excludeDirs -contains $_.Name) {
        return
      }
      Copy-Tree -From $_.FullName -To (Join-Path $To $_.Name)
      return
    }

    foreach ($pattern in $excludeFiles) {
      if ($_.Name -like $pattern) {
        return
      }
    }
    Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $To $_.Name) -Force
  }
}

function Set-JsonFile([string]$Path, [scriptblock]$Edit) {
  $json = Get-Content -Raw -LiteralPath $Path | ConvertFrom-Json
  & $Edit $json
  $json | ConvertTo-Json -Depth 100 | Set-Content -LiteralPath $Path -NoNewline -Encoding UTF8
}

function Set-TextReplace([string]$Path, [hashtable]$Map) {
  if (-not (Test-Path -LiteralPath $Path)) {
    return
  }
  $text = Get-Content -Raw -LiteralPath $Path
  foreach ($key in $Map.Keys) {
    $text = $text.Replace($key, $Map[$key])
  }
  Set-Content -LiteralPath $Path -Value $text -NoNewline -Encoding UTF8
}

function Write-MinimalReadme([string]$Path) {
  $content = @'
# CodexDeck

CodexDeck is a desktop workspace for Codex account management, API provider switching, usage views, and notification routes.

## Focus

- Manage Codex OAuth accounts, API providers, and account groups.
- Track account and API usage, with manual refresh and planned polling refresh.
- Configure notification data sources, delivery channels, message templates, and notification rules.

## Local Development

```bash
npm install
npm run dev:desktop
```

Browser preview:

```bash
npm run dev
```

## Verification

```bash
npm run lint
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

## Release Hygiene

Before public release, regenerate sanitized screenshots and confirm that the repository does not contain real accounts, tokens, API keys, webhooks, chat IDs, private keys, signing keys, or local runtime caches.

## License

MIT
'@
  Set-Content -LiteralPath $Path -Value $content -NoNewline -Encoding UTF8
}

$source = Resolve-FullPath $SourceRoot
$workspace = Get-WorkspaceRoot $source
if ([string]::IsNullOrWhiteSpace($ReleaseRoot)) {
  $ReleaseRoot = Join-Path $workspace "CodexDeck-release"
}
if ([string]::IsNullOrWhiteSpace($SnapshotsRoot)) {
  $SnapshotsRoot = Join-Path $workspace "CodexDeck-snapshots"
}
$release = Resolve-FullPath $ReleaseRoot
$snapshots = Resolve-FullPath $SnapshotsRoot

if (-not (Test-Path -LiteralPath $source)) {
  throw "Source directory does not exist: $source"
}

Assert-SafeReleaseTarget -Path $release -WorkspaceRoot $workspace
New-Item -ItemType Directory -Force -Path $release | Out-Null
New-Item -ItemType Directory -Force -Path $snapshots | Out-Null

Get-ChildItem -LiteralPath $release -Force | Where-Object { $_.Name -ne ".git" } | ForEach-Object {
  Remove-Item -LiteralPath $_.FullName -Recurse -Force
}

$items = @(
  ".github",
  "public",
  "scripts",
  "src",
  "src-tauri",
  ".gitignore",
  "changelog.md",
  "eslint.config.js",
  "index.html",
  "LICENSE",
  "package.json",
  "package-lock.json",
  "README.md",
  "tsconfig.app.json",
  "tsconfig.json",
  "tsconfig.node.json",
  "vite.config.ts"
)

foreach ($item in $items) {
  $from = Join-Path $source $item
  $to = Join-Path $release $item
  if (Test-Path -LiteralPath $from -PathType Container) {
    Copy-Tree -From $from -To $to
  } elseif (Test-Path -LiteralPath $from -PathType Leaf) {
    Copy-Item -LiteralPath $from -Destination $to -Force
  }
}

Write-MinimalReadme -Path (Join-Path $release "README.md")

Set-JsonFile -Path (Join-Path $release "package.json") -Edit {
  param($json)
  $json.name = "codexdeck"
  $json.homepage = "https://github.com/Barbital11111/CodexDeck"
  $json.bugs.url = "https://github.com/Barbital11111/CodexDeck/issues"
  $json.repository.url = "https://github.com/Barbital11111/CodexDeck.git"
}

Set-TextReplace -Path (Join-Path $release "package-lock.json") -Map @{
  '"name": "codex-switch"' = '"name": "codexdeck"'
  'https://github.com/Barbital11111/codex-switch' = 'https://github.com/Barbital11111/CodexDeck'
}

Set-TextReplace -Path (Join-Path $release "src-tauri\Cargo.toml") -Map @{
  'repository = "https://github.com/Barbital11111/codex-switch"' = 'repository = "https://github.com/Barbital11111/CodexDeck"'
}

Set-TextReplace -Path (Join-Path $release "src\constants\externalLinks.ts") -Map @{
  'https://github.com/Barbital11111/codex-switch' = 'https://github.com/Barbital11111/CodexDeck'
  'github.com/Barbital11111/codex-switch' = 'github.com/Barbital11111/CodexDeck'
}

Set-TextReplace -Path (Join-Path $release "scripts\build-local-signed-release.ps1") -Map @{
  'codex-tools-updater.key' = 'codexdeck-updater.key'
  'codex-tools' = 'codexdeck'
}

Set-TextReplace -Path (Join-Path $release "scripts\prepare-manual-release.ps1") -Map @{
  'Codex Tools' = 'CodexDeck'
  'codex-tools' = 'codexdeck'
}

Set-JsonFile -Path (Join-Path $release "src-tauri\tauri.conf.json") -Edit {
  param($json)
  $json.productName = "CodexDeck"
  $json.mainBinaryName = "CodexDeck"
  $json.app.windows[0].title = "CodexDeck"
  $json.plugins.updater.endpoints = @("https://github.com/Barbital11111/CodexDeck/releases/latest/download/latest.json")
}

Set-TextReplace -Path (Join-Path $release "index.html") -Map @{
  '/codex-tools.png' = '/codexdeck.png'
  '<title>codex-tools</title>' = '<title>CodexDeck</title>'
}

Push-Location $release
try {
  if (-not (Test-Path -LiteralPath ".git")) {
    git init -b main | Out-Null
  }
  if (-not $NoRemote) {
    $existingRemote = git remote 2>$null
    if ($existingRemote -notcontains "origin") {
      git remote add origin $RemoteUrl
    } else {
      git remote set-url origin $RemoteUrl
    }
  }

  git add -A
  if (-not $NoCommit) {
    $status = git status --porcelain
    if (-not [string]::IsNullOrWhiteSpace($status)) {
      git commit -m "chore: export CodexDeck release snapshot" | Out-Null
    }
  }

  if (-not $SkipSnapshot) {
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $bundlePath = Join-Path $snapshots "CodexDeck-$stamp.bundle"
    $zipPath = Join-Path $snapshots "CodexDeck-$stamp.zip"
    git bundle create $bundlePath --all | Out-Null
    if (Test-Path -LiteralPath $zipPath) {
      Remove-Item -LiteralPath $zipPath -Force
    }
    Compress-Archive -Path (Join-Path $release "*") -DestinationPath $zipPath -Force
  }
} finally {
  Pop-Location
}

Write-Host "CodexDeck release export complete:"
Write-Host "  Release:   $release"
Write-Host "  Snapshots: $snapshots"
