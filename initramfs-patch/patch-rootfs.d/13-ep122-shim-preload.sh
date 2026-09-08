#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 13: EP122.service.d/10-qemu.conf - load ep122_shim.so via LD_PRELOAD
#
# Root cause: the original apl_start.sh does not contain the QEMU_PRELOAD line
# that patch 01 tried to replace (subucom_stub.so / djlink_shim.so never existed
# in this rootfs).  Without ep122_shim.so being preloaded into EP122, the USB bind
# intercept is inactive.
#
# Symptom: when EP122 processes the "mount /media/usb/sda ..." event from
# /proc/udev_usb1, it writes the USB interface name to:
#   /sys/bus/usb/drivers/usb-storage/bind
#   /sys/bus/usb/drivers/usb-storage/unbind
#   /sys/bus/usb/drivers/usb/unbind
# In QEMU -machine virt there is no xHCI/EHCI controller so these sysfs writes
# return ENODEV → EP122 shows "USB Error. Remove the device."
#
# Fix: install a systemd service drop-in that sets
#   Environment=LD_PRELOAD=/home/root/ep122_shim.so
# for EP122.service (which exec's apl_start.sh → EP122).  EP122 and all children
# inherit LD_PRELOAD; ep122_shim.so intercepts the bind writes → /dev/null.
#
# The same object carries the EP122 mods (guest/ep122_shim/cdj3k-mods): Gate
# Cue, MOD SETTINGS, Themes, STEMS, X-PAD.  They install themselves only inside
# an EP122 whose every symbol they can resolve, and every feature ships OFF in
# MOD SETTINGS.  The emulator's "EP122 Mods" menu toggle is the runtime gate:
# off, it puts `ep122_no_mods` on the kernel cmdline, the ExecStartPre below
# turns that into EP122_NO_MODS=1 in EP122's environment, and the mods'
# constructor returns before touching the process.  The shim's own plumbing
# (time-shift, jog, DRM, USB bind) is unaffected either way - dropping
# LD_PRELOAD could not separate the two.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

DROP_IN_DIR="$ROOTFS/etc/systemd/system/EP122.service.d"
mkdir -p "$DROP_IN_DIR"

cat > "$DROP_IN_DIR/10-qemu.conf" << 'SVCEOF'
[Service]
# Environment=EP122_TIME_SHIFT_DEBUG=1
# Environment=EP122_LINK_DEBUG=1
# Mods logging ships at ERROR: a deck in a booth says nothing while it works.
# Uncomment ONE of these to turn it up -- debug is every DJ action, trace adds
# the per-frame firehose.  Read it with `journalctl -u EP122`.  The STEMS
# sidecar has its own knob, STEMD_LOGLEVEL on stemd-client.service.
# Environment=EP122_MOD_LOGLEVEL=debug
# Environment=EP122_MOD_LOGLEVEL=trace

# Runtime mod gate.  /run is tmpfs, so the file is rebuilt on every boot from
# the kernel cmdline the emulator passed; the `-` makes a missing file a no-op.
ExecStartPre=/bin/sh -c 'if grep -qw ep122_no_mods /proc/cmdline; then echo EP122_NO_MODS=1 > /run/ep122-mods.env; else rm -f /run/ep122-mods.env; fi'
EnvironmentFile=-/run/ep122-mods.env

Environment=LD_PRELOAD=/home/root/ep122_shim.so
SVCEOF

chmod 644 "$DROP_IN_DIR/10-qemu.conf"
echo "  -> EP122.service.d/10-qemu.conf installed (LD_PRELOAD=ep122_shim.so, mods gate)"
