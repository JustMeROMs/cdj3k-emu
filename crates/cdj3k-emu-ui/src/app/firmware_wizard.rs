//! Firmware install wizard: decrypt UPD → extract kernel + initramfs → provision eMMC.

use std::path::PathBuf;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};

use egui::{
    Align, Button, Color32, ComboBox, Context, Frame, Margin, RichText, Rounding, Stroke, TextEdit,
};

use cdj3k_emu_platform::{desktop::open_file_picker, menu_state};

use super::diagnostics::DiagnosticsWindow;

// ── Provision step ────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub enum ProvisionStep {
    Decrypting,
    ExtractingKernel,
    PatchingInitramfs,
    CreatingEmmc,
    Done,
    Error(String),
}

impl ProvisionStep {
    fn label(&self) -> &str {
        match self {
            Self::Decrypting => "Decrypting firmware…",
            Self::ExtractingKernel => "Extracting kernel…",
            Self::PatchingInitramfs => "Patching initramfs…",
            Self::CreatingEmmc => "Creating eMMC image…",
            Self::Done => "Done",
            Self::Error(_) => "Error",
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Error(_))
    }
}

// ── Wizard ────────────────────────────────────────────────────────────────────

pub struct FirmwareWizard {
    pub open: bool,
    upd_path: String,
    key_path: String,
    /// Target slot for provisioning (1..=MAX_INSTANCES). Defaults to the
    /// current window's instance so the obvious thing happens by default.
    target_instance: u32,
    /// Shared log buffer; provision thread appends lines, UI reads every frame.
    log: Arc<Mutex<String>>,
    /// Set while provisioning is running; None = form or finished.
    provision_status: Option<Arc<Mutex<ProvisionStep>>>,
    /// Cached terminal state after thread completes.
    terminal: Option<ProvisionStep>,
    /// Tracks whether the wizard viewport has had a focus command sent since
    /// it was last opened. eframe creates the viewport on the first frame the
    /// wizard is shown; the main window keeps stealing focus on macOS until we
    /// explicitly raise the wizard.
    focused_after_open: bool,
    diagnostics: DiagnosticsWindow,
}

impl FirmwareWizard {
    pub fn new() -> Self {
        Self {
            open: false,
            upd_path: String::new(),
            key_path: String::new(),
            target_instance: menu_state::lock().current_instance_id.max(1),
            log: Arc::new(Mutex::new(String::new())),
            provision_status: None,
            terminal: None,
            focused_after_open: false,
            diagnostics: DiagnosticsWindow::new(),
        }
    }

