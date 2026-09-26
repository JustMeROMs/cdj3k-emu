//! MainLcdStream - reads the main LCD from the QEMU shm display backend.
//!
//! Shm file layout (written by qemu/patch/shm-display.c):
//!
//!   [0]   u32  magic       0x514D5348  ("QMS\x00")
//!   [4]   u32  generation  incremented with RELEASE after every dirty blit
//!   [8]   u32  width
//!   [12]  u32  height
//!   [16]  u32  stride      bytes per row
//!   [20]  u32  format      1 = RGBA8888  (QEMU converts from XRGB on its side)
//!   [24]  u32  dirty_x
//!   [28]  u32  dirty_y
//!   [32]  u32  dirty_w
//!   [36]  u32  dirty_h
//!   [64]  u8[] pixels      stride × height bytes
//!
//! The reader polls `generation` with Acquire semantics; when it changes,
//! dirty_x/y/w/h and pixel data are coherent.
//!
//! Pixel format: format=1 (RGBA8888, R,G,B,A byte order).
//! shm_gfx_update converts XRGB8888→RGBA8888 on the QEMU side so the host
//! can bulk-copy rows without any per-pixel channel swap.
//!
//! File path: `{socket_dir}/main.shm`  (created by QEMU at boot).

use memmap2::Mmap;
use std::fs::File;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub const LCD_W: usize = 1280;
pub const LCD_H: usize = 720;

const SHM_MAGIC: u32 = 0x514D_5348;
/// Byte offset where pixel data begins in the shm file (public for the GL upload path).
pub const SHM_PIXELS_OFFSET: usize = 64;

/// Poll interval for the shm generation counter. 500 µs comfortably tracks QEMU's
/// ~60 Hz dirty publishes without burning CPU.
const POLL_INTERVAL: Duration = Duration::from_micros(500);
/// Backoff between "shm not yet present" / "magic gone" retries.
const RECONNECT_DELAY: Duration = Duration::from_secs(1);

/// A dirty-region notification - carries the mmap reference so the UI thread can
/// upload directly to the GPU without any intermediate pixel copy.
pub struct DisplayDirty {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Row stride in bytes (= shm header `stride` field).
    pub stride: u32,
    /// Shared reference to the shm mapping; pixels live at
    /// `mmap[SHM_PIXELS_OFFSET + y*stride + x*4 ..]`.
    pub mmap: Arc<Mmap>,
}

/// Background-thread shm reader for the main LCD.
pub struct MainLcdStream {
    slot: Arc<Mutex<Option<DisplayDirty>>>,
    connected: Arc<AtomicBool>,
    /// Monotonic count of generations observed since process start. Lets the UI
    /// gate its "still booting" overlay on actual frame production rather than
    /// just the shm being mapped.
    frames_seen: Arc<AtomicU32>,
    shm_path: String,
}

impl MainLcdStream {
    pub fn new(socket_dir: &str, gate: crate::RepaintGate) -> Self {
        let shm_path = format!("{}/main.shm", socket_dir.trim_end_matches('/'));
        let slot: Arc<Mutex<Option<DisplayDirty>>> = Arc::new(Mutex::new(None));
        let slot_clone = Arc::clone(&slot);
        let connected = Arc::new(AtomicBool::new(false));
        let connected_clone = Arc::clone(&connected);
        let frames_seen = Arc::new(AtomicU32::new(0));
        let frames_seen_clone = Arc::clone(&frames_seen);
        let path = shm_path.clone();

        thread::Builder::new()
            .name("main-lcd-shm".into())
            .spawn(move || shm_loop(&path, slot_clone, connected_clone, frames_seen_clone, gate))
            .expect("spawn main-lcd-shm thread");

        Self {
            slot,
            connected,
            frames_seen,
            shm_path,
        }
    }

    /// Total generations observed by the background reader since process start.
    pub fn frames_seen(&self) -> u32 {
        self.frames_seen.load(Ordering::Relaxed)
    }

