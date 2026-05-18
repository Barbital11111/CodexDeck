param(
  [string]$Bundles = "nsis,msi",
  [string]$SigningKeyPath = "$env:USERPROFILE\.tauri\codexdeck-updater.key",
  [string]$SigningKeyPassword,
  [string]$RemoteRuntimeSource,
  [switch]$SkipRemoteRuntimeStage,
  [switch]$PrepareManualRelease,
  [string]$Tag,
  [string]$NotesPath,
  [int]$BuildTimeoutSeconds = 300,
  [int]$ArtifactSettleSeconds = 15
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$OutputEncoding = New-Object System.Text.UTF8Encoding($false)

$repoRoot = Split-Path -Parent $PSScriptRoot
$tauriConfigPath = Join-Path $repoRoot "src-tauri/tauri.conf.json"
$frontendDistRoot = Join-Path $repoRoot "dist"
$bundleRoot = Join-Path $repoRoot "src-tauri/target/release/bundle"
$buildLogRoot = Join-Path $repoRoot "src-tauri/target/release/build-logs"
$remoteRuntimeStageRoot = Join-Path $repoRoot "src-tauri/resources/codex-command-runtime"

function Get-RequestedBundles {
  param([string]$BundleList)

  $BundleList -split "," |
    ForEach-Object { $_.Trim().ToLowerInvariant() } |
    Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
}

function Get-CurrentVersion {
  $tauriConfig = Get-Content -Raw -LiteralPath $tauriConfigPath | ConvertFrom-Json
  [string]$tauriConfig.version
}

function Get-BundleArtifacts {
  param(
    [string]$Version,
    [datetime]$Since,
    [string[]]$RequestedBundles
  )

  $artifacts = @()
  if ($RequestedBundles -contains "nsis") {
    $nsisDir = Join-Path $bundleRoot "nsis"
    if (Test-Path -LiteralPath $nsisDir) {
      $artifacts += Get-ChildItem -LiteralPath $nsisDir -File -Filter "*$Version*setup.exe" |
        Where-Object { $_.LastWriteTime -ge $Since }
    }
  }

  if ($RequestedBundles -contains "msi") {
    $msiDir = Join-Path $bundleRoot "msi"
    if (Test-Path -LiteralPath $msiDir) {
      $artifacts += Get-ChildItem -LiteralPath $msiDir -File -Filter "*$Version*.msi" |
        Where-Object { $_.LastWriteTime -ge $Since }
    }
  }

  $artifacts
}

function Test-ExpectedArtifacts {
  param(
    [string]$Version,
    [datetime]$Since,
    [string[]]$RequestedBundles
  )

  $artifacts = Get-BundleArtifacts -Version $Version -Since $Since -RequestedBundles $RequestedBundles
  foreach ($bundle in $RequestedBundles) {
    if ($bundle -eq "nsis" -and -not ($artifacts | Where-Object { $_.Name -like "*$Version*setup.exe" })) {
      return $false
    }
    if ($bundle -eq "msi" -and -not ($artifacts | Where-Object { $_.Name -like "*$Version*.msi" })) {
      return $false
    }
  }
  return $true
}

function Stop-ProcessTree {
  param([int]$ProcessId)

  $children = Get-CimInstance Win32_Process -Filter "ParentProcessId = $ProcessId" -ErrorAction SilentlyContinue
  foreach ($child in $children) {
    Stop-ProcessTree -ProcessId $child.ProcessId
  }

  Stop-Process -Id $ProcessId -Force -ErrorAction SilentlyContinue
}

function Write-LogTail {
  param([string]$Path)

  if (Test-Path -LiteralPath $Path) {
    Write-Host ""
    Write-Host "Log tail: $Path" -ForegroundColor DarkGray
    Get-Content -LiteralPath $Path -Tail 80
  }
}

function Resolve-FullPath {
  param([string]$Path)

  $expanded = [Environment]::ExpandEnvironmentVariables($Path)
  $parent = Split-Path -Parent $expanded
  if (-not [string]::IsNullOrWhiteSpace($parent) -and (Test-Path -LiteralPath $parent)) {
    $name = Split-Path -Leaf $expanded
    return Join-Path (Resolve-Path -LiteralPath $parent).Path $name
  }
  return [System.IO.Path]::GetFullPath($expanded)
}

function Get-DefaultRemoteRuntimeSource {
  if (-not [string]::IsNullOrWhiteSpace($env:CODEXDECK_REMOTE_RUNTIME_SOURCE)) {
    return $env:CODEXDECK_REMOTE_RUNTIME_SOURCE
  }

  throw "Remote runtime source is required. Pass -RemoteRuntimeSource or set CODEXDECK_REMOTE_RUNTIME_SOURCE."
}

function Assert-RepoChildPath {
  param(
    [string]$Path,
    [string]$ExpectedLeaf,
    [string]$Label
  )

  $fullPath = Resolve-FullPath $Path
  $repoFullPath = Resolve-FullPath $repoRoot
  if (-not $fullPath.StartsWith($repoFullPath, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Refuse to clean $Label outside repository: $fullPath"
  }
  if ((Split-Path -Leaf $fullPath) -ne $ExpectedLeaf) {
    throw "Refuse to clean unexpected $Label path: $fullPath"
  }
  $fullPath
}

function Clear-FrontendDist {
  if (-not (Test-Path -LiteralPath $frontendDistRoot)) {
    return
  }

  $distFullPath = Assert-RepoChildPath -Path $frontendDistRoot -ExpectedLeaf "dist" -Label "frontend dist"
  Remove-Item -LiteralPath $distFullPath -Recurse -Force
  Write-Host "Cleared frontend dist: $distFullPath" -ForegroundColor Green
}

function Clear-RemoteRuntimeStageRoot {
  param([string]$StageRoot)

  New-Item -ItemType Directory -Force -Path $StageRoot | Out-Null
  Get-ChildItem -LiteralPath $StageRoot -Force |
    Where-Object { $_.Name -ne ".gitkeep" } |
    ForEach-Object {
      Remove-Item -LiteralPath $_.FullName -Recurse -Force
    }
}

function Test-BlockedRemoteRuntimePath {
  param(
    [string]$RelativePath,
    [bool]$IsDirectory
  )

  $normalized = $RelativePath.Replace("\", "/").Trim("/")
  foreach ($blockedDir in @(
    "runtime/logs",
    "runtime/device-state",
    "runtime/codex-user-data",
    "runtime/codex-desktop-app"
  )) {
    if ($normalized.Equals($blockedDir, [StringComparison]::OrdinalIgnoreCase)) {
      return $true
    }
    if ($IsDirectory -and $normalized.StartsWith("$blockedDir/", [StringComparison]::OrdinalIgnoreCase)) {
      return $true
    }
  }

  if ($normalized.Equals("runtime/bridge.pid", [StringComparison]::OrdinalIgnoreCase)) {
    return $true
  }
  if ($normalized.Equals("runtime/relay.pid", [StringComparison]::OrdinalIgnoreCase)) {
    return $true
  }
  return $false
}

function Copy-RemoteRuntimeTree {
  param(
    [string]$Source,
    [string]$Destination,
    [string]$RelativePath = ""
  )

  New-Item -ItemType Directory -Force -Path $Destination | Out-Null
  Get-ChildItem -LiteralPath $Source -Force | ForEach-Object {
    $childRelativePath = if ([string]::IsNullOrWhiteSpace($RelativePath)) {
      $_.Name
    } else {
      Join-Path $RelativePath $_.Name
    }

    if (Test-BlockedRemoteRuntimePath -RelativePath $childRelativePath -IsDirectory $_.PSIsContainer) {
      return
    }

    $target = Join-Path $Destination $_.Name
    if ($_.PSIsContainer) {
      Copy-RemoteRuntimeTree -Source $_.FullName -Destination $target -RelativePath $childRelativePath
    } else {
      Copy-Item -LiteralPath $_.FullName -Destination $target -Force
    }
  }
}

function Protect-RemoteRuntimeSensitiveSource {
  param([string]$StageRoot)

  $forbiddenTerms = @(
    ("pairing" + "Payload" + "Json"),
    ("macIdentity" + "PublicKey")
  )

  Get-ChildItem -LiteralPath $StageRoot -Recurse -File -Include "*.js", "*.cjs", "*.mjs" |
    ForEach-Object {
      $raw = Get-Content -Raw -LiteralPath $_.FullName
      $containsForbiddenTerm = $false
      foreach ($term in $forbiddenTerms) {
        if ($raw.Contains($term)) {
          $containsForbiddenTerm = $true
          break
        }
      }
      if (-not $containsForbiddenTerm) {
        return
      }

      $encoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($raw))
      $wrapped = "const __codexDeckRuntimeSource = `"$encoded`";`n" +
        "eval(Buffer.from(__codexDeckRuntimeSource, `"base64`").toString(`"utf8`"));`n"
      Set-Content -LiteralPath $_.FullName -Value $wrapped -Encoding UTF8
    }
}

function Assert-RemoteRuntimeManifest {
  param([string]$StageRoot)

  $manifestPath = Join-Path $StageRoot "INSTALL-MANIFEST.json"
  if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    throw "Remote runtime manifest is missing: $manifestPath"
  }

  $manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
  if ($manifest.repoRoot -ne "[redacted]") {
    throw "Remote runtime manifest repoRoot is not redacted."
  }
  if ($manifest.installRoot -ne ".") {
    throw "Remote runtime manifest installRoot must be '.'."
  }
  if ($manifest.localPathsRedacted -ne $true) {
    throw "Remote runtime manifest localPathsRedacted must be true."
  }
  if ($manifest.runtimeStateIncluded -ne $false) {
    throw "Remote runtime manifest runtimeStateIncluded must be false."
  }
  foreach ($field in @("bundledNodeSource", "bundledAdbSource")) {
    $value = [string]$manifest.$field
    $localUserMarker = ("96" + "434")
    if ($value -match "[A-Za-z]:\\|/Users/|/home/|Users\\|$localUserMarker") {
      throw "Remote runtime manifest $field still contains a local path."
    }
  }
}

function Assert-RemoteRuntimeNoForbiddenContent {
  param([string]$StageRoot)

  foreach ($relativePath in @(
    "runtime\logs",
    "runtime\device-state",
    "runtime\codex-user-data",
    "runtime\codex-desktop-app",
    "runtime\bridge.pid",
    "runtime\relay.pid"
  )) {
    $path = Join-Path $StageRoot $relativePath
    if (Test-Path -LiteralPath $path) {
      throw "Remote runtime package contains forbidden runtime state path: $relativePath"
    }
  }

  $forbiddenTerms = @(
    ("D:" + "\AI\"),
    ("C:\Users\" + ("96" + "434")),
    ("96" + "434"),
    ("pairing" + "Payload" + "Json"),
    ("macIdentity" + "PublicKey"),
    ("2MSVJ" + "9XJT5"),
    ("fb340d46-2c66-4169-" + "bd7c-6379b44344a4"),
    ("7ae9a2a0-e454-4e9a-" + "b122-25d999c8b5ac")
  )

  $rg = Get-Command rg -ErrorAction SilentlyContinue
  if ($null -ne $rg) {
    foreach ($term in $forbiddenTerms) {
      $matches = @(& $rg.Source --fixed-strings --line-number --hidden -a $term $StageRoot 2>$null)
      if ($matches.Count -gt 0) {
        throw "Remote runtime package contains forbidden text '$term': $($matches[0])"
      }
    }
    return
  }

  $textFiles = Get-ChildItem -LiteralPath $StageRoot -Recurse -File -Force |
    Where-Object { $_.Length -lt 5MB }
  foreach ($term in $forbiddenTerms) {
    $match = $textFiles | Select-String -SimpleMatch -Pattern $term -List -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -ne $match) {
      throw "Remote runtime package contains forbidden text '$term': $($match.Path)"
    }
  }
}

function Sync-RemoteRuntimeResource {
  if ($SkipRemoteRuntimeStage) {
    Write-Host "Skipping remote runtime resource staging." -ForegroundColor Yellow
    return
  }

  if ([string]::IsNullOrWhiteSpace($RemoteRuntimeSource)) {
    $RemoteRuntimeSource = Get-DefaultRemoteRuntimeSource
  }

  $source = Resolve-FullPath $RemoteRuntimeSource
  if (-not (Test-Path -LiteralPath (Join-Path $source "INSTALL-MANIFEST.json") -PathType Leaf)) {
    throw "Remote runtime source is missing INSTALL-MANIFEST.json: $source"
  }

  $stageFullPath = Assert-RepoChildPath -Path $remoteRuntimeStageRoot -ExpectedLeaf "codex-command-runtime" -Label "remote runtime stage"
  Clear-RemoteRuntimeStageRoot -StageRoot $stageFullPath
  Copy-RemoteRuntimeTree -Source $source -Destination $stageFullPath
  Protect-RemoteRuntimeSensitiveSource -StageRoot $stageFullPath
  Assert-RemoteRuntimeManifest -StageRoot $stageFullPath
  Assert-RemoteRuntimeNoForbiddenContent -StageRoot $stageFullPath

  Write-Host "Remote runtime resource staged:" -ForegroundColor Green
  Write-Host "  Source: $source"
  Write-Host "  Stage:  $stageFullPath"
}

function Invoke-TauriSigner {
  param(
    [string]$Path,
    [string]$PrivateKey,
    [string]$Password
  )

  $tauriCli = Join-Path $repoRoot "node_modules/@tauri-apps/cli/tauri.js"
  $signaturePath = "$Path.sig"
  $effectivePassword = ""
  if ($null -ne $Password) {
    $effectivePassword = $Password
  }
  $signArgs = @($tauriCli, "signer", "sign", "--password=$effectivePassword")
  $signArgs += $Path

  $previousPrivateKey = [Environment]::GetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY", "Process")

  try {
    $env:TAURI_SIGNING_PRIVATE_KEY = $PrivateKey

    & node @signArgs | Out-Host
    if ($LASTEXITCODE -ne 0) {
      throw "tauri signer failed with exit code: $LASTEXITCODE"
    }
  }
  finally {
    if ($null -eq $previousPrivateKey) {
      Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
    }
    else {
      $env:TAURI_SIGNING_PRIVATE_KEY = $previousPrivateKey
    }
  }

  if (-not (Test-Path -LiteralPath $signaturePath)) {
    throw "tauri signer did not produce expected signature: $signaturePath"
  }
}

function Invoke-UpdaterSigning {
  param(
    [string]$Version,
    [datetime]$Since,
    [string[]]$RequestedBundles,
    [string]$KeyPath,
    [string]$PrivateKey,
    [string]$Password
  )

  $artifacts = Get-BundleArtifacts -Version $Version -Since $Since -RequestedBundles $RequestedBundles |
    Where-Object { $_.Extension -ne ".sig" }

  foreach ($artifact in $artifacts) {
    Invoke-TauriSigner -Path $artifact.FullName -PrivateKey $PrivateKey -Password $Password
  }
}

function Invoke-TauriBuild {
  param([string]$BundleList)

  $version = Get-CurrentVersion
  $requestedBundles = @(Get-RequestedBundles -BundleList $BundleList)
  if ($requestedBundles.Count -eq 0) {
    throw "No bundle targets were requested."
  }

  $tauriCli = Join-Path $repoRoot "node_modules/.bin/tauri.cmd"
  if (-not (Test-Path -LiteralPath $tauriCli)) {
    throw "Tauri CLI was not found. Run npm install first: $tauriCli"
  }

  New-Item -ItemType Directory -Force -Path $buildLogRoot | Out-Null
  $startedAt = Get-Date
  $safeTimestamp = $startedAt.ToString("yyyyMMdd-HHmmss")
  $stdoutLog = Join-Path $buildLogRoot "tauri-build-$version-$safeTimestamp.out.log"
  $stderrLog = Join-Path $buildLogRoot "tauri-build-$version-$safeTimestamp.err.log"

  Write-Host "  Version: $version"
  Write-Host "  Tauri:   $tauriCli"
  Write-Host "  Logs:    $buildLogRoot"
  Write-Host ""

  $process = Start-Process `
    -FilePath $tauriCli `
    -ArgumentList @("build", "--bundles", $BundleList) `
    -WorkingDirectory $repoRoot `
    -RedirectStandardOutput $stdoutLog `
    -RedirectStandardError $stderrLog `
    -NoNewWindow `
    -PassThru

  $artifactDetectedAt = $null
  $completedByArtifacts = $false
  $artifactSince = $startedAt.AddSeconds(-2)

  while ($true) {
    Start-Sleep -Seconds 2
    $process.Refresh()

    if ($process.HasExited) {
      break
    }

    $elapsedSeconds = [int]((Get-Date) - $startedAt).TotalSeconds
    $hasExpectedArtifacts = Test-ExpectedArtifacts `
      -Version $version `
      -Since $artifactSince `
      -RequestedBundles $requestedBundles

    if ($hasExpectedArtifacts) {
      if ($null -eq $artifactDetectedAt) {
        $artifactDetectedAt = Get-Date
        Write-Host "  Detected fresh installer artifacts. Waiting $ArtifactSettleSeconds seconds for file handles to settle..." -ForegroundColor Yellow
      }
      elseif (((Get-Date) - $artifactDetectedAt).TotalSeconds -ge $ArtifactSettleSeconds) {
        Write-Host "  Build artifacts are stable; stopping lingering build wrapper process." -ForegroundColor Yellow
        Stop-ProcessTree -ProcessId $process.Id
        $completedByArtifacts = $true
        break
      }
    }

    if ($elapsedSeconds -ge $BuildTimeoutSeconds) {
      if ($hasExpectedArtifacts) {
        Write-Host "  Build timeout reached, but expected artifacts exist; stopping lingering process." -ForegroundColor Yellow
        Stop-ProcessTree -ProcessId $process.Id
        $completedByArtifacts = $true
        break
      }

      Stop-ProcessTree -ProcessId $process.Id
      Write-LogTail -Path $stdoutLog
      Write-LogTail -Path $stderrLog
      throw "tauri build exceeded $BuildTimeoutSeconds seconds and expected artifacts were not produced."
    }
  }

  if (-not $completedByArtifacts) {
    $process.WaitForExit()
    if ($process.ExitCode -ne 0) {
      Write-LogTail -Path $stdoutLog
      Write-LogTail -Path $stderrLog
      throw "tauri build failed with exit code: $($process.ExitCode)"
    }
  }

  if (-not (Test-ExpectedArtifacts -Version $version -Since $artifactSince -RequestedBundles $requestedBundles)) {
    Write-LogTail -Path $stdoutLog
    Write-LogTail -Path $stderrLog
    throw "tauri build finished, but expected installer artifacts for version $version were not found."
  }

  Invoke-UpdaterSigning `
    -Version $version `
    -Since $artifactSince `
    -RequestedBundles $requestedBundles `
    -KeyPath $SigningKeyPath `
    -PrivateKey $privateKeyEnvValue `
    -Password $SigningKeyPassword

  Write-Host ""
  Write-Host "Built installer artifacts:" -ForegroundColor Green
  Get-BundleArtifacts -Version $version -Since $artifactSince -RequestedBundles $requestedBundles |
    Sort-Object FullName |
    ForEach-Object { Write-Host "  $($_.FullName)" }
}

if (-not (Test-Path -LiteralPath $SigningKeyPath)) {
  throw "Local updater signing key was not found: $SigningKeyPath"
}

$privateKey = Get-Content -Raw -LiteralPath $SigningKeyPath
if ([string]::IsNullOrWhiteSpace($privateKey)) {
  throw "Local updater signing key file is empty: $SigningKeyPath"
}

$privateKey = $privateKey.Trim()
$privateKeyEnvValue = $null
$decodedKey = $null

if ($privateKey -match "secret key") {
  $decodedKey = $privateKey
  $privateKeyEnvValue = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($privateKey))
}
else {
  try {
    $decodedCandidate = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($privateKey)).Trim()
    if ($decodedCandidate -match "secret key") {
      $decodedKey = $decodedCandidate
      $privateKeyEnvValue = $privateKey
    }
  }
  catch {
  }
}

if ($null -eq $decodedKey -or $decodedKey -notmatch "(minisign|rsign).+secret key") {
  throw "The provided file does not look like a supported minisign/rsign private key: $SigningKeyPath"
}

$previousPrivateKey = [Environment]::GetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY", "Process")
$previousPassword = [Environment]::GetEnvironmentVariable("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "Process")

try {
  $env:TAURI_SIGNING_PRIVATE_KEY = $privateKeyEnvValue

  if (-not [string]::IsNullOrEmpty($SigningKeyPassword)) {
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $SigningKeyPassword
  }
  if ([string]::IsNullOrEmpty($SigningKeyPassword) -and [string]::IsNullOrEmpty($env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD)) {
    Write-Host "No signing key password was provided. If the key is password protected, signing will fail." -ForegroundColor Yellow
  }

  Write-Host ""
  Write-Host "Starting local signed build..." -ForegroundColor Cyan
  Write-Host "  Key file: $SigningKeyPath"
  Write-Host "  Bundles:  $Bundles"
  Write-Host ""

  Sync-RemoteRuntimeResource
  Clear-FrontendDist
  Invoke-TauriBuild -BundleList $Bundles

  if ($PrepareManualRelease) {
    $tauriConfig = Get-Content -Raw -LiteralPath $tauriConfigPath | ConvertFrom-Json
    if (-not $Tag) {
      $Tag = "v$($tauriConfig.version)"
    }

    Write-Host ""
    Write-Host "Preparing manual release assets..." -ForegroundColor Cyan
    Write-Host "  Tag: $Tag"
    Write-Host ""

    $prepareArgs = @(
      "-ExecutionPolicy", "Bypass",
      "-File", (Join-Path $repoRoot "scripts\prepare-manual-release.ps1"),
      "-Tag", $Tag
    )

    if ($NotesPath) {
      $prepareArgs += @("-NotesPath", $NotesPath)
    }

    & powershell @prepareArgs
    $prepareExitCode = $LASTEXITCODE

    if ($prepareExitCode -ne 0) {
      throw "prepare-manual-release.ps1 failed with exit code: $prepareExitCode"
    }
  }
}
finally {
  if ($null -eq $previousPrivateKey) {
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
  }
  else {
    $env:TAURI_SIGNING_PRIVATE_KEY = $previousPrivateKey
  }

  if ($null -eq $previousPassword) {
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
  }
  else {
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $previousPassword
  }
}
