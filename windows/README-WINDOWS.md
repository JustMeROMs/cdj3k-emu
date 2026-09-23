# cdj3k-emu Windows port — alpha 1

This branch is the first Windows x64 porting milestone of cdj3k-emu 0.1.2.

## What is implemented

- Windows-safe runtime/data paths.
- External `qemu-system-aarch64.exe` process launcher instead of the macOS embedded QEMU dylib.
- TCG selected on Windows x64.
- QMP remains TCP based.
- `ctrl` and `cfg` virtio-serial channels use localhost TCP on Windows instead of Unix-domain sockets.
- Windows QEMU audio argument uses DirectSound for the initial port.
- macOS vmnet/TAP implementation is isolated behind platform-specific modules.
- Windows CI workflow builds the Rust application.

## Runtime layout

Place QEMU either:

1. beside `cdj3k-emu.exe`, or
2. in `qemu\qemu-system-aarch64.exe`, or
3. set `CDJ3K_QEMU_EXE` to the full path.

Firmware files are not distributed with this project. Per-instance files are expected under the app data directory in `instance-N`:

- `Image`
- `initramfs-patched.cpio.gz`
- `emmc.qcow2`

## Important alpha limitation

The stock QEMU Windows binary is not enough for full CDJ emulation. The upstream project uses custom QEMU patches for the shared-memory display, jog LCD/shared memory, virtio-sound behavior and related integration. A Windows build of that patched QEMU is the next major milestone.

The Rust host-side port in this alpha is intended to compile and launch an external QEMU backend. Full boot/display/audio must be validated after the patched Windows QEMU backend is produced.
