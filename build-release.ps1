# Build Release Script for milo-pet
# Builds with static CRT linking for better portability

param(
    [string]$Version = "0.2.0"
)

Write-Host "Building milo-pet v$Version..." -ForegroundColor Cyan

# Clean previous build
Write-Host "Cleaning previous build..." -ForegroundColor Yellow
cargo clean

# Build with static CRT linking
Write-Host "Building with static CRT (RUSTFLAGS: -C target-feature=+crt-static)..." -ForegroundColor Yellow
$env:RUSTFLAGS = "-C target-feature=+crt-static"
cargo build --release

if ($LASTEXITCODE -ne 0) {
    Write-Host "Build failed!" -ForegroundColor Red
    exit 1
}

# Create release directory
$releaseDir = "release/milo-pet-v$Version"
Write-Host "Creating release package at $releaseDir..." -ForegroundColor Yellow

if (Test-Path $releaseDir) {
    Remove-Item $releaseDir -Recurse -Force
}
New-Item -ItemType Directory -Path $releaseDir -Force | Out-Null

# Copy executable
Copy-Item "target/release/milo-pet.exe" "$releaseDir/"

# Copy assets
Copy-Item "assets" "$releaseDir/" -Recurse

# Create README
$readme = @"
Milo Pet v$Version - Offline Desktop Pet for Windows
================================================

Requirements:
- Windows 10/11 (64-bit)
- No additional runtimes required (statically linked)

Installation:
1. Extract milo-pet-v$Version-windows.zip to any folder
2. Run milo-pet.exe

Features:
- Transparent layered window with pixel-perfect alpha compositing
- Frame-based animation (idle loop @ 24fps)
- Interactive physics: drag, throw with gravity & bounce
- Idle sleep animation: pet falls asleep after 30 seconds of inactivity
- Click to bounce, drag to pick up and throw
- Right-click menu: Return to home / Force sleep / Exit
- DPI-aware scaling (Per-Monitor V2)
- Completely offline - no network requests

Configuration:
Edit assets/config.toml to customize:
- width: Window width (logical pixels, auto-scaled by DPI)
- corner: Initial position (top-right / top-left / bottom-right / bottom-left)
- sleep_after: Idle timeout before sleep animation (seconds, 0 = disable)
- physics.gravity: Gravity strength (px/s²)
- physics.restitution: Bounce coefficient (0.0-1.0)
- physics.throw_power: Throw force multiplier

Controls:
- Left click + drag: Pick up and throw
- Left click (quick): Make pet bounce
- Right click: Context menu
  - Return to home position
  - Force sleep (wake by clicking)
  - Exit

Files:
- milo-pet.exe: Main application (statically linked, ~5MB)
- assets/frames.bin: Idle animation frame pack (67MB)
- assets/sleep.bin: Sleep animation frame pack (39MB)
- assets/config.toml: Configuration file

Build Information:
- Version: $Version
- Platform: Windows x86_64
- Linking: Static CRT (no VC++ Redistributable required)
- Rust version: 2021 edition
"@

Set-Content -Path "$releaseDir/README.txt" -Value $readme -Encoding UTF8

# Create ZIP archive
$zipPath = "release/milo-pet-v$Version-windows.zip"
Write-Host "Creating ZIP archive: $zipPath..." -ForegroundColor Yellow

if (Test-Path $zipPath) {
    Remove-Item $zipPath -Force
}

Compress-Archive -Path $releaseDir -DestinationPath $zipPath -Force

# Display results
Write-Host "`nBuild complete!" -ForegroundColor Green
Write-Host "Executable: $releaseDir/milo-pet.exe" -ForegroundColor Cyan
Write-Host "Archive: $zipPath" -ForegroundColor Cyan
Write-Host "Archive size: $([math]::Round((Get-Item $zipPath).Length / 1MB, 2)) MB" -ForegroundColor Cyan

# Verify static linking
Write-Host "`nVerifying static linking..." -ForegroundColor Yellow
$deps = ldd "$releaseDir/milo-pet.exe" 2>$null | Select-String -Pattern "vcruntime|msvcp"
if ($deps) {
    Write-Host "⚠ Warning: Still has VC++ dependencies:" -ForegroundColor Yellow
    $deps | ForEach-Object { Write-Host "  $_" -ForegroundColor Yellow }
} else {
    Write-Host "✓ No VC++ Redistributable dependencies detected" -ForegroundColor Green
}
