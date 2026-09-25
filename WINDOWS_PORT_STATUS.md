# Windows port status - Alpha 5

Working from previous milestones:
- Windows Rust frontend compiles and launches.
- Firmware wizard opens on Windows.
- ARM64 Linux guest kernel/modules/tools build and package successfully.
- Windows firmware provisioning userland (MSYS2 bash/cpio/find/coreutils/etc.)
  is bundled and its CI verification passes.

Alpha 5 engineering target:
- Build and package the project's patched `qemu-system-aarch64.exe` for
  Windows x64.
- Preserve the custom main-LCD shared-memory display transport on Win32.
- Bundle `qemu-img.exe` and MinGW runtime DLLs.
- Reuse unchanged Linux guest resources instead of spending ~4 hours
  rebuilding them on every Windows-only change.

Alpha 5 does not claim a booting CDJ yet. The new QEMU CI job is deliberately
separate so Windows compiler/linker failures can be fixed before firmware boot
integration is attempted.

Still to validate after QEMU builds:
- QEMU starts from the packaged Windows app.
- Main LCD shared-memory stream.
- Jog LCD ivshmem transport.
- Virtio serial ctrl/cfg TCP channels.
- DirectSound / virtio-sound audio.
- Virtual USB media.
- Networking / Pro DJ Link.

Never commit Pioneer firmware or keys.
