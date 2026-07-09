# day-installer.ps1 — install the `day` CLI on Windows.
#
# Rendered from scripts/release/templates/installer.ps1 by render-installers.py for release
# __DAY_VERSION__ (URLs and sha256 checksums baked in per release, in the style of cargo-dist's
# PowerShell installer). Usage:
#
#   powershell -ExecutionPolicy Bypass -c "irm __DAY_INSTALLER_BASE__/day-installer.ps1 | iex"
#
# Options (environment):
#   DAY_INSTALL_DIR   install directory (default: $env:CARGO_HOME\bin, ~\.cargo\bin, or
#                     %LOCALAPPDATA%\day\bin — the chosen directory is added to the user PATH)

$ErrorActionPreference = 'Stop'

$AppVersion = '__DAY_VERSION__'
$BaseUrl = '__DAY_BASE_URL__'

# --- platform detection ------------------------------------------------------
$arch = $env:PROCESSOR_ARCHITECTURE
switch ($arch) {
  'AMD64' { $triple = 'x86_64-pc-windows-msvc'; $sha256 = '__SHA256_x86_64_pc_windows_msvc__' }
  'ARM64' { $triple = 'aarch64-pc-windows-msvc'; $sha256 = '__SHA256_aarch64_pc_windows_msvc__' }
  default { throw "day-installer: unsupported architecture: $arch" }
}
$artifact = "day-$triple.zip"
$url = "$BaseUrl/$artifact"

# --- download + verify ---------------------------------------------------------
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("day-install-" + [System.Guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
  Write-Host "downloading day $AppVersion ($triple)"
  Write-Host "  from $url"
  $zip = Join-Path $tmp $artifact
  Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing

  $actual = (Get-FileHash -Algorithm SHA256 -Path $zip).Hash.ToLowerInvariant()
  if ($actual -ne $sha256) {
    throw "day-installer: checksum mismatch for $artifact`n  expected: $sha256`n  actual:   $actual"
  }
  Write-Host "verified sha256:$sha256"

  Expand-Archive -Path $zip -DestinationPath $tmp -Force
  $exe = Join-Path $tmp 'day.exe'
  if (-not (Test-Path $exe)) { throw 'day-installer: archive did not contain day.exe' }

  # --- install -------------------------------------------------------------------
  if ($env:DAY_INSTALL_DIR) {
    $dest = $env:DAY_INSTALL_DIR
  } elseif ($env:CARGO_HOME -and (Test-Path (Join-Path $env:CARGO_HOME 'bin'))) {
    $dest = Join-Path $env:CARGO_HOME 'bin'
  } elseif (Test-Path (Join-Path $HOME '.cargo\bin')) {
    $dest = Join-Path $HOME '.cargo\bin'
  } else {
    $dest = Join-Path $env:LOCALAPPDATA 'day\bin'
  }
  New-Item -ItemType Directory -Path $dest -Force | Out-Null
  Copy-Item -Path $exe -Destination (Join-Path $dest 'day.exe') -Force
  Write-Host "installed $dest\day.exe"

  # Add to the USER PATH if missing (takes effect in new terminals).
  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  if (-not (($userPath -split ';') -contains $dest)) {
    [Environment]::SetEnvironmentVariable('Path', "$userPath;$dest", 'User')
    Write-Host "added $dest to your user PATH (open a new terminal to pick it up)"
  }

  Write-Host ''
  Write-Host "run 'day --version' to verify, and 'day doctor' to check platform toolchains."
} finally {
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