    pub fn show(&mut self, ctx: &Context) {
        self.diagnostics.show(ctx);

        if !self.open {
            self.focused_after_open = false;
            return;
        }

        // Poll running thread for completion.
        if let Some(status) = &self.provision_status {
            let step = status.lock().unwrap().clone();
            if step.is_terminal() {
                self.terminal = Some(step);
                self.provision_status = None;
            }
        }

        let close_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let close_inner = close_flag.clone();

        let need_focus = !self.focused_after_open;
        let this = &mut *self;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("firmware_wizard"),
            egui::ViewportBuilder::default()
                .with_title("Install Firmware")
                .with_inner_size([620.0, 460.0])
                .with_resizable(false)
                .with_active(true)
                .with_resizable(true),
            move |ctx, _class| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    close_inner.store(true, Relaxed);
                }
                if need_focus {
                    // Raise above the just-created main window. Done from
                    // inside the viewport closure so the command targets this
                    // viewport's id, not the parent's.
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                egui::CentralPanel::default()
                    .frame(
                        Frame::default()
                            .fill(Color32::from_rgb(24, 25, 28))
                            .inner_margin(Margin::same(18.0)),
                    )
                    .show(ctx, |ui| {
                        this.draw(ui, &close_inner);
                    });
            },
        );
        self.focused_after_open = true;

        if close_flag.load(Relaxed) {
            self.open = false;
            self.reset();
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui, close: &Arc<std::sync::atomic::AtomicBool>) {
        let running = self.provision_status.is_some();
        let accent = Color32::from_rgb(64, 160, 220);
        const EDGE_INSET: f32 = 8.0;

        // ── Header ────────────────────────────────────────────────────────────
        ui.label(
            RichText::new("Install Firmware")
                .size(20.0)
                .strong()
                .color(Color32::from_rgb(230, 230, 232)),
        );
        ui.label(
            RichText::new("Decrypt a CDJ-3000 .UPD, patch the kernel and initramfs, and provision an eMMC image.")
                .size(12.0)
                .color(Color32::from_rgb(150, 150, 155)),
        );
        ui.add_space(12.0);

        // ── Status banner (running / done / error) ────────────────────────────
        match (&self.provision_status, &self.terminal) {
            (Some(status), _) => {
                let label = status.lock().unwrap().label().to_owned();
                self.banner(ui, Color32::from_rgb(36, 50, 70), Some(accent), |ui| {
                    ui.spinner();
                    ui.add_space(8.0);
                    ui.label(RichText::new(&label).size(13.0).color(Color32::WHITE));
                });
            }
            (None, Some(ProvisionStep::Done)) => {
                self.banner(
                    ui,
                    Color32::from_rgb(28, 56, 36),
                    Some(Color32::from_rgb(80, 200, 120)),
                    |ui| {
                        ui.label(
                            RichText::new("Firmware installed successfully")
                                .size(14.0)
                                .strong()
                                .color(Color32::from_rgb(170, 240, 190)),
                        );
                    },
                );
            }
            (None, Some(ProvisionStep::Error(msg))) => {
                let msg = msg.clone();
                self.banner(
                    ui,
                    Color32::from_rgb(60, 30, 30),
                    Some(Color32::from_rgb(220, 80, 80)),
                    |ui| {
                        ui.label(
                            RichText::new(msg)
                                .size(13.0)
                                .color(Color32::from_rgb(255, 200, 200)),
                        );
                    },
                );
            }
            _ => {}
        }

        // ── Form fields (hidden while running) ───────────────────────────────
        if !running {
            ui.add_space(4.0);
            self.section_label(ui, "Install to slot");
            ui.horizontal(|ui| {
                ComboBox::from_id_salt("wizard_slot")
                    .width(160.0)
                    .selected_text(format!("Slot {}", self.target_instance))
                    .show_ui(ui, |ui| {
                        for n in 1..=menu_state::MAX_INSTANCES {
                            ui.selectable_value(&mut self.target_instance, n, format!("Slot {n}"));
                        }
                    });
                ui.add_space(8.0);
                let cur = menu_state::lock().current_instance_id;
                let hint = if self.target_instance == cur {
                    "this window"
                } else {
                    "open from the Instances menu after install"
                };
                ui.label(
                    RichText::new(hint)
                        .size(11.0)
                        .color(Color32::from_rgb(140, 140, 145)),
                );
            });

            // Compute once so both rows are guaranteed the same field width.
            let field_w = {
                const BTN_W: f32 = 80.0;
                ui.available_width() - BTN_W - ui.spacing().item_spacing.x - EDGE_INSET
            };

            ui.add_space(14.0);
            self.section_label(ui, "Firmware (.UPD)");
            self.path_row(ui, "wizard_upd", true, field_w);
            let (upd_ok, upd_status) = self.firmware_status();
            self.validation_line(ui, upd_ok, &upd_status);

            ui.add_space(10.0);
            self.section_label(ui, "Decryption key");
            self.path_row(ui, "wizard_key", false, field_w);
            let (key_ok, key_status) = self.key_status();
            self.validation_line(ui, key_ok, &key_status);

            ui.add_space(10.0);
            let (env_ok, env_status) = self.windows_environment_status();
            self.validation_line(ui, env_ok, &env_status);
        }

        ui.add_space(14.0);

        // ── Log area ──────────────────────────────────────────────────────────
        self.section_label(ui, "Log");
        let log_snapshot = self.log.lock().unwrap().clone();
        let log_height = ui.available_height() - 56.0; // leave room for bottom buttons
        Frame::default()
            .fill(Color32::from_rgb(14, 15, 17))
            .stroke(Stroke::new(1.0, Color32::from_rgb(40, 42, 48)))
            .rounding(Rounding::same(6.0))
            .inner_margin(Margin::same(6.0))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("provision_log")
                    .max_height(log_height.max(80.0))
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(
                            TextEdit::multiline(&mut log_snapshot.as_str())
                                .desired_width(ui.available_width())
                                .desired_rows(8)
                                .frame(false)
                                .font(egui::TextStyle::Monospace)
                                .text_color(Color32::from_rgb(180, 230, 180)),
                        );
                    });
            });

        ui.add_space(10.0);

        // ── Bottom action bar ─────────────────────────────────────────────────
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(EDGE_INSET);
            match (&self.provision_status, &self.terminal) {
                (Some(_), _) => {}

                (None, Some(ProvisionStep::Done)) => {
                    let restart = Button::new(
                        RichText::new("Restart")
                            .size(15.0)
                            .strong()
                            .color(Color32::WHITE),
                    )
                    .fill(accent);
                    if ui.add_sized([110.0, 32.0], restart).clicked() {
                        let mut s = menu_state::lock();
                        s.shade_forced = true;
                        s.restart_requested = true;
                        drop(s);
                        close.store(true, Relaxed);
                    }
                }

                (None, Some(ProvisionStep::Error(_))) => {
                    if ui.add_sized([100.0, 32.0], Button::new("Close")).clicked() {
                        close.store(true, Relaxed);
                    }
                    if ui.add_sized([100.0, 32.0], Button::new("← Back")).clicked() {
                        self.terminal = None;
                    }
                }

                _ => {
                    let can_install =
                        self.firmware_status().0
                        && self.key_status().0
                        && self.windows_environment_status().0;
                    ui.add_enabled_ui(can_install, |ui| {
                        let install = Button::new(
                            RichText::new("Install")
                                .size(15.0)
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(accent);
                        if ui.add_sized([110.0, 32.0], install).clicked() {
                            self.start_provision(ui.ctx().clone());
                        }
                    });
                    if ui.add_sized([100.0, 32.0], Button::new("Cancel")).clicked() {
                        close.store(true, Relaxed);
                    }
                }
            }
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let diagnostics = Button::new(
                RichText::new("System Diagnostics…")
                    .size(12.0)
                    .strong()
                    .color(Color32::from_rgb(210, 220, 230)),
            )
            .fill(Color32::from_rgb(45, 50, 58));
            if ui.add_sized([160.0, 28.0], diagnostics).clicked() {
                self.diagnostics.open = true;
            }
            ui.label(
                RichText::new("No firmware or AES key required")
                    .size(10.5)
                    .color(Color32::from_rgb(125, 150, 135)),
            );
        });
    }

    fn section_label(&self, ui: &mut egui::Ui, text: &str) {
        ui.label(
            RichText::new(text)
                .size(11.0)
                .color(Color32::from_rgb(170, 170, 175))
                .strong(),
        );
        ui.add_space(2.0);
    }

    fn banner(
        &self,
        ui: &mut egui::Ui,
        fill: Color32,
        accent: Option<Color32>,
        contents: impl FnOnce(&mut egui::Ui),
    ) {
        Frame::default()
            .fill(fill)
            .stroke(accent.map_or(Stroke::NONE, |c| Stroke::new(1.0, c)))
            .rounding(Rounding::same(6.0))
            .inner_margin(Margin::symmetric(10.0, 8.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| contents(ui));
            });
        ui.add_space(8.0);
    }

    fn path_row(&mut self, ui: &mut egui::Ui, salt: &str, is_upd: bool, field_w: f32) {
        const BTN_W: f32 = 80.0;
        const BTN_H: f32 = 24.0;
        let hint = if is_upd {
            "/path/to/firmware.UPD"
        } else {
            "/path/to/aes256.key"
        };
        // Accumulate the file-picker result outside the closure to avoid
        // conflicting borrows with the TextEdit's &mut path reference.
        let mut new_pick: Option<String> = None;
        ui.horizontal(|ui| {
            let path = if is_upd {
                &mut self.upd_path
            } else {
                &mut self.key_path
            };
            ui.add(
                TextEdit::singleline(path)
                    .id_salt(salt)
                    .desired_width(field_w)
                    .hint_text(hint)
                    .font(egui::TextStyle::Monospace),
            );
            if ui
                .add_sized([BTN_W, BTN_H], Button::new("Browse…"))
                .clicked()
            {
                let picked = if is_upd {
                    open_file_picker("Select firmware (.UPD)", &["UPD"])
                } else {
                    open_file_picker("Select key file", &[])
                };
                if let Some(p) = picked {
                    new_pick = Some(p.to_string_lossy().into_owned());
                }
            }
        });
        if let Some(val) = new_pick {
            if is_upd {
                self.upd_path = val;
            } else {
                self.key_path = val;
            }
        }
    }

    fn validation_line(&self, ui: &mut egui::Ui, ok: bool, text: &str) {
        let (symbol, color) = if ok {
            ("✓", Color32::from_rgb(95, 210, 135))
        } else {
            ("●", Color32::from_rgb(220, 105, 105))
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(symbol).size(12.0).strong().color(color));
            ui.label(
                RichText::new(text)
                    .size(10.5)
                    .color(if ok {
                        Color32::from_rgb(145, 205, 160)
                    } else {
                        Color32::from_rgb(205, 145, 145)
                    }),
            );
        });
    }

    fn firmware_status(&self) -> (bool, String) {
        if self.upd_path.trim().is_empty() {
            return (false, "Firmware not selected".to_string());
        }
        let p = std::path::Path::new(&self.upd_path);
        if !p.is_file() {
            return (false, "Firmware file not found".to_string());
        }
        let ext_ok = p
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| x.eq_ignore_ascii_case("upd"))
            .unwrap_or(false);
        if !ext_ok {
            return (false, "Selected file is not a .UPD firmware file".to_string());
        }
        (true, "Firmware OK".to_string())
    }

    fn key_status(&self) -> (bool, String) {
        if self.key_path.trim().is_empty() {
            return (false, "Decryption key not selected".to_string());
        }
        let p = std::path::Path::new(&self.key_path);
        if !p.is_file() {
            return (false, "Decryption key file not found".to_string());
        }
        (true, "Key file OK".to_string())
    }

    fn windows_environment_status(&self) -> (bool, String) {
        #[cfg(windows)]
        {
            let resources = bundled_resources();
            let kernel = resources.join("Image");
            if !kernel.is_file() {
                return (false, format!("Guest kernel missing: {}", kernel.display()));
            }

            let bin = resources.join("msys2").join("usr").join("bin");
            for tool in [
                "bash.exe",
                "cpio.exe",
                "find.exe",
                "chmod.exe",
                "sed.exe",
                "grep.exe",
                "gzip.exe",
            ] {
                if !bin.join(tool).is_file() {
                    return (false, format!("Provisioning tool missing: {tool}"));
                }
            }

            let qemu_img = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("qemu-img.exe")));
            match qemu_img {
                Some(p) if p.is_file() => {}
                Some(p) => {
                    return (false, format!("qemu-img.exe missing: {}", p.display()));
                }
                None => return (false, "Could not resolve application folder".to_string()),
            }

            return (true, "Windows provisioning environment ready".to_string());
        }

        #[cfg(not(windows))]
        {
            (true, "Provisioning environment ready".to_string())
        }
    }

    fn windows_preflight(&self) -> Result<Vec<String>, String> {
        let mut ok = Vec::new();

        let upd = std::path::Path::new(&self.upd_path);
        if !upd.is_file() {
            return Err(format!("Firmware file not found: {}", upd.display()));
        }
        ok.push(format!("Firmware found: {}", upd.display()));

        if upd
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| !x.eq_ignore_ascii_case("upd"))
            .unwrap_or(true)
        {
            return Err("Firmware file must be a .UPD file".to_string());
        }

        let key = std::path::Path::new(&self.key_path);
        if !key.is_file() {
            return Err(format!("Decryption key file not found: {}", key.display()));
        }
        ok.push(format!("Key file found: {}", key.display()));

        let resources = bundled_resources();
        let kernel = resources.join("Image");
        if !kernel.is_file() {
            return Err(format!(
                "Bundled guest kernel missing: {}",
                kernel.display()
            ));
        }
        ok.push(format!("Guest kernel OK: {}", kernel.display()));

        #[cfg(windows)]
        {
            let bin = resources.join("msys2").join("usr").join("bin");
            for tool in ["bash.exe", "cpio.exe", "find.exe", "chmod.exe", "sed.exe", "grep.exe", "gzip.exe"] {
                let p = bin.join(tool);
                if !p.is_file() {
                    return Err(format!("Bundled provisioning tool missing: {}", p.display()));
                }
            }
            ok.push(format!(
                "Provisioning tools OK: {}",
                bin.display()
            ));

            let qemu_img = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("qemu-img.exe")));
            match qemu_img {
                Some(p) if p.is_file() => ok.push(format!("qemu-img OK: {}", p.display())),
                Some(p) => {
                    return Err(format!(
                        "qemu-img.exe missing from Windows package: {}",
                        p.display()
                    ))
                }
                None => return Err("Could not resolve Windows application folder".to_string()),
            }
        }

        Ok(ok)
    }

    fn start_provision(&mut self, ctx: Context) {
        #[cfg(windows)]
        {
            match self.windows_preflight() {
                Ok(lines) => {
                    let mut log = self.log.lock().unwrap();
                    log.clear();
                    log.push_str("[preflight] Windows firmware installer checks\n");
                    for line in lines {
                        log.push_str("[OK] ");
                        log.push_str(&line);
                        log.push('\n');
                    }
                    log.push_str("[preflight] Ready to provision firmware\n");
                }
                Err(e) => {
                    *self.log.lock().unwrap() = format!(
                        "[preflight] FAILED\n[error] {e}\n"
                    );
                    self.terminal = Some(ProvisionStep::Error(format!(
                        "Preflight failed: {e}"
                    )));
                    return;
                }
            }
        }

        let key = match cdj3k_emu_firmware::LuksKey::from_file(std::path::Path::new(&self.key_path))
        {
            Ok(k) => k,
            Err(e) => {
                *self.log.lock().unwrap() = format!("[key] could not read key file: {e}\n");
                self.terminal = Some(ProvisionStep::Error(format!(
                    "Could not read key file: {e}"
                )));
                return;
            }
        };

        let upd = PathBuf::from(&self.upd_path);
        let target = self.target_instance;
        let status = Arc::new(Mutex::new(ProvisionStep::Decrypting));
        self.provision_status = Some(status.clone());

        // Recycle the same log Arc. On Windows, keep the preflight results
        // visible above the provisioning steps.
        #[cfg(not(windows))]
        self.log.lock().unwrap().clear();
        let log = self.log.clone();

        std::thread::Builder::new()
            .name("cdj3k-emu-provision".into())
            .spawn(move || provision(upd, key, target, status, log, ctx))
            .expect("failed to spawn provision thread");
    }

    fn reset(&mut self) {
        self.provision_status = None;
        self.terminal = None;
        self.log.lock().unwrap().clear();
        // Keep paths/key/slot so the user doesn't have to re-enter if they reopen.
    }
}

