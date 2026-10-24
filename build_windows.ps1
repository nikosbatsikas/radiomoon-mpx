# Builds the Windows setup file and the portable zip of RadioMoon MPX (transmitter + receiver).
#   .\build_windows.ps1                 -> x64
#   .\build_windows.ps1 -Arch x86       or      -Arch arm64
# Needs: Rust (https://rustup.rs, MSVC toolchain) and Inno Setup 6 (https://jrsoftware.org/isdl.php).
param([ValidateSet("x64", "x86", "arm64")] [string]$Arch = "x64")
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$targets = @{ x64 = "x86_64-pc-windows-msvc"; x86 = "i686-pc-windows-msvc"; arm64 = "aarch64-pc-windows-msvc" }
$target = $targets[$Arch]
rustup target add $target
cargo build --release --target $target
if ($LASTEXITCODE -ne 0) { throw "Build failed" }

$v = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
if (Test-Path "stage") { Remove-Item "stage" -Recurse -Force }
$stage = Join-Path $PSScriptRoot "stage\RadioMoon-MPX"
New-Item -ItemType Directory -Force $stage, "dist" | Out-Null
Copy-Item "target\$target\release\radiomoon-mpx-transmitter.exe" "$stage\RadioMoon-MPX-Transmitter.exe"
Copy-Item "target\$target\release\radiomoon-mpx-receiver.exe" "$stage\RadioMoon-MPX-Receiver.exe"
Copy-Item "README.md", "CHANGELOG.md" $stage

$iscc = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe", "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe") |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup 6 was not found. Install it from https://jrsoftware.org/isdl.php and run this script again." }
& $iscc "/DAppVersion=$v" "/DArch=$Arch" "/DSourceDir=$stage" "/DOutDir=$PSScriptRoot\dist" "installer\radiomoon-mpx.iss"
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed" }

# portable zip (the flag file makes the programs keep their settings next to themselves)
New-Item -ItemType File "$stage\portable.flag" | Out-Null
Compress-Archive -Path $stage -DestinationPath "dist\RadioMoon-MPX-$v-windows-$Arch-portable.zip" -Force

Write-Host ""
Write-Host "Done. Files in the dist folder:"
Get-ChildItem dist | ForEach-Object { Write-Host "  $($_.Name)" }
