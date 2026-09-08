#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Patch 30: stemd-client.service - the network half of the STEMS feature.
#
# stemd_client is a standalone daemon and is deliberately not coupled to EP122:
# it owns the HTTP client, the resolver and every retry loop, so none of that
# runs inside the process that owns the deck's audio thread. If it wedges or
# dies, EP122 keeps playing and STEMS simply reports unavailable.
#
# That independence is what lets this be a plain unit. What it actually needs:
#
#   /run                 binds /run/stemd-client.sock; tmpfs, wiped on boot, and
#                        it unlinks a stale node itself before binding
#   /dev/shm             where downloaded stems are written for the shim to
#                        adopt (the shim unlinks each one once it has an fd)
#   network              to reach the stemd server
#   avahi-browse         ONLY for AUTO discovery. After= and not Requires=, so a
#                        deck with avahi missing still starts the daemon
#
# No arguments and no config file; the only environment it reads is
# STEMD_LOGLEVEL (see the unit below). It does not fork, logs to stderr for
# journald, ignores SIGPIPE so a client vanishing mid-upload is an EPIPE to
# handle rather than a death, and unlinks its socket on the way out.
#
# Ordering against EP122 does not matter: the shim retries the connection on an
# interval, so whichever starts first, they find each other.
#
# The binary (/usr/bin/stemd_client) is placed by the firmware provisioner from
# the bundle's Contents/Resources/tools/, before this patch runs.
set -euo pipefail
: "${ROOTFS:?ROOTFS must be set by dispatcher}"

SERVICE_DIR="$ROOTFS/etc/systemd/system"
MULTI_USER_WANTS="$SERVICE_DIR/multi-user.target.wants"
mkdir -p "$MULTI_USER_WANTS"

cat > "$SERVICE_DIR/stemd-client.service" << 'SVCEOF'
[Unit]
Description=stemd client (STEMS separation sidecar for EP122)
After=network.target
After=avahi-daemon.service
StartLimitIntervalSec=0

[Service]
Type=simple
ExecStart=/usr/bin/stemd_client
Restart=always
RestartSec=2s
# A separation job holds a job id on the server; SIGTERM lets the daemon close
# the session rather than leaving one parked until the server reaps it.
KillSignal=SIGTERM
TimeoutStopSec=5s
# Logging ships at ERROR: the deck re-HELLOs every 30 s and the steady-state
# answer is not worth a journal line. Uncomment one to turn it up; debug adds
# the per-refresh discovery detail and every job transition.
# Environment=STEMD_LOGLEVEL=info
# Environment=STEMD_LOGLEVEL=debug

[Install]
WantedBy=multi-user.target
SVCEOF

chmod 644 "$SERVICE_DIR/stemd-client.service"
ln -sf /etc/systemd/system/stemd-client.service \
    "$MULTI_USER_WANTS/stemd-client.service"

echo "  [30] stemd-client.service installed"
