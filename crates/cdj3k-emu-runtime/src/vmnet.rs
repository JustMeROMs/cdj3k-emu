//! socket_vmnet integration for bridged Pro DJ Link networking.
//!
//! socket_vmnet (github.com/lima-vm/socket_vmnet) runs as root and exposes a
//! vmnet interface over a Unix socket.  QEMU connects with:
//!   -netdev stream,id=net0,server=off,addr.type=unix,addr.path=<sock>
//!
//! Elevation uses the macOS Security framework (AuthorizationCreate +
//! AuthorizationCopyRights + AuthorizationExecuteWithPrivileges), which shows
//! the native admin dialog with TouchID / Apple Watch support.

use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const BREW_CANDIDATES: &[&str] = &[
    "/opt/homebrew/bin/socket_vmnet",
    "/usr/local/bin/socket_vmnet",
    "/opt/homebrew/opt/socket_vmnet/bin/socket_vmnet",
];

/// Fixed vmnet network identifier for host-only mode.
///
/// Passing `--vmnet-network-identifier=<UUID>` maps to vmnet.framework's
/// `vmnet_network_identifier_key` (macOS 11+): every interface that shares this
/// UUID joins one common isolated vmnet network *with no DHCP server*.  Same
/// UUID -> same L2 segment, so all instances still see each other's broadcast /
/// unicast (this is not per-guest isolation) - it only drops the built-in
/// `bootpd` lease service.
///
/// With no DHCP, the guest's `link-monitor.sh` `udhcpc -T 2` times out and
/// falls through to `avahi-autoipd`, self-assigning a 169.254/16 link-local
/// address - matching real Pro DJ Link gear on a router-less network, which is
/// what we want to test.  A single constant keeps every instance on the same
/// segment across restarts.
const VMNET_HOST_NETWORK_ID: &str = "7b3d5e2a-9c14-4f6b-b2e1-0a1c2d3e4f50";

/// One QEMU instance's hold on a shared socket_vmnet daemon.
///
/// The daemon is shared by every instance on the same interface, so no single
/// instance owns it.  Each handle holds a **lease**: a file named after its
/// PID under `<sock>.clients/` (see [`lease_dir`]).  The root-side watchdog
/// spawned alongside socket_vmnet (see `launch_elevated`) keeps the daemon
/// alive while any lease names a live PID, and reaps it when either:
///   - no lease is live any more (every instance exited, crashed or was
///     SIGKILLed - a dead PID's lease is swept by the watchdog itself), or
///   - the socket file is unlinked (the process-exit cleanup, when it finds
///     itself the last client).
///
/// We can't kill the daemon directly because it runs as root and the host
/// app runs as the user, but the user *can* unlink the socket (parent dir
/// is user-owned), which the watchdog uses as a shutdown signal.  The
/// lease is written BEFORE the socket is probed or the daemon launched, so
/// the watchdog never observes a window with a live daemon and no client.
///
/// PID reuse: a crashed instance leaves a lease behind until the watchdog's
/// next `kill -0` sees the PID dead; if another process takes that PID first,
/// the daemon lingers until it exits.  Clean exits remove their lease.
pub struct SocketVmnet {
    socket_path: PathBuf,
    /// This instance's lease file.
    lease: PathBuf,
}

/// `<sock>.clients/` - one file per attached instance, named by PID.  Lives
/// next to the socket in the user-owned DJPL net dir so a djx-emu instance
/// attaching to the same daemon can hold a lease the same way.
fn lease_dir(socket_path: &Path) -> PathBuf {
    let mut name = socket_path.as_os_str().to_owned();
    name.push(".clients");
    PathBuf::from(name)
}

