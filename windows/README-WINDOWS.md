# CDJ3K Emulator - Windows Alpha 5

This is an experimental Windows x64 port of `cdj3k-emu`.

## Package layout

- `cdj3k-emu.exe` - Windows frontend
- `qemu/` - patched Windows QEMU ARM64 backend and required DLLs
- `qemu-img.exe` - helper used when creating/converting eMMC/USB images
- `resources/` - guest kernel, modules, patch files and provisioning tools
- `QEMU-SELFTEST.bat` - verifies the packaged QEMU executables can start

## First test without firmware

Double-click `QEMU-SELFTEST.bat`.

A successful package should print QEMU and qemu-img version information and:

`PASS: patched QEMU runtime can start.`

This self-test does not boot Pioneer firmware and does not require a key.

## Firmware

The application accepts a user-supplied CDJ-3000 `.UPD` and matching key for
local provisioning. Firmware and keys are not included in this project and
must not be committed to GitHub.

## Performance

Windows x64 must emulate the ARM64 CDJ guest with QEMU TCG. It will be much
slower than the Apple-Silicon/HVF build. Functional bring-up comes first;
performance tuning follows after display/control/audio paths are validated.
