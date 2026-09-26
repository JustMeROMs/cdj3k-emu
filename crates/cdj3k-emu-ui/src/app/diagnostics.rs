//! Windows host diagnostics panel. Designed to be useful before firmware is provisioned.

use std::sync::{Arc, Mutex};

use egui::{Button, Color32, Context, Frame, Margin, RichText, Rounding, Stroke};

#[derive(Clone)]
struct TestState {
    running: bool,
    result: Option<Result<String, String>>,
}

impl Default for TestState {
    fn default() -> Self {
        Self {
            running: false,
            result: None,
        }
    }
}

pub struct DiagnosticsWindow {
    pub open: bool,
    test: Arc<Mutex<TestState>>,
    display_test: Arc<Mutex<TestState>>,
    /// Captured framebuffer from the synthetic main.shm pipeline. Kept as a
    /// ColorImage so the diagnostics viewport can upload it through egui's
    /// texture manager and visibly render the same bytes the stream observed.
    display_preview: Arc<Mutex<Option<egui::ColorImage>>>,
    focused_after_open: bool,
}

impl DiagnosticsWindow {
    pub fn new() -> Self {
        Self {
            open: false,
            test: Arc::new(Mutex::new(TestState::default())),
            display_test: Arc::new(Mutex::new(TestState::default())),
            display_preview: Arc::new(Mutex::new(None)),
            focused_after_open: false,
        }
    }

    pub fn show(&mut self, ctx: &Context) {
        if !self.open {
            self.focused_after_open = false;
            return;
        }

        let close_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let close_inner = close_flag.clone();
        let need_focus = !self.focused_after_open;
        let test = self.test.clone();
        let display_test = self.display_test.clone();
        let display_preview = self.display_preview.clone();

        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("windows_system_diagnostics"),
            egui::ViewportBuilder::default()
                .with_title("CDJ3K Emulator - System Diagnostics")
                .with_inner_size([650.0, 540.0])
                .with_resizable(true)
                .with_active(true),
            move |ctx, _class| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    close_inner.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                if need_focus {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }

                egui::CentralPanel::default()
                    .frame(
                        Frame::default()
                            .fill(Color32::from_rgb(24, 25, 28))
                            .inner_margin(Margin::same(18.0)),
                    )
                    .show(ctx, |ui| {
                        ui.label(
                            RichText::new("Windows System Diagnostics")
                                .size(20.0)
                                .strong()
                                .color(Color32::from_rgb(230, 232, 235)),
                        );
                        ui.label(
                            RichText::new(
                                "These checks do not need CDJ-3000 firmware or an AES key.",
                            )
                            .size(11.5)
                            .color(Color32::from_rgb(145, 150, 158)),
                        );
                        ui.add_space(12.0);

                        draw_static_checks(ui);

                        ui.add_space(12.0);
                        ui.separator();
                        ui.add_space(10.0);

                        let state_snapshot = test.lock().unwrap().clone();

                        ui.horizontal(|ui| {
                            let run = Button::new(
                                RichText::new(if state_snapshot.running {
                                    "Running host test…"
                                } else {
                                    "Run Host Test"
                                })
                                .strong()
                                .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(55, 125, 185));

                            if ui
                                .add_enabled(!state_snapshot.running, run)
                                .clicked()
                            {
                                run_host_test(test.clone(), ctx.clone());
                            }

                            if ui.button("Open Logs Folder").clicked() {
                                open_logs_folder();
                            }
                        });

                        ui.add_space(8.0);
                        let display_snapshot = display_test.lock().unwrap().clone();
                        ui.horizontal(|ui| {
                            let run_display = Button::new(
                                RichText::new(if display_snapshot.running {
                                    "Running Display Test…"
                                } else {
                                    "Run Display Test"
                                })
                                .strong()
                                .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(120, 85, 175));

                            if ui
                                .add_enabled(!display_snapshot.running, run_display)
                                .clicked()
                            {
                                // Clear the previous preview before a fresh run.
                                if let Ok(mut preview) = display_preview.lock() {
                                    *preview = None;
                                }
                                run_display_test(
                                    display_test.clone(),
                                    display_preview.clone(),
                                    ctx.clone(),
                                );
                            }

                            ui.label(
                                RichText::new("Synthetic 1280×720 main.shm pipeline")
                                    .size(10.5)
                                    .color(Color32::from_rgb(145, 150, 158)),
                            );
                        });

                        if display_snapshot.running || display_snapshot.result.is_some() {
                            ui.add_space(6.0);
                            let display_text = match &display_snapshot.result {
                                None if display_snapshot.running => "Running…".to_string(),
                                None => "Not run yet".to_string(),
                                Some(Ok(s)) => format!("PASS\n{s}"),
                                Some(Err(e)) => format!("FAIL\n{e}"),
                            };
                            let display_ok = matches!(&display_snapshot.result, Some(Ok(_)));
                            Frame::default()
                                .fill(Color32::from_rgb(14, 15, 17))
                                .stroke(Stroke::new(
                                    1.0,
                                    if display_ok {
                                        Color32::from_rgb(115, 80, 170)
                                    } else {
                                        Color32::from_rgb(55, 58, 64)
                                    },
                                ))
                                .rounding(Rounding::same(6.0))
                                .inner_margin(Margin::same(10.0))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(display_text)
                                            .monospace()
                                            .size(10.5)
                                            .color(if display_ok {
                                                Color32::from_rgb(195, 170, 235)
                                            } else {
                                                Color32::from_rgb(195, 198, 205)
                                            }),
                                    );
                                });
                        }