// ── Background provisioning ───────────────────────────────────────────────────

fn bundled_resources() -> std::path::PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let macos = exe.parent().unwrap_or(std::path::Path::new("."));
        let resources = macos.parent().unwrap_or(macos).join("Resources");
        if resources.exists() {
            return resources;
        }
        let dev = macos.join("resources");
        if dev.exists() {
            return dev;
        }
    }
    std::path::PathBuf::from("resources")
}

fn provision(
    upd_path: PathBuf,
    key: cdj3k_emu_firmware::LuksKey,
    target_instance: u32,
    status: Arc<Mutex<ProvisionStep>>,
    log: Arc<Mutex<String>>,
    ctx: Context,
) {
    macro_rules! set {
        ($step:expr) => {{
            *status.lock().unwrap() = $step;
            ctx.request_repaint();
        }};
    }
    macro_rules! log {
        ($($arg:tt)*) => {{
            let line = format!($($arg)*);
            eprintln!("cdj3k-emu-provision: {line}");
            {
                let mut g = log.lock().unwrap();
                g.push_str(&line);
                g.push('\n');
            }
            ctx.request_repaint();
        }};
    }
    macro_rules! try_step {
        ($step:expr, $expr:expr) => {
            match $expr {
                Ok(v) => v,
                Err(e) => {
                    log!("[error] {}", e.to_string());
                    set!(ProvisionStep::Error(
                        "Error while processing: see the console for details".to_string()
                    ));
                    return;
                }
            }
        };
    }

    log!("[slot] target = instance-{}", target_instance);

    // 1. Decrypt UPD → tmp ISO
    set!(ProvisionStep::Decrypting);
    log!("[decrypt] opening {}", upd_path.display());
    let tmp_iso =
        std::env::temp_dir().join(format!("cdj3k-emu-firmware-{}.iso.tmp", target_instance));
    log!("[decrypt] output → {}", tmp_iso.display());
    try_step!(
        ProvisionStep::Decrypting,
        cdj3k_emu_firmware::decrypt_upd(&upd_path, &key, &tmp_iso)
    );
    log!("[decrypt] OK - ISO written");

    let firmware_info = match cdj3k_emu_firmware::read_firmware_info(&tmp_iso) {
        Ok(info) => {
            let unknown = "(unknown)";
            log!(
                "[firmware] release={} rev_apl={} rev_kernel={} miniloader={}",
                info.release.as_deref().unwrap_or(unknown),
                info.rev_apl.as_deref().unwrap_or(unknown),
                info.rev_kernel.as_deref().unwrap_or(unknown),
                info.miniloader.as_deref().unwrap_or(unknown),
            );
            info
        }
        Err(e) => {
            log!("[firmware] could not read version info ({e}), defaulting to all-unknown");
            cdj3k_emu_storage::FirmwareInfo::default()
        }
    };

    // 2. Install vanilla kernel + extract Pioneer initramfs
    set!(ProvisionStep::ExtractingKernel);
    let out_dir = cdj3k_emu_storage::default_path(target_instance)
        .parent()
        .unwrap()
        .to_path_buf();
    try_step!(
        ProvisionStep::ExtractingKernel,
        std::fs::create_dir_all(&out_dir)
    );

    // Vanilla kernel ships in the app bundle - no Pioneer extraction or SMC patching.
    let resources_dir = bundled_resources();
    let kernel_src = resources_dir.join("Image");
    let kernel_out = out_dir.join("Image");
    log!(
        "[kernel] installing kernel {} → {}",
        kernel_src.display(),
        kernel_out.display()
    );
    try_step!(
        ProvisionStep::ExtractingKernel,
        std::fs::copy(&kernel_src, &kernel_out)
            .map(|_| ())
            .map_err(|e| std::io::Error::other(format!("copy Image: {e}")))
    );
    log!("[kernel] OK");

    // 3. Patch initramfs - still uses Pioneer firmware as the rootfs base
    //    (EP122 binary and Pioneer libraries live there).
    //    The Pioneer kernel is extracted to a temp file solely to unpack the
    //    embedded initramfs; it is not used as the final kernel.
    set!(ProvisionStep::PatchingInitramfs);
    log!("[patch] resources dir: {}", resources_dir.display());
    let pioneer_kernel_tmp = out_dir.join("Image.pioneer.tmp");
    let initramfs_patched = out_dir.join("initramfs-patched.cpio.gz");
    let initramfs_orig = out_dir.join("initramfs-orig.cpio");

    log!(
        "[initramfs] extracting Pioneer kernel (initramfs source) → {}",
        pioneer_kernel_tmp.display()
    );
    try_step!(
        ProvisionStep::PatchingInitramfs,
        cdj3k_emu_firmware::extract_kernel(&tmp_iso, &pioneer_kernel_tmp, |msg| log!("{}", msg))
            .map_err(|e| std::io::Error::other(format!("{e}")))
    );
    log!(
        "[initramfs] extracting from Pioneer kernel → {}",
        initramfs_orig.display()
    );
    try_step!(
        ProvisionStep::PatchingInitramfs,
        cdj3k_emu_firmware::extract_initramfs(&pioneer_kernel_tmp, &initramfs_orig)
            .map_err(|e| std::io::Error::other(format!("{e:?}")))
    );
    let _ = std::fs::remove_file(&pioneer_kernel_tmp);
    log!("[initramfs] OK");

    try_step!(
        ProvisionStep::PatchingInitramfs,
        cdj3k_emu_firmware::patch_initramfs(&initramfs_orig, &resources_dir, &initramfs_patched)
            .map_err(|e| std::io::Error::other(e.to_string()))
    );
    log!("[patch] OK → {}", initramfs_patched.display());
    let _ = std::fs::remove_file(&initramfs_orig);

    // 4. Create eMMC qcow2 (always replace on firmware install)
    set!(ProvisionStep::CreatingEmmc);
    let emmc_path = cdj3k_emu_storage::default_path(target_instance);
    if emmc_path.exists() {
        log!("[emmc] removing existing image → {}", emmc_path.display());
        try_step!(
            ProvisionStep::CreatingEmmc,
            std::fs::remove_file(&emmc_path)
        );
    }
    log!("[emmc] provisioning → {}", emmc_path.display());
    let emmc_cfg = cdj3k_emu_storage::EmmcConfig {
        path: emmc_path,
        instance_id: target_instance,
        firmware: firmware_info,
    };
    try_step!(
        ProvisionStep::CreatingEmmc,
        cdj3k_emu_storage::provision_emmc(&emmc_cfg)
    );
    log!("[emmc] OK");

    let _ = std::fs::remove_file(&tmp_iso);

    log!("[done] all steps completed");
    set!(ProvisionStep::Done);

    // Only auto-boot if we provisioned the slot this window owns; otherwise
    // the user can launch the target via the Instances menu.
    let mut s = menu_state::lock();
    if target_instance == s.current_instance_id {
        s.qemu_boot_requested = true;
    }
}
