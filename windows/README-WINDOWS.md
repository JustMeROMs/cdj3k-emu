# CDJ3K Emulator – Windows port (Alpha 3)

This branch is an experimental Windows port of cdj3k-emu.

## What Alpha 3 proves

- The Rust/egui Windows frontend builds and launches on Windows 10/11 x64.
- The firmware wizard opens on Windows.
- GitHub Actions now builds the public ARM64 Linux guest runtime resources used by the emulator and packages them under `resources/` next to `cdj3k-emu.exe`.

## What is still in progress

- The initramfs patch stage still depends on Unix shell/cpio behavior. Do not expect firmware provisioning to complete on Windows yet.
- A Windows build of the patched QEMU backend is not bundled yet.
- Main LCD / jog LCD shared-memory transports still need their Win32 QEMU implementation.
- Audio, USB media and Pro DJ Link will be enabled after QEMU boots reliably.

## Important

Do **not** commit or upload Pioneer firmware `.UPD` files or decryption keys to GitHub. Keep them on your own computer.

## Testing Alpha 3

Download the `cdj3k-emu-windows-alpha` artifact from the latest successful Windows Build workflow. Extract the whole ZIP to a normal folder before running `cdj3k-emu.exe`.

The package should now contain:

- `cdj3k-emu.exe`
- this README
- `WINDOWS_PORT_STATUS.md`
- `resources/Image`
- `resources/modules/*.ko`
- `resources/tools/*`
- `resources/patch/*`

The next development milestone is a Windows-native initramfs patch/provision path.