    /// Take the latest dirty notification, if any.
    pub fn take(&self) -> Option<DisplayDirty> {
        self.slot.lock().ok()?.take()
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub fn addr_str(&self) -> &str {
        &self.shm_path
    }
}

// ---------------------------------------------------------------------------
// Background thread
// ---------------------------------------------------------------------------

fn shm_loop(
    shm_path: &str,
    slot: Arc<Mutex<Option<DisplayDirty>>>,
    connected: Arc<AtomicBool>,
    frames_seen: Arc<AtomicU32>,
    gate: crate::RepaintGate,
) {
    let mut wait_logged = false;
    loop {
        // Wait for the shm file to appear and contain a valid header.
        let mmap = loop {
            match open_shm(shm_path) {
                Some(m) => {
                    eprintln!("[main_stream] opened {shm_path}");
                    wait_logged = false;
                    connected.store(true, Ordering::Relaxed);
                    gate.request();
                    break m;
                }
                None => {
                    // Missing main.shm is normal while no firmware is provisioned
                    // or QEMU is stopped. Avoid console noise; connection state is
                    // surfaced in the diagnostics UI instead.
                    wait_logged = true;
                    thread::sleep(RECONNECT_DELAY);
                }
            }
        };

        poll_loop(&mmap, &slot, &frames_seen, &gate);

        // QEMU restarted (magic gone).
        eprintln!("[main_stream] disconnected, reconnecting");
        connected.store(false, Ordering::Relaxed);
        gate.request();
        thread::sleep(RECONNECT_DELAY);
    }
}

/// Inner loop: poll generation until magic disappears (QEMU gone/restarted).
/// A static display keeps the same generation indefinitely - that is normal,
/// not stale - so there is no timeout-based exit.
fn poll_loop(
    mmap: &Arc<Mmap>,
    slot: &Arc<Mutex<Option<DisplayDirty>>>,
    frames_seen: &Arc<AtomicU32>,
    gate: &crate::RepaintGate,
) {
    let mut last_gen: u32 = read_u32(mmap, 4);

    // Local dirty rect accumulator (x0, y0, x1, y1).
    // Accumulates the union of all dirty rects received since the last
    // successful publish.  Prevents cursor-ghost artifacts when the UI
    // thread is busy and cdj3k-emu misses intermediate dirty rect updates
    // (e.g. "erase old cursor" fires between two polls - without
    // accumulation the stale cursor pixels would never be uploaded).
    let mut acc: Option<(usize, usize, usize, usize)> = None;
    // Track surface dimensions to detect switches (640×480 → 1280×720).
    let mut last_w: usize = 0;
    let mut last_h: usize = 0;

    loop {
        thread::sleep(POLL_INTERVAL);

        // Magic check on every tick - disappears when QEMU exits or restarts.
        if read_u32(mmap, 0) != SHM_MAGIC {
            eprintln!("[main_stream] magic gone, reconnecting");
            return;
        }

        // Acquire load of generation - pairs with QEMU's RELEASE add.
        let gen = read_u32_acquire(mmap, 4);

        if gen == last_gen {
            continue;
        }
        puffin::profile_scope!("main_lcd_gen_bump");
        last_gen = gen;
        frames_seen.fetch_add(1, Ordering::Relaxed);

        // Re-read dimensions on every frame - the surface can switch
        // mid-session (e.g. initial 640×480 QEMU console → 1280×720 Xorg).
        let width = read_u32(mmap, 8) as usize;
        let height = read_u32(mmap, 12) as usize;
        let stride = read_u32(mmap, 16) as usize;

        if width == 0
            || height == 0
            || stride < width * 4
            || mmap.len() < SHM_PIXELS_OFFSET + stride * height
        {
            continue;
        }

        // Reset accumulator on surface dimension change.
        if width != last_w || height != last_h {
            acc = None;
            last_w = width;
            last_h = height;
        }

        // Read and validate dirty rect from header.
        let dx = read_u32(mmap, 24) as usize;
        let dy = read_u32(mmap, 28) as usize;
        let dw = read_u32(mmap, 32) as usize;
        let dh = read_u32(mmap, 36) as usize;

        if dw == 0 || dh == 0 || dx + dw > width || dy + dh > height {
            continue;
        }

        // Expand the local accumulator to cover this dirty rect.
        acc = Some(match acc {
            None => (dx, dy, dx + dw, dy + dh),
            Some((ax0, ay0, ax1, ay1)) => {
                (ax0.min(dx), ay0.min(dy), ax1.max(dx + dw), ay1.max(dy + dh))
            }
        });

        // Try to publish the accumulated region.  If the slot is still
        // occupied the accumulator keeps growing - next tick will cover
        // everything that was missed.
        if let Ok(mut g) = slot.try_lock() {
            if g.is_none() {
                if let Some((x0, y0, x1, y1)) = acc.take() {
                    let uw = x1 - x0;
                    let uh = y1 - y0;

                    // Zero-copy: share the mmap reference so the UI thread
                    // uploads directly from the shm file into the GPU texture.
                    *g = Some(DisplayDirty {
                        x: x0 as u32,
                        y: y0 as u32,
                        w: uw as u32,
                        h: uh as u32,
                        stride: stride as u32,
                        mmap: Arc::clone(mmap),
                    });
                    gate.request();
                }
            }
            // Slot busy: keep accumulating, don't take() so acc remains set.
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn open_shm(path: &str) -> Option<Arc<Mmap>> {
    let file = File::open(path).ok()?;
    let mmap = unsafe { Mmap::map(&file).ok()? };
    if mmap.len() < SHM_PIXELS_OFFSET {
        return None;
    }
    if read_u32(&mmap, 0) != SHM_MAGIC {
        return None;
    }
    Some(Arc::new(mmap))
}

/// Plain little-endian read - use for non-generation fields after the acquire.
fn read_u32(mmap: &Mmap, offset: usize) -> u32 {
    let bytes: [u8; 4] = mmap[offset..offset + 4].try_into().unwrap();
    u32::from_le_bytes(bytes)
}

/// Acquire load of a u32 - pairs with QEMU's __ATOMIC_RELEASE store.
fn read_u32_acquire(mmap: &Mmap, offset: usize) -> u32 {
    // SAFETY: `mmap.as_ptr()` is page-aligned (mmap-allocated regions
    // always are), and the header layout fixes the generation counter
    // at offset 4, which is 4-byte aligned and therefore satisfies
    // `AtomicU32`'s alignment.  The offset is well within bounds of the
    // mapped region (caller-enforced via the `Mmap` size check at
    // construction time).
    let ptr = unsafe { mmap.as_ptr().add(offset) as *const AtomicU32 };
    // SAFETY: `ptr` was just derived from a live `Mmap` borrowed for the
    // duration of this call, the dereferenced `AtomicU32` provides its
    // own synchronisation, and the QEMU side performs only atomic
    // accesses to the same word.
    unsafe { (*ptr).load(Ordering::Acquire) }
}


/// Result of the Windows synthetic main-display pipeline diagnostic.
#[derive(Debug, Clone)]
pub struct SyntheticDisplayTestResult {
    pub frames_seen: u32,
    pub dirty_notifications: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub duration_ms: u128,
    pub approx_fps: f64,
    pub sample_rgba: [u8; 4],
    /// Full RGBA8888 framebuffer captured from the same shared-memory mapping
    /// after the synthetic animation completes. Diagnostics uploads this to an
    /// egui texture so the user can visually confirm the final UI texture path.
    pub preview_rgba: Vec<u8>,
}

/// Exercise the same `main.shm` reader used by the real LCD path without
/// requiring CDJ firmware. A synthetic producer writes a moving RGBA pattern
/// into a valid QEMU shm-display file and bumps `generation` at ~60 Hz.
///
/// This verifies:
/// - header layout / dimensions / stride
/// - generation polling
/// - dirty-region delivery
/// - pixel visibility through the mmap reader
#[cfg(windows)]
pub fn run_synthetic_display_test(ctx: egui::Context) -> Result<SyntheticDisplayTestResult, String> {
    use memmap2::MmapMut;
    use std::fs::OpenOptions;
    use std::sync::atomic::{AtomicU32 as StdAtomicU32, Ordering as StdOrdering};
    use std::time::Instant;

    const W: usize = 1280;
    const H: usize = 720;
    const STRIDE: usize = W * 4;
    const LEN: usize = SHM_PIXELS_OFFSET + STRIDE * H;
    const TEST_FRAMES: u32 = 90;

    let dir = std::env::temp_dir().join("cdj3k-emu-displaytest");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("create display-test directory {}: {e}", dir.display()))?;
    let path = dir.join("main.shm");

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .map_err(|e| format!("create {}: {e}", path.display()))?;
    file.set_len(LEN as u64)
        .map_err(|e| format!("resize {}: {e}", path.display()))?;

    let mut mmap = unsafe { MmapMut::map_mut(&file) }
        .map_err(|e| format!("mmap {}: {e}", path.display()))?;

    fn put_u32(buf: &mut [u8], off: usize, v: u32) {
        buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }

    put_u32(&mut mmap, 0, SHM_MAGIC);
    put_u32(&mut mmap, 4, 0);
    put_u32(&mut mmap, 8, W as u32);
    put_u32(&mut mmap, 12, H as u32);
    put_u32(&mut mmap, 16, STRIDE as u32);
    put_u32(&mut mmap, 20, 1);
    put_u32(&mut mmap, 24, 0);
    put_u32(&mut mmap, 28, 0);
    put_u32(&mut mmap, 32, W as u32);
    put_u32(&mut mmap, 36, H as u32);

    // Initial pattern: horizontal/vertical gradient.
    for y in 0..H {
        for x in 0..W {
            let off = SHM_PIXELS_OFFSET + y * STRIDE + x * 4;
            mmap[off] = ((x * 255) / (W - 1)) as u8;
            mmap[off + 1] = ((y * 255) / (H - 1)) as u8;
            mmap[off + 2] = 64;
            mmap[off + 3] = 255;
        }
    }
    mmap.flush().ok();

    let gate = crate::RepaintGate::new_60fps(ctx);
    let stream = MainLcdStream::new(&dir.to_string_lossy(), gate);

    // Give the reader time to map the freshly-created file.
    let connect_deadline = Instant::now() + Duration::from_secs(3);
    while !stream.is_connected() && Instant::now() < connect_deadline {
        thread::sleep(Duration::from_millis(20));
    }
    if !stream.is_connected() {
        return Err("MainLcdStream did not connect to synthetic main.shm".to_string());
    }

    let start = Instant::now();
    let mut dirty_notifications = 0u32;
    let mut last_sample = [0u8; 4];

    for frame in 1..=TEST_FRAMES {
        // A moving 96x96 square provides a realistic small dirty rectangle.
        let box_w = 96usize;
        let box_h = 96usize;
        let x0 = ((frame as usize * 13) % (W - box_w)).max(1);
        let y0 = ((frame as usize * 7) % (H - box_h)).max(1);

        for y in y0..y0 + box_h {
            for x in x0..x0 + box_w {
                let off = SHM_PIXELS_OFFSET + y * STRIDE + x * 4;
                mmap[off] = (frame.wrapping_mul(3) & 0xff) as u8;
                mmap[off + 1] = (255u32.wrapping_sub(frame * 2) & 0xff) as u8;
                mmap[off + 2] = ((x + y) & 0xff) as u8;
                mmap[off + 3] = 255;
            }
        }

        put_u32(&mut mmap, 24, x0 as u32);
        put_u32(&mut mmap, 28, y0 as u32);
        put_u32(&mut mmap, 32, box_w as u32);
        put_u32(&mut mmap, 36, box_h as u32);

        // RELEASE-store generation just like QEMU's shm backend.
        let gen_ptr = unsafe { mmap.as_mut_ptr().add(4) as *mut StdAtomicU32 };
        unsafe { (&*gen_ptr).store(frame, StdOrdering::Release) };

        thread::sleep(Duration::from_millis(17));

        if let Some(dirty) = stream.take() {
            dirty_notifications += 1;
            let off = SHM_PIXELS_OFFSET
                + dirty.y as usize * dirty.stride as usize
                + dirty.x as usize * 4;
            if off + 4 <= dirty.mmap.len() {
                last_sample.copy_from_slice(&dirty.mmap[off..off + 4]);
            }
        }
    }

    // Allow final generation to be observed.
    thread::sleep(Duration::from_millis(100));
    while let Some(dirty) = stream.take() {
        dirty_notifications += 1;
        let off = SHM_PIXELS_OFFSET
            + dirty.y as usize * dirty.stride as usize
            + dirty.x as usize * 4;
        if off + 4 <= dirty.mmap.len() {
            last_sample.copy_from_slice(&dirty.mmap[off..off + 4]);
        }
    }

    let elapsed = start.elapsed();
    let frames_seen = stream.frames_seen();
    if frames_seen < TEST_FRAMES / 2 {
        return Err(format!(
            "MainLcdStream observed too few frame generations: {frames_seen}/{TEST_FRAMES}"
        ));
    }
    if dirty_notifications == 0 {
        return Err("MainLcdStream produced no dirty notifications".to_string());
    }

    // Clear magic so the reader exits its poll loop cleanly.
    put_u32(&mut mmap, 0, 0);
    mmap.flush().ok();

    let seconds = elapsed.as_secs_f64().max(0.001);

    // Capture the exact RGBA framebuffer bytes from main.shm. This is not a
    // separately generated preview: it is copied from the same mapping that
    // MainLcdStream just consumed.
    let preview_rgba =
        mmap[SHM_PIXELS_OFFSET..SHM_PIXELS_OFFSET + STRIDE * H].to_vec();

    Ok(SyntheticDisplayTestResult {
        frames_seen,
        dirty_notifications,
        width: W as u32,
        height: H as u32,
        stride: STRIDE as u32,
        duration_ms: elapsed.as_millis(),
        approx_fps: frames_seen as f64 / seconds,
        sample_rgba: last_sample,
        preview_rgba,
    })
}


#[derive(Debug, Clone)]
pub struct QemuShmIntegrationResult {
    pub connected: bool,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub generation: u32,
    pub shm_bytes: u64,
    pub preview_rgba: Vec<u8>,
}

#[cfg(windows)]
pub fn inspect_qemu_main_shm(
    ctx: egui::Context,
    sock_dir: &std::path::Path,
) -> Result<QemuShmIntegrationResult, String> {
    use memmap2::MmapOptions;
    use std::fs::OpenOptions;
    use std::time::Instant;

    let gate = crate::RepaintGate::new_60fps(ctx);
    let stream = MainLcdStream::new(&sock_dir.to_string_lossy(), gate);

    let deadline = Instant::now() + Duration::from_secs(4);
    while !stream.is_connected() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(25));
    }
    if !stream.is_connected() {
        return Err(format!(
            "MainLcdStream did not connect to {}",
            sock_dir.join("main.shm").display()
        ));
    }

    let path = sock_dir.join("main.shm");
    let file = OpenOptions::new()
        .read(true)
        .open(&path)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    let meta = file.metadata()
        .map_err(|e| format!("stat {}: {e}", path.display()))?;
    let mmap = unsafe { MmapOptions::new().map(&file) }
        .map_err(|e| format!("mmap {}: {e}", path.display()))?;

    if mmap.len() < SHM_PIXELS_OFFSET {
        return Err(format!("main.shm too small: {} bytes", mmap.len()));
    }

    fn get_u32(buf: &[u8], off: usize) -> u32 {
        u32::from_le_bytes(buf[off..off + 4].try_into().unwrap())
    }

    let magic = get_u32(&mmap, 0);
    if magic != SHM_MAGIC {
        return Err(format!(
            "unexpected shm magic 0x{magic:08x}, expected 0x{SHM_MAGIC:08x}"
        ));
    }

    let generation = get_u32(&mmap, 4);
    let width = get_u32(&mmap, 8);
    let height = get_u32(&mmap, 12);
    let stride = get_u32(&mmap, 16);

    if width == 0 || height == 0 || stride < width.saturating_mul(4) {
        return Err(format!(
            "invalid QEMU framebuffer geometry: {width}x{height}, stride {stride}"
        ));
    }

    let pixel_bytes = stride as usize * height as usize;
    let end = SHM_PIXELS_OFFSET
        .checked_add(pixel_bytes)
        .ok_or_else(|| "framebuffer size overflow".to_string())?;
    if end > mmap.len() {
        return Err(format!(
            "main.shm truncated: need {end} bytes, have {}",
            mmap.len()
        ));
    }

    let mut preview_rgba = vec![0u8; width as usize * height as usize * 4];
    for y in 0..height as usize {
        let src = SHM_PIXELS_OFFSET + y * stride as usize;
        let dst = y * width as usize * 4;
        preview_rgba[dst..dst + width as usize * 4]
            .copy_from_slice(&mmap[src..src + width as usize * 4]);
    }

    Ok(QemuShmIntegrationResult {
        connected: true,
        width,
        height,
        stride,
        generation,
        shm_bytes: meta.len(),
        preview_rgba,
    })
}


/// Result of the live real-QEMU shared-memory animation diagnostic.
#[derive(Debug, Clone)]
pub struct LiveQemuDisplayResult {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub frames_written: u32,
    pub frames_observed: u32,
    pub dirty_notifications: u32,
    pub dropped_generations: u32,
    pub duration_ms: u128,
    pub observed_fps: f64,
    pub avg_frame_interval_ms: f64,
    pub min_frame_interval_ms: f64,
    pub max_frame_interval_ms: f64,
    pub final_rgba: Vec<u8>,
}

/// Exercise a live, continuously changing framebuffer using the *real*
/// QEMU-created main.shm file and the normal MainLcdStream reader.
///
/// QEMU owns/creates the shared-memory surface and provides a valid graphics
/// console. Because no proprietary guest firmware is available to draw pixels,
/// this diagnostic writes a moving test overlay directly into that QEMU-owned
/// framebuffer mapping. This validates live end-to-end updates through the
/// exact shared-memory file and stream reader used by the emulator.
#[cfg(windows)]
pub fn run_live_qemu_shm_animation<F>(
    ctx: egui::Context,
    sock_dir: &std::path::Path,
    frames: u32,
    mut on_frame: F,
) -> Result<LiveQemuDisplayResult, String>
where
    F: FnMut(u32, u32, Vec<u8>),
{
    use memmap2::MmapMut;
    use std::fs::OpenOptions;
    use std::sync::atomic::{AtomicU32 as StdAtomicU32, Ordering as StdOrdering};
    use std::time::Instant;

    let path = sock_dir.join("main.shm");
    let deadline = Instant::now() + Duration::from_secs(4);
    while !path.is_file() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(25));
    }
    if !path.is_file() {
        return Err(format!("QEMU main.shm not found: {}", path.display()));
    }

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("open {} read/write: {e}", path.display()))?;
    let mut mmap = unsafe { MmapMut::map_mut(&file) }
        .map_err(|e| format!("mmap {}: {e}", path.display()))?;

    if mmap.len() < SHM_PIXELS_OFFSET {
        return Err(format!("main.shm too small: {} bytes", mmap.len()));
    }

    fn get_u32(buf: &[u8], off: usize) -> u32 {
        u32::from_le_bytes(buf[off..off + 4].try_into().unwrap())
    }
    fn put_u32(buf: &mut [u8], off: usize, v: u32) {
        buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }

    let magic = get_u32(&mmap, 0);
    if magic != SHM_MAGIC {
        return Err(format!(
            "unexpected shm magic 0x{magic:08x}, expected 0x{SHM_MAGIC:08x}"
        ));
    }

    let width = get_u32(&mmap, 8);
    let height = get_u32(&mmap, 12);
    let stride = get_u32(&mmap, 16);
    if width == 0 || height == 0 || stride < width.saturating_mul(4) {
        return Err(format!(
            "invalid QEMU framebuffer geometry: {width}x{height}, stride {stride}"
        ));
    }

    let required = SHM_PIXELS_OFFSET + stride as usize * height as usize;
    if required > mmap.len() {
        return Err(format!(
            "QEMU main.shm truncated: need {required} bytes, have {}",
            mmap.len()
        ));
    }

    let gate = crate::RepaintGate::new_60fps(ctx);
    let stream = MainLcdStream::new(&sock_dir.to_string_lossy(), gate);

    let connect_deadline = Instant::now() + Duration::from_secs(3);
    while !stream.is_connected() && Instant::now() < connect_deadline {
        thread::sleep(Duration::from_millis(20));
    }
    if !stream.is_connected() {
        return Err("MainLcdStream did not connect to QEMU main.shm".to_string());
    }

    let start = Instant::now();
    let mut dirty_notifications = 0u32;
    let mut dropped_generations = 0u32;
    let mut intervals_ms: Vec<f64> = Vec::new();
    let mut last_observed_at: Option<Instant> = None;
    let mut last_seen_generation = stream.frames_seen();
    let base_generation = get_u32(&mmap, 4);

    // Draw a moving high-contrast bar at ~30 FPS for a few seconds.
    for frame in 0..frames {
        let bar_w = (width / 6).max(24);
        let x0 = if width > bar_w {
            (frame * 17) % (width - bar_w)
        } else {
            0
        };
        let y0 = height / 3;
        let bar_h = (height / 3).max(24).min(height.saturating_sub(y0));

        // Dim the full framebuffer slightly so motion is visually obvious.
        for y in 0..height as usize {
            let row = SHM_PIXELS_OFFSET + y * stride as usize;
            for x in 0..width as usize {
                let off = row + x * 4;
                mmap[off] = ((x as u32 * 255 / width.max(1)) as u8) / 3;
                mmap[off + 1] = ((y as u32 * 255 / height.max(1)) as u8) / 3;
                mmap[off + 2] = 24;
                mmap[off + 3] = 255;
            }
        }

        for y in y0..y0 + bar_h {
            let row = SHM_PIXELS_OFFSET + y as usize * stride as usize;
            for x in x0..x0 + bar_w {
                let off = row + x as usize * 4;
                mmap[off] = (frame.wrapping_mul(9) & 0xff) as u8;
                mmap[off + 1] = 220;
                mmap[off + 2] = (255u32.wrapping_sub(frame * 5) & 0xff) as u8;
                mmap[off + 3] = 255;
            }
        }

        put_u32(&mut mmap, 24, 0);
        put_u32(&mut mmap, 28, 0);
        put_u32(&mut mmap, 32, width);
        put_u32(&mut mmap, 36, height);

        let gen = base_generation.wrapping_add(frame).wrapping_add(1);
        let gen_ptr = unsafe { mmap.as_mut_ptr().add(4) as *mut StdAtomicU32 };
        unsafe { (&*gen_ptr).store(gen, StdOrdering::Release) };

        if let Some(_) = stream.take() {
            dirty_notifications += 1;
            let now = Instant::now();
            if let Some(prev) = last_observed_at {
                intervals_ms.push((now - prev).as_secs_f64() * 1000.0);
            }
            last_observed_at = Some(now);

            let current_seen = stream.frames_seen();
            if current_seen > last_seen_generation + 1 {
                dropped_generations = dropped_generations
                    .saturating_add(current_seen - last_seen_generation - 1);
            }
            last_seen_generation = current_seen;
        }

        // Push a UI preview roughly every 3 frames (~10 FPS) so the diagnostics
        // window visibly animates without copying a full framebuffer at 30 FPS.
        if frame % 3 == 0 {
            let mut packed = vec![0u8; width as usize * height as usize * 4];
            for y in 0..height as usize {
                let src = SHM_PIXELS_OFFSET + y * stride as usize;
                let dst = y * width as usize * 4;
                packed[dst..dst + width as usize * 4]
                    .copy_from_slice(&mmap[src..src + width as usize * 4]);
            }
            on_frame(width, height, packed);
        }

        thread::sleep(Duration::from_millis(33));
    }

    thread::sleep(Duration::from_millis(120));
    while let Some(_) = stream.take() {
        dirty_notifications += 1;
    }

    let elapsed = start.elapsed();
    let frames_observed = stream.frames_seen();
    if frames_observed < frames / 2 {
        return Err(format!(
            "MainLcdStream observed too few live QEMU shm generations: {frames_observed}/{frames}"
        ));
    }

    let mut final_rgba = vec![0u8; width as usize * height as usize * 4];
    for y in 0..height as usize {
        let src = SHM_PIXELS_OFFSET + y * stride as usize;
        let dst = y * width as usize * 4;
        final_rgba[dst..dst + width as usize * 4]
            .copy_from_slice(&mmap[src..src + width as usize * 4]);
    }

    let secs = elapsed.as_secs_f64().max(0.001);
    let (avg_frame_interval_ms, min_frame_interval_ms, max_frame_interval_ms) =
        if intervals_ms.is_empty() {
            (0.0, 0.0, 0.0)
        } else {
            let sum: f64 = intervals_ms.iter().sum();
            let min = intervals_ms
                .iter()
                .copied()
                .fold(f64::INFINITY, f64::min);
            let max = intervals_ms
                .iter()
                .copied()
                .fold(0.0_f64, f64::max);
            (sum / intervals_ms.len() as f64, min, max)
        };

    Ok(LiveQemuDisplayResult {
        width,
        height,
        stride,
        frames_written: frames,
        frames_observed,
        dirty_notifications,
        dropped_generations,
        duration_ms: elapsed.as_millis(),
        observed_fps: frames_observed as f64 / secs,
        avg_frame_interval_ms,
        min_frame_interval_ms,
        max_frame_interval_ms,
        final_rgba,
    })
}