/// True if any lease other than `own` names a live process.  Dead PIDs' leases
/// are removed on the way (same sweep the root watchdog does).
pub(crate) fn other_clients_alive(dir: &Path, own: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let mut alive = false;
    for entry in entries.flatten() {
        let path = entry.path();
        if path == own {
            continue;
        }
        let pid = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.parse::<libc::pid_t>().ok());
        // SAFETY: signal 0 performs only the existence/permission check.
        // EPERM means the PID exists but belongs to another user - alive.
        let exists = pid.is_some_and(|pid| {
            let rc = unsafe { libc::kill(pid, 0) };
            rc == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
        });
        if exists {
            alive = true;
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
    alive
}

/// Validate an interface name before letting it reach a root-elevated shell
/// template.  `sh_quote` already neutralises injection, but rejecting names
/// that don't match the BSD ifname grammar is a cheap defence-in-depth check
/// against ever feeding garbage to the elevated watcher.
pub(crate) fn is_valid_iface(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 16
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl SocketVmnet {
    /// Start socket_vmnet in bridged mode on `iface`, or reuse an already-running
    /// daemon for that interface.  The socket is shared across all instances.
    ///
    /// Shows the native macOS admin dialog only when a new daemon must be started.
    pub fn start_bridged(iface: &str) -> io::Result<Self> {
        if !is_valid_iface(iface) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid interface name: {iface:?}"),
            ));
        }
        Self::acquire(socket_path_for(iface), Some(iface))
    }

    /// Start socket_vmnet in host-only mode, or reuse an already-running daemon.
    /// Apple's vmnet.framework creates a host-side bridge interface (e.g.
    /// `bridge100`); all QEMU instances connecting to this socket share the same
    /// L2 fabric and see each other via standard broadcast / unicast, with no
    /// physical interface bridged - that shared, NIC-less segment is the reason
    /// this mode exists.
    ///
    /// We pass `--vmnet-network-identifier` (see [`VMNET_HOST_NETWORK_ID`]) so
    /// the segment carries **no DHCP server**: guests fall through to
    /// avahi-autoipd link-local (169.254/16), matching real Pro DJ Link gear.
    pub fn start_host() -> io::Result<Self> {
        Self::acquire(socket_path_for("host"), None)
    }

    /// Take a lease on `socket_path`, then attach to the daemon behind it or
    /// launch one.  `iface` is `None` for host-only mode.
    ///
    /// The lease goes down first: if the daemon is already live its watchdog
    /// counts us from this moment, and if we launch it ourselves the watchdog
    /// starts with one live client.  Shows the native macOS admin dialog only
    /// when a new daemon must be started.
    fn acquire(socket_path: PathBuf, iface: Option<&str>) -> io::Result<Self> {
        cdj3k_emu_platform::runtime_paths::ensure_djpl_net_dir()?;
        let leases = lease_dir(&socket_path);
        std::fs::create_dir_all(&leases)?;
        let lease = leases.join(std::process::id().to_string());
        std::fs::write(&lease, format!("{}\n", std::process::id()))?;
        let this = Self { socket_path, lease };

        // If the daemon is already live, reuse it - no elevation, no restart.
        if socket_accepts(&this.socket_path) {
            return Ok(this);
        }

        // Stale socket file without a live daemon behind it.
        if this.socket_path.exists() {
            let _ = std::fs::remove_file(&this.socket_path);
        }

        let bin = find_binary()?;
        // A failed launch drops `this`, which releases the lease.
        launch_elevated(&bin, iface, &this.socket_path, &leases)?;
        Ok(this)
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Release this instance's lease.  Nothing else: if it was the last one
    /// the root-side watchdog reaps socket_vmnet on its next poll, and the
    /// process-exit handler (`instance::cleanup_runtime_files`) is the one
    /// place that removes sockets and directories.  Safe to call repeatedly.
    pub fn stop(&self) {
        let _ = std::fs::remove_file(&self.lease);
    }
}

impl Drop for SocketVmnet {
    fn drop(&mut self) {
        self.stop();
    }
}

// ── Security.framework FFI ────────────────────────────────────────────────────

