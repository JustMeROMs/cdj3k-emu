# Windows Port Status

## Completed

- Windows 10/11 x64 Rust frontend compiles in GitHub Actions.
- Windows application launches and opens the firmware wizard.
- Windows paths and process termination are implemented.
- Windows QEMU audio argument uses DirectSound for the initial port.
- Guest ARM64 kernel/modules/tools are now built by CI and staged into the Windows artifact.

## Current milestone

Make firmware provisioning self-contained on Windows:

1. decrypt user-supplied `.UPD` (already pure Rust)
2. extract firmware/kernel (already Rust)
3. extract embedded initramfs (already Rust)
4. replace Unix `bash`/`find`/`cpio` patching with a Windows-compatible implementation
5. create eMMC image using bundled `qemu-img.exe`

## Following milestone

Build/port the project's patched QEMU to Windows and connect:

- main LCD shared memory
- jog LCD shared memory
- control/config channels
- audio
- USB media
- networking / Pro DJ Link

Never commit Pioneer firmware or keys.