                        let display_preview_ok =
                            matches!(&display_snapshot.result, Some(Ok(_)));
                        if display_preview_ok {
                            if let Ok(preview) = display_preview.lock() {
                                if let Some(image) = preview.as_ref() {
                                    ui.add_space(8.0);
                                    ui.label(
                                        RichText::new("Captured main.shm framebuffer preview")
                                            .size(10.5)
                                            .strong()
                                            .color(Color32::from_rgb(180, 185, 195)),
                                    );
                                    let texture = ui.ctx().load_texture(
                                        "cdj3k-diagnostics-main-shm-preview",
                                        image.clone(),
                                        egui::TextureOptions::LINEAR,
                                    );
                                    // Preserve 16:9 while fitting comfortably in the
                                    // diagnostics viewport.
                                    let preview_size = egui::vec2(512.0, 288.0);
                                    ui.image((texture.id(), preview_size));
                                    ui.label(
                                        RichText::new(
                                            "Visible texture is uploaded from the captured shared-memory framebuffer.",
                                        )
                                        .size(9.8)
                                        .color(Color32::from_rgb(125, 135, 145)),
                                    );
                                }
                            }
                        }

                        ui.add_space(12.0);
                        ui.label(
                            RichText::new("Host test result")
                                .size(11.0)
                                .strong()
                                .color(Color32::from_rgb(175, 178, 185)),
                        );
                        ui.add_space(4.0);

                        let result_text = match &state_snapshot.result {
                            None if state_snapshot.running => "Running…".to_string(),
                            None => "Not run yet".to_string(),
                            Some(Ok(s)) => format!("PASS\n{s}"),
                            Some(Err(e)) => format!("FAIL\n{e}"),
                        };

                        let ok = matches!(&state_snapshot.result, Some(Ok(_)));
                        Frame::default()
                            .fill(Color32::from_rgb(14, 15, 17))
                            .stroke(Stroke::new(
                                1.0,
                                if ok {
                                    Color32::from_rgb(70, 145, 95)
                                } else {
                                    Color32::from_rgb(55, 58, 64)
                                },
                            ))
                            .rounding(Rounding::same(6.0))
                            .inner_margin(Margin::same(10.0))
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .max_height(210.0)
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new(result_text)
                                                .monospace()
                                                .size(11.0)
                                                .color(if ok {
                                                    Color32::from_rgb(155, 225, 175)
                                                } else {
                                                    Color32::from_rgb(195, 198, 205)
                                                }),
                                        );
                                    });
                            });
                    });
            },
        );

        self.focused_after_open = true;
        if close_flag.load(std::sync::atomic::Ordering::Relaxed) {
            self.open = false;
        }
    }
}

fn status_row(ui: &mut egui::Ui, ok: bool, name: &str, detail: &str) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(if ok { "✓" } else { "●" })
                .strong()
                .color(if ok {
                    Color32::from_rgb(95, 210, 135)
                } else {
                    Color32::from_rgb(220, 105, 105)
                }),
        );
        ui.label(
            RichText::new(name)
                .strong()
                .color(Color32::from_rgb(215, 217, 222)),
        );
        ui.label(
            RichText::new(detail)
                .size(10.5)
                .color(Color32::from_rgb(145, 150, 158)),
        );
    });
}

fn draw_static_checks(ui: &mut egui::Ui) {
    #[cfg(windows)]
    {
        let qemu = cdj3k_emu_runtime::external_qemu_exe();
        match qemu {
            Some(p) => status_row(ui, true, "QEMU", &p.display().to_string()),
            None => status_row(ui, false, "QEMU", "qemu-system-aarch64.exe not found"),
        }

        let resources = bundled_resources();
        let image = resources.join("Image");
        status_row(
            ui,
            image.is_file(),
            "Guest kernel",
            &image.display().to_string(),
        );

        let tools = resources.join("msys2").join("usr").join("bin");
        let required = [
            "bash.exe",
            "cpio.exe",
            "find.exe",
            "chmod.exe",
            "sed.exe",
            "grep.exe",
            "gzip.exe",
        ];
        let tools_ok = required.iter().all(|x| tools.join(x).is_file());
        status_row(
            ui,
            tools_ok,
            "Provisioning tools",
            if tools_ok {
                "bundled toolchain ready"
            } else {
                "one or more bundled tools are missing"
            },
        );

        let runtime = std::env::temp_dir().join("cdj3k-emu-hosttest");
        status_row(
            ui,
            true,
            "Runtime folder",
            &runtime.display().to_string(),
        );

        let qemu_img = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("qemu-img.exe")));
        match qemu_img {
            Some(p) => status_row(ui, p.is_file(), "qemu-img", &p.display().to_string()),
            None => status_row(ui, false, "qemu-img", "application folder unavailable"),
        }
    }

    #[cfg(not(windows))]
    {
        status_row(
            ui,
            true,
            "Platform",
            "Diagnostics panel is primarily for the Windows port",
        );
    }
}