type AuthorizationRef = *mut libc::c_void;
type OSStatus = i32;
type AuthorizationFlags = u32;

const K_AUTH_FLAG_DEFAULTS: AuthorizationFlags = 0;
const K_AUTH_FLAG_INTERACTION_ALLOWED: AuthorizationFlags = 1 << 0;
const K_AUTH_FLAG_EXTEND_RIGHTS: AuthorizationFlags = 1 << 1;

#[repr(C)]
struct AuthorizationItem {
    name: *const libc::c_char,
    value_length: libc::size_t,
    value: *mut libc::c_void,
    flags: u32,
}

#[repr(C)]
struct AuthorizationItemSet {
    count: u32,
    items: *mut AuthorizationItem,
}

#[link(name = "Security", kind = "framework")]
extern "C" {
    fn AuthorizationCreate(
        rights: *const AuthorizationItemSet,
        environment: *const AuthorizationItemSet,
        flags: AuthorizationFlags,
        authorization: *mut AuthorizationRef,
    ) -> OSStatus;

    fn AuthorizationCopyRights(
        authorization: AuthorizationRef,
        rights: *const AuthorizationItemSet,
        environment: *const AuthorizationItemSet,
        flags: AuthorizationFlags,
        authorized_rights: *mut *mut AuthorizationItemSet,
    ) -> OSStatus;

    fn AuthorizationExecuteWithPrivileges(
        authorization: AuthorizationRef,
        path_to_tool: *const libc::c_char,
        options: AuthorizationFlags,
        arguments: *const *const libc::c_char,
        communication_pipe: *mut *mut libc::FILE,
    ) -> OSStatus;

    fn AuthorizationFree(authorization: AuthorizationRef, flags: AuthorizationFlags) -> OSStatus;
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn socket_path_for(iface: &str) -> PathBuf {
    cdj3k_emu_platform::runtime_paths::vmnet_sock(iface)
}

fn find_binary() -> io::Result<String> {
    if let Ok(exe) = std::env::current_exe() {
        let bundled = exe.parent().unwrap_or(Path::new(".")).join("socket_vmnet");
        if bundled.exists() {
            return Ok(bundled.to_string_lossy().into_owned());
        }
    }
    for c in BREW_CANDIDATES {
        if Path::new(c).exists() {
            return Ok(c.to_string());
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "socket_vmnet not found - run bundle.sh or install via Homebrew",
    ))
}

fn launch_elevated(
    bin: &str,
    iface: Option<&str>,
    socket_path: &Path,
    leases: &Path,
) -> io::Result<()> {
    // The elevated shell does two things, both backgrounded:
    //   1. Spawn socket_vmnet itself.
    //   2. Spawn a watchdog subshell that polls the lease files under
    //      `leases` (one per attached instance, named by PID - see
    //      `SocketVmnet`) and the socket file presence; when no lease names
    //      a live PID, or the socket is gone, it kills the daemon and unlinks
    //      the socket.  A lease whose PID is dead is swept on the spot, so a
    //      crashed instance never pins the daemon.  The watchdog inherits
    //      root from this elevated shell, so it actually has permission to
    //      SIGTERM the daemon - something the user-level host process never
    //      could.
    //
    // The watchdog also outlives this shell: `( ... ) &` forks a subshell,
    // and AuthorizationExecuteWithPrivileges spawns us without a controlling
    // tty, so there's no SIGHUP source.  `trap '' HUP` belt-and-suspenders.
    //
    // `iface = Some(name)` selects bridged mode on that physical interface;
    // `iface = None` selects host-only mode (Apple creates a fresh `bridge*`
    // for the isolated network), with `--vmnet-network-identifier` to suppress
    // the built-in DHCP server (see `VMNET_HOST_NETWORK_ID`).
    //
    // vmnet.framework ignores `--vmnet-gateway`/`--vmnet-mask` once a network
    // identifier is set (it picks its own random 192.168.x.1/24 for the host
    // bridge), so we can't ask socket_vmnet to put the host on link-local.
    // Instead, host-only injects `addr_block`: still inside this one elevated
    // shell (no extra password prompt), it waits for the bridge to appear,
    // deletes vmnet's random primary, and hands the interface to macOS's
    // IPConfiguration agent in DHCP mode.  With no DHCP server on the segment
    // (the network identifier suppresses vmnet's bootpd), IPConfiguration falls
    // through to its IPv4LL "self-assigned IP" path and picks a 169.254.x - the
    // host analog of the guest's `avahi-autoipd`.  So the host *decides* its own
    // link-local address (nothing attributed), IPConfiguration owns it as the
    // interface's single address, and host + guests share one non-DHCP
    // link-local /16.  BR_BEFORE is captured before launch so we can identify
    // *our* freshly-created bridge.
    //
    // Trade-off: IPConfiguration waits out its DHCP timeout (~16 s observed)
    // before self-assigning, so the host is not reachable at L3 for that window.
    // It runs in the backgrounded watchdog subshell, so it never blocks startup,
    // and L2 sniffing (Wireshark) needs no address and is unaffected.
    let bin_q = sh_quote(bin);
    let (mode_args, addr_block) = match iface {
        Some(name) => (
            format!("--vmnet-mode bridged --vmnet-interface {}", sh_quote(name)),
            String::new(),
        ),
        None => (
            format!("--vmnet-mode host --vmnet-network-identifier {VMNET_HOST_NETWORK_ID}"),
            r#"  br=""
  j=0
  while [ $j -lt 40 ]; do
    for b in $(ifconfig -l | tr ' ' '\n' | grep '^bridge'); do
      case " $BR_BEFORE " in
        *" $b "*) : ;;
        *) br="$b"; break ;;
      esac
    done
    [ -n "$br" ] && break
    sleep 0.25; j=$((j+1))
  done
  if [ -n "$br" ]; then
    for a in $(ifconfig "$br" | awk '/inet /{print $2}'); do
      ifconfig "$br" inet "$a" delete 2>/dev/null
    done
    ipconfig set "$br" DHCP 2>/dev/null
  fi
"#
            .to_string(),
        ),
    };
    let sock_q = sh_quote(&socket_path.to_string_lossy());
    let leases_q = sh_quote(&leases.to_string_lossy());
    let cmd = format!(
        r#"BR_BEFORE=$(ifconfig -l | tr ' ' '\n' | grep '^bridge' | tr '\n' ' ')
nohup {bin} {mode_args} {sock} >/dev/null 2>&1 &
SV_PID=$!
( trap '' HUP
  i=0
  while [ $i -lt 40 ] && [ ! -S {sock} ]; do sleep 0.1; i=$((i+1)); done
{addr_block}  while [ -S {sock} ]; do
    live=0
    for l in {leases}/*; do
      [ -e "$l" ] || continue
      if kill -0 "$(basename "$l")" 2>/dev/null; then live=1; else rm -f "$l"; fi
    done
    [ "$live" -eq 1 ] || break
    sleep 1
  done
  [ -n "$br" ] && ipconfig set "$br" NONE 2>/dev/null
  kill "$SV_PID" 2>/dev/null
  sleep 0.3
  kill -9 "$SV_PID" 2>/dev/null
  rm -f {sock}
) </dev/null >/dev/null 2>&1 &
"#,
        bin = bin_q,
        mode_args = mode_args,
        addr_block = addr_block,
        sock = sock_q,
        leases = leases_q,
    );
    run_elevated(&cmd)?;

    // Poll until socket_vmnet is actually accepting connections (not just that the
    // socket file exists - a crashed process leaves the file behind).
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "socket_vmnet did not start within 5 s",
            ));
        }
        thread::sleep(Duration::from_millis(100));
        if socket_accepts(socket_path) {
            break;
        }
    }

    Ok(())
}

/// Try to connect to a Unix socket and immediately close.  Returns true if
/// something is actually listening - distinguishes a live daemon from a stale
/// socket file left behind by a crashed process.
fn socket_accepts(path: &std::path::Path) -> bool {
    use std::os::unix::net::UnixStream;
    UnixStream::connect(path).is_ok()
}

/// Run an arbitrary shell command as root via macOS Authorization Services.
/// Shows the native admin dialog (TouchID / Apple Watch eligible).
/// Shared by `vmnet` and `tapbridge`.
pub fn run_elevated(sh_cmd: &str) -> io::Result<()> {
    use std::ffi::CString;
    use std::ptr;

    let sh_path = CString::new("/bin/sh").unwrap();
    let sh_flag = CString::new("-c").unwrap();
    let cmd = CString::new(sh_cmd).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "shell command contains null byte",
        )
    })?;
    let right_name = CString::new("system.privilege.admin").unwrap();

    unsafe {
        let mut auth: AuthorizationRef = ptr::null_mut();
        let st = AuthorizationCreate(ptr::null(), ptr::null(), K_AUTH_FLAG_DEFAULTS, &mut auth);
        if st != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("AuthorizationCreate failed: {st}"),
            ));
        }

        let mut right_item = AuthorizationItem {
            name: right_name.as_ptr(),
            value_length: 0,
            value: ptr::null_mut(),
            flags: 0,
        };
        let rights = AuthorizationItemSet {
            count: 1,
            items: &mut right_item as *mut _,
        };
        let st = AuthorizationCopyRights(
            auth,
            &rights,
            ptr::null(),
            K_AUTH_FLAG_INTERACTION_ALLOWED | K_AUTH_FLAG_EXTEND_RIGHTS,
            ptr::null_mut(),
        );
        if st != 0 {
            AuthorizationFree(auth, K_AUTH_FLAG_DEFAULTS);
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "admin elevation cancelled or failed",
            ));
        }

        let args: [*const libc::c_char; 3] = [sh_flag.as_ptr(), cmd.as_ptr(), ptr::null()];
        let st = AuthorizationExecuteWithPrivileges(
            auth,
            sh_path.as_ptr(),
            K_AUTH_FLAG_DEFAULTS,
            args.as_ptr(),
            ptr::null_mut(),
        );
        AuthorizationFree(auth, K_AUTH_FLAG_DEFAULTS);

        if st != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("AuthorizationExecuteWithPrivileges failed: {st}"),
            ));
        }
    }
    Ok(())
}

/// Wrap a string in single quotes for /bin/sh, escaping any embedded `'`.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_dir_sits_next_to_the_socket() {
        let d = lease_dir(Path::new("/tmp/djpl-501/vmnet-host.sock"));
        assert_eq!(d, PathBuf::from("/tmp/djpl-501/vmnet-host.sock.clients"));
    }

    /// A live PID's lease counts; a dead PID's lease is swept; our own lease
    /// never counts as "other".
    #[test]
    fn other_clients_alive_counts_live_pids_and_sweeps_dead_ones() {
        let dir = std::env::temp_dir().join(format!("cdj3k-lease-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let own = dir.join(std::process::id().to_string());
        std::fs::write(&own, "").unwrap();

        // Only our own lease: nobody else.
        assert!(!other_clients_alive(&dir, &own));

        // A lease for a PID that cannot exist (pid_t max on macOS is 99998).
        let dead = dir.join("999999");
        std::fs::write(&dead, "").unwrap();
        assert!(!other_clients_alive(&dir, &own));
        assert!(!dead.exists(), "dead lease must be swept");

        // A lease for a live process (pid 1, launchd: alive, and owned by
        // root, so kill(1, 0) answers EPERM rather than 0).
        let live = dir.join("1");
        std::fs::write(&live, "").unwrap();
        assert!(other_clients_alive(&dir, &own));
        assert!(live.exists(), "live lease must be kept");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
