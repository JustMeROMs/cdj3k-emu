$ErrorActionPreference = "Stop"
Write-Host "Building cdj3k-emu Windows alpha..."
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
  throw "Rust/Cargo not found. Install Rust from https://rustup.rs then reopen PowerShell."
}
cargo build --release -p cdj3k-emu
New-Item -ItemType Directory -Force dist | Out-Null
Copy-Item target\release\cdj3k-emu.exe dist\cdj3k-emu.exe -Force
Copy-Item windows\README-WINDOWS.md dist\README-WINDOWS.md -Force
Write-Host "Built: dist\cdj3k-emu.exe"