#[cfg(windows)]
fn run_host_test(test: Arc<Mutex<TestState>>, ctx: Context) {
    {
        let mut state = test.lock().unwrap();
        state.running = true;
        state.result = None;
    }

    std::thread::Builder::new()
        .name("cdj3k-host-diagnostics".into())
        .spawn(move || {
            let result = run_host_test_inner();
            {
                let mut state = test.lock().unwrap();
                state.running = false;
                state.result = Some(result);
            }
            ctx.request_repaint();
        })
        .ok();
}

#[cfg(not(windows))]
fn run_host_test(test: Arc<Mutex<TestState>>, ctx: Context) {
    let mut state = test.lock().unwrap();
    state.running = false;
    state.result = Some(Ok("Windows-only host integration test".to_string()));
    drop(state);
    ctx.request_repaint();
}

#[cfg(windows)]
fn run_host_test_inner() -> Result<String, String> {
    use std::process::Command;

    let exe = std::env::current_exe()
        .map_err(|e| format!("Could not locate cdj3k-emu.exe: {e}"))?;

    let out = Command::new(exe)
        .arg("--windows-host-test")
        .output()
        .map_err(|e| format!("Could not start host test: {e}"))?;

    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&out.stdout));
    text.push_str(&String::from_utf8_lossy(&out.stderr));

    if out.status.success() {
        Ok(text.trim().to_string())
    } else {
        Err(text.trim().to_string())
    }
}

#[cfg(windows)]
fn open_logs_folder() {
    use std::process::Command;
    let dir = std::env::temp_dir().join("cdj3k-emu-hosttest");
    let _ = std::fs::create_dir_all(&dir);
    let _ = Command::new("explorer.exe").arg(dir).spawn();
}

#[cfg(not(windows))]
fn open_logs_folder() {}

fn bundled_resources() -> std::path::PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let exe_dir = exe.parent().unwrap_or(std::path::Path::new("."));
        let bundle_resources = exe_dir
            .parent()
            .unwrap_or(exe_dir)
            .join("Resources");
        if bundle_resources.exists() {
            return bundle_resources;
        }
        let local_resources = exe_dir.join("resources");
        if local_resources.exists() {
            return local_resources;
        }
    }
    std::path::PathBuf::from("resources")
}


#[cfg(windows)]
fn run_display_test(
    test: Arc<Mutex<TestState>>,
    preview: Arc<Mutex<Option<egui::ColorImage>>>,
    ctx: Context,
) {
    {
        let mut state = test.lock().unwrap();
        state.running = true;
        state.result = None;
    }

    std::thread::Builder::new()
        .name("cdj3k-display-diagnostics".into())
        .spawn(move || {
            let result = match cdj3k_emu_streams::main_stream::run_synthetic_display_test(
                ctx.clone(),
            ) {
                Ok(r) => {
                    let expected = r.width as usize * r.height as usize * 4;
                    if r.preview_rgba.len() != expected {
                        Err(format!(
                            "Framebuffer capture size mismatch: {} bytes, expected {}",
                            r.preview_rgba.len(),
                            expected
                        ))
                    } else {
                        let image = egui::ColorImage::from_rgba_unmultiplied(
                            [r.width as usize, r.height as usize],
                            &r.preview_rgba,
                        );
                        if let Ok(mut slot) = preview.lock() {
                            *slot = Some(image);
                        }

                        Ok(format!(
                            "Resolution: {}×{}\nStride: {} bytes\nFrames observed: {}\nDirty notifications: {}\nElapsed: {} ms\nApprox reader FPS: {:.1}\nSample RGBA: {:?}\nTexture preview: READY",
                            r.width,
                            r.height,
                            r.stride,
                            r.frames_seen,
                            r.dirty_notifications,
                            r.duration_ms,
                            r.approx_fps,
                            r.sample_rgba
                        ))
                    }
                }
                Err(e) => Err(e),
            };
            {
                let mut state = test.lock().unwrap();
                state.running = false;
                state.result = Some(result);
            }
            ctx.request_repaint();
        })
        .ok();
}

#[cfg(not(windows))]
fn run_display_test(
    test: Arc<Mutex<TestState>>,
    _preview: Arc<Mutex<Option<egui::ColorImage>>>,
    ctx: Context,
) {
    let mut state = test.lock().unwrap();
    state.running = false;
    state.result = Some(Ok("Synthetic Windows display pipeline test is Windows-only".to_string()));
    drop(state);
    ctx.request_repaint();
}
