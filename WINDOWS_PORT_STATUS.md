# Windows Port Status

Baseline: upstream cdj3k-emu 0.1.2.
Target: Windows 10/11 x64 first, ARM64/WHPX later.

## Alpha 1 changes

1. Portable temp/runtime directory handling.
2. Windows `%LOCALAPPDATA%` storage root.
3. External QEMU process lifecycle on non-macOS hosts.
4. Windows process force-termination fallback via `taskkill`.
5. Windows TCP transport for `ctrl` and `cfg` virtio serial channels.
6. QEMU DirectSound audio backend selection on Windows.
7. macOS-only vmnet/TAP code isolated behind stubs on Windows.
8. Windows application bootstrap added.
9. GitHub Actions Windows compile/build workflow added.

## Next milestones

- Get green `cargo check --workspace` on `windows-latest` and fix compiler errors exposed by CI.
- Build patched QEMU for Windows and exclude macOS-only patches (`coreaudio`, HVF/QoS).
- Validate `display=shm` on Windows and main LCD mmap reader.
- Validate ivshmem jog LCD.
- Boot one CDJ firmware instance end-to-end.
- Replace DirectSound with WASAPI if latency/stability warrants it.
- Implement Windows USB media handling.
- Implement Pro DJ Link networking on Windows (likely TAP/WinPcap/Npcap or a dedicated L2 bridge helper).
- Package app + patched QEMU into a single portable zip/installer.
