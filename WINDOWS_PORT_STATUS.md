# Windows port status - Alpha 4

Working:
- Windows Rust frontend compiles in GitHub Actions.
- Windows application launches and opens the firmware wizard.
- ARM64 Linux guest kernel/modules/tools are built and packaged.
- Windows package contains its own firmware-provisioning Unix userland under
  `resources/msys2/usr/bin` (bash, cpio, find, coreutils, sed, grep, gzip, etc.).
- Firmware patcher explicitly invokes those bundled tools on Windows; WSL is not required.

Alpha 4 goal:
- User-supplied local .UPD + key can complete decrypt -> extract -> initramfs patch -> eMMC provision.

Still to do:
- Build/package the patched QEMU aarch64 Windows backend.
- Wire Windows display/jog transports and control channel to QEMU.
- Boot the provisioned CDJ firmware on Windows.
- Validate audio, USB media, networking / Pro DJ Link.

Never commit Pioneer firmware or keys.
