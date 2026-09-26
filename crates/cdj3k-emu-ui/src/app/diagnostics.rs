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
    qemu_display_test: Arc<Mutex<TestState>>,
    qemu_display_preview: Arc<Mutex<Option<egui::ColorImage>>>,
    live_qemu_test: Arc<Mutex<TestState>>,
    live_qemu_preview: Arc<Mutex<Option<egui::ColorImage>>>,
    qemu_stress_test: Arc<Mutex<TestState>>,
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
            qemu_display_test: Arc::new(Mutex::new(TestState::default())),
            qemu_display_preview: Arc::new(Mutex::new(None)),
            live_qemu_test: Arc::new(Mutex::new(TestState::default())),
            live_qemu_preview: Arc::new(Mutex::new(None)),
            qemu_stress_test: Arc::new(Mutex::new(TestState::default())),
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
        let qemu_display_test = self.qemu_display_test.clone();
        let qemu_display_preview = self.qemu_display_preview.clone();
        let live_qemu_test = self.live_qemu_test.clone();
        let live_qemu_preview = self.live_qemu_preview.clone();
        let qemu_stress_test = self.qemu_stress_test.clone();
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
                        let qemu_display_snapshot = qemu_display_test.lock().unwrap().clone();
                        ui.horizontal(|ui| {
                            let run = Button::new(
                                RichText::new(if qemu_display_snapshot.running {
                                    "Running QEMU Display Test…"
                                } else {
                                    "Run QEMU Display Test"
                                })
                                .strong()
                                .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(180, 105, 45));

                            if ui.add_enabled(!qemu_display_snapshot.running, run).clicked() {
                                if let Ok(mut preview) = qemu_display_preview.lock() {
                                    *preview = None;
                                }
                                run_qemu_display_test(
                                    qemu_display_test.clone(),
                                    qemu_display_preview.clone(),
                                    ctx.clone(),
                                );
                            }

                            ui.label(
                                RichText::new("Real QEMU console → main.shm → MainLcdStream")
                                    .size(10.5)
                                    .color(Color32::from_rgb(145, 150, 158)),
                            );
                        });

                        if qemu_display_snapshot.running || qemu_display_snapshot.result.is_some() {
                            ui.add_space(6.0);
                            let text = match &qemu_display_snapshot.result {
                                None if qemu_display_snapshot.running => "Running…".to_string(),
                                None => "Not run yet".to_string(),
                                Some(Ok(s)) => format!("PASS\n{s}"),
                                Some(Err(e)) => format!("FAIL\n{e}"),
                            };
                            let ok = matches!(&qemu_display_snapshot.result, Some(Ok(_)));
                            Frame::default()
                                .fill(Color32::from_rgb(14, 15, 17))
                                .stroke(Stroke::new(
                                    1.0,
                                    if ok {
                                        Color32::from_rgb(190, 115, 55)
                                    } else {
                                        Color32::from_rgb(55, 58, 64)
                                    },
                                ))
                                .rounding(Rounding::same(6.0))
                                .inner_margin(Margin::same(10.0))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(text)
                                            .monospace()
                                            .size(10.5)
                                            .color(if ok {
                                                Color32::from_rgb(240, 190, 145)
                                            } else {
                                                Color32::from_rgb(195, 198, 205)
                                            }),
                                    );
                                });

                            if ok {
                                if let Ok(preview) = qemu_display_preview.lock() {
                                    if let Some(image) = preview.as_ref() {
                                        ui.add_space(8.0);
                                        ui.label(
                                            RichText::new("Real QEMU framebuffer preview")
                                                .size(10.5)
                                                .strong()
                                                .color(Color32::from_rgb(180, 185, 195)),
                                        );
                                        let texture = ui.ctx().load_texture(
                                            "cdj3k-diagnostics-qemu-shm-preview",
                                            image.clone(),
                                            egui::TextureOptions::LINEAR,
                                        );
                                        let max_w = 512.0;
                                        let aspect =
                                            image.size[1] as f32 / image.size[0].max(1) as f32;
                                        ui.image((texture.id(), egui::vec2(max_w, max_w * aspect)));
                                    }
                                }
                            }
                        }

                        ui.add_space(8.0);
                        let live_snapshot = live_qemu_test.lock().unwrap().clone();
                        ui.horizontal(|ui| {
                            let run_live = Button::new(
                                RichText::new(if live_snapshot.running {
                                    "Running Live QEMU Test…"
                                } else {
                                    "Run Live QEMU Test"
                                })
                                .strong()
                                .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(35, 145, 125));

                            if ui.add_enabled(!live_snapshot.running, run_live).clicked() {
                                if let Ok(mut preview) = live_qemu_preview.lock() {
                                    *preview = None;
                                }
                                run_live_qemu_test(
                                    live_qemu_test.clone(),
                                    live_qemu_preview.clone(),
                                    ctx.clone(),
                                );
                            }

                            ui.label(
                                RichText::new("Real QEMU main.shm live animation (~30 FPS)")
                                    .size(10.5)
                                    .color(Color32::from_rgb(145, 150, 158)),
                            );
                        });

                        if live_snapshot.running || live_snapshot.result.is_some() {
                            ui.add_space(6.0);
                            let live_text = match &live_snapshot.result {
                                None if live_snapshot.running => "Running…".to_string(),
                                None => "Not run yet".to_string(),
                                Some(Ok(s)) => format!("PASS\n{s}"),
                                Some(Err(e)) => format!("FAIL\n{e}"),
                            };
                            let live_ok = matches!(&live_snapshot.result, Some(Ok(_)));

                            Frame::default()
                                .fill(Color32::from_rgb(14, 15, 17))
                                .stroke(Stroke::new(
                                    1.0,
                                    if live_ok {
                                        Color32::from_rgb(55, 155, 135)
                                    } else {
                                        Color32::from_rgb(55, 58, 64)
                                    },
                                ))
                                .rounding(Rounding::same(6.0))
                                .inner_margin(Margin::same(10.0))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(live_text)
                                            .monospace()
                                            .size(10.5)
                                            .color(if live_ok {
                                                Color32::from_rgb(155, 225, 205)
                                            } else {
                                                Color32::from_rgb(195, 198, 205)
                                            }),
                                    );
                                });

                            if let Ok(preview) = live_qemu_preview.lock() {
                                if let Some(image) = preview.as_ref() {
                                    ui.add_space(8.0);
                                    ui.label(
                                        RichText::new("Live QEMU shared-memory preview")
                                            .size(10.5)
                                            .strong()
                                            .color(Color32::from_rgb(180, 185, 195)),
                                    );
                                    let texture = ui.ctx().load_texture(
                                        "cdj3k-diagnostics-live-qemu-preview",
                                        image.clone(),
                                        egui::TextureOptions::LINEAR,
                                    );
                                    let max_w = 512.0;
                                    let aspect =
                                        image.size[1] as f32 / image.size[0].max(1) as f32;
                                    ui.image((texture.id(), egui::vec2(max_w, max_w * aspect)));
                                }
                            }
                        }

                        ui.add_space(8.0);
                        let stress_snapshot = qemu_stress_test.lock().unwrap().clone();
                        ui.horizontal(|ui| {
                            let run_stress = Button::new(
                                RichText::new(if stress_snapshot.running {
                                    "Running 5-Cycle Stress Test…"
                                } else {
                                    "Run 5-Cycle Stress Test"
                                })
                                .strong()
                                .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(165, 75, 75));

                            if ui.add_enabled(!stress_snapshot.running, run_stress).clicked() {
                                run_qemu_stress_test(qemu_stress_test.clone(), ctx.clone());
                            }

                            ui.label(
                                RichText::new("Launch/stop QEMU 5× and verify cleanup")
                                    .size(10.5)
                                    .color(Color32::from_rgb(145, 150, 158)),
                            );
                        });

                        if stress_snapshot.running || stress_snapshot.result.is_some() {
                            ui.add_space(6.0);
                            let stress_text = match &stress_snapshot.result {
                                None if stress_snapshot.running => "Running…".to_string(),
                                None => "Not run yet".to_string(),
                                Some(Ok(s)) => format!("PASS\n{s}"),
                                Some(Err(e)) => format!("FAIL\n{e}"),
                            };
                            let stress_ok = matches!(&stress_snapshot.result, Some(Ok(_)));

                            Frame::default()
                                .fill(Color32::from_rgb(14, 15, 17))
                                .stroke(Stroke::new(
                                    1.0,
                                    if stress_ok {
                                        Color32::from_rgb(165, 85, 85)
                                    } else {
                                        Color32::from_rgb(55, 58, 64)
                                    },
                                ))
                                .rounding(Rounding::same(6.0))
                                .inner_margin(Margin::same(10.0))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(stress_text)
                                            .monospace()
                                            .size(10.5)
                                            .color(if stress_ok {
                                                Color32::from_rgb(235, 175, 175)
                                            } else {
                                                Color32::from_rgb(195, 198, 205)
                                            }),
                                    );
                                });
                        }

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


#[cfg(windows)]
fn run_qemu_display_test(
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
        .name("cdj3k-qemu-display-diagnostics".into())
        .spawn(move || {
            let result = run_qemu_display_test_inner(preview, ctx.clone());
            {
                let mut state = test.lock().unwrap();
                state.running = false;
                state.result = Some(result);
            }
            ctx.request_repaint();
        })
        .ok();
}

#[cfg(windows)]
fn run_qemu_display_test_inner(
    preview: Arc<Mutex<Option<egui::ColorImage>>>,
    ctx: Context,
) -> Result<String, String> {
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::Duration;

    let qemu = cdj3k_emu_runtime::external_qemu_exe()
        .ok_or_else(|| "bundled qemu-system-aarch64.exe not found".to_string())?;

    let dir = std::env::temp_dir().join("cdj3k-emu-qemu-displaytest");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("create {}: {e}", dir.display()))?;

    let shm = dir.join("main.shm");
    let log = dir.join("qemu-displaytest.log");
    let stdout_file = std::fs::File::create(&log)
        .map_err(|e| format!("create {}: {e}", log.display()))?;
    let stderr_file = stdout_file
        .try_clone()
        .map_err(|e| format!("clone QEMU log handle: {e}"))?;

    let display_arg = format!("shm,path={}", shm.display());
    let argv = vec![
        "-machine".to_string(), "virt".to_string(),
        "-accel".to_string(), "tcg".to_string(),
        "-cpu".to_string(), "cortex-a72".to_string(),
        "-m".to_string(), "128".to_string(),
        "-nodefaults".to_string(),
        "-device".to_string(), "virtio-gpu-pci".to_string(),
        "-display".to_string(), display_arg,
        "-monitor".to_string(), "none".to_string(),
        "-serial".to_string(), "none".to_string(),
        "-S".to_string(),
    ];

    let mut child = Command::new(&qemu)
        .args(&argv)
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .spawn()
        .map_err(|e| format!("spawn QEMU: {e}"))?;

    thread::sleep(Duration::from_secs(2));

    if let Ok(Some(status)) = child.try_wait() {
        let qemu_log = std::fs::read_to_string(&log).unwrap_or_default();
        return Err(format!("QEMU exited early with {status}\n{qemu_log}"));
    }

    let qemu_log = std::fs::read_to_string(&log).unwrap_or_default();
    if qemu_log.contains("no graphic console found") || qemu_log.contains("console -1") {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("QEMU did not create a valid graphics console\n{qemu_log}"));
    }

    let inspected =
        cdj3k_emu_streams::main_stream::inspect_qemu_main_shm(ctx, &dir);

    let _ = child.kill();
    let _ = child.wait();

    let r = inspected?;
    let expected = r.width as usize * r.height as usize * 4;
    if r.preview_rgba.len() != expected {
        return Err(format!(
            "captured framebuffer size mismatch: {} bytes, expected {}",
            r.preview_rgba.len(),
            expected
        ));
    }

    let image = egui::ColorImage::from_rgba_unmultiplied(
        [r.width as usize, r.height as usize],
        &r.preview_rgba,
    );
    if let Ok(mut slot) = preview.lock() {
        *slot = Some(image);
    }

    Ok(format!(
        "MainLcdStream connected: {}\nResolution: {}×{}\nStride: {} bytes\nGeneration: {}\nmain.shm size: {} bytes\nQEMU console: VALID\nVisible preview: READY",
        r.connected, r.width, r.height, r.stride, r.generation, r.shm_bytes
    ))
}

#[cfg(not(windows))]
fn run_qemu_display_test(
    test: Arc<Mutex<TestState>>,
    _preview: Arc<Mutex<Option<egui::ColorImage>>>,
    ctx: Context,
) {
    let mut state = test.lock().unwrap();
    state.running = false;
    state.result = Some(Ok("QEMU display integration test is Windows-only".to_string()));
    drop(state);
    ctx.request_repaint();
}


#[cfg(windows)]
fn run_live_qemu_test(
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
        .name("cdj3k-live-qemu-display".into())
        .spawn(move || {
            let result = run_live_qemu_test_inner(preview, ctx.clone());
            {
                let mut state = test.lock().unwrap();
                state.running = false;
                state.result = Some(result);
            }
            ctx.request_repaint();
        })
        .ok();
}

#[cfg(windows)]
fn run_live_qemu_test_inner(
    preview: Arc<Mutex<Option<egui::ColorImage>>>,
    ctx: Context,
) -> Result<String, String> {
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::Duration;

    let qemu = cdj3k_emu_runtime::external_qemu_exe()
        .ok_or_else(|| "bundled qemu-system-aarch64.exe not found".to_string())?;

    let dir = std::env::temp_dir().join("cdj3k-emu-live-qemu-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("create {}: {e}", dir.display()))?;

    let shm = dir.join("main.shm");
    let log = dir.join("qemu-live-display.log");
    let stdout_file = std::fs::File::create(&log)
        .map_err(|e| format!("create {}: {e}", log.display()))?;
    let stderr_file = stdout_file
        .try_clone()
        .map_err(|e| format!("clone QEMU log handle: {e}"))?;

    let display_arg = format!("shm,path={}", shm.display());
    let argv = vec![
        "-machine".to_string(), "virt".to_string(),
        "-accel".to_string(), "tcg".to_string(),
        "-cpu".to_string(), "cortex-a72".to_string(),
        "-m".to_string(), "128".to_string(),
        "-nodefaults".to_string(),
        "-device".to_string(), "virtio-gpu-pci".to_string(),
        "-display".to_string(), display_arg,
        "-monitor".to_string(), "none".to_string(),
        "-serial".to_string(), "none".to_string(),
        "-S".to_string(),
    ];

    let mut child = Command::new(&qemu)
        .args(&argv)
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .spawn()
        .map_err(|e| format!("spawn QEMU: {e}"))?;

    thread::sleep(Duration::from_secs(2));

    if let Ok(Some(status)) = child.try_wait() {
        let qemu_log = std::fs::read_to_string(&log).unwrap_or_default();
        return Err(format!("QEMU exited early with {status}\n{qemu_log}"));
    }

    let qemu_log = std::fs::read_to_string(&log).unwrap_or_default();
    if qemu_log.contains("no graphic console found") || qemu_log.contains("console -1") {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("QEMU did not create a valid graphics console\n{qemu_log}"));
    }

    let preview_for_frames = preview.clone();
    let ctx_for_frames = ctx.clone();
    let result = cdj3k_emu_streams::main_stream::run_live_qemu_shm_animation(
        ctx.clone(),
        &dir,
        90,
        move |w, h, rgba| {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [w as usize, h as usize],
                &rgba,
            );
            if let Ok(mut slot) = preview_for_frames.lock() {
                *slot = Some(image);
            }
            ctx_for_frames.request_repaint();
        },
    );

    let _ = child.kill();
    let _ = child.wait();

    let r = result?;
    Ok(format!(
        "Resolution: {}×{}\nStride: {} bytes\nFrames written: {}\nFrames observed: {}\nDirty notifications: {}\nDropped generations: {}\nElapsed: {} ms\nObserved FPS: {:.1}\nFrame interval avg/min/max: {:.2}/{:.2}/{:.2} ms\nQEMU console: VALID\nLive preview: READY",
        r.width,
        r.height,
        r.stride,
        r.frames_written,
        r.frames_observed,
        r.dirty_notifications,
        r.dropped_generations,
        r.duration_ms,
        r.observed_fps,
        r.avg_frame_interval_ms,
        r.min_frame_interval_ms,
        r.max_frame_interval_ms
    ))
}

#[cfg(not(windows))]
fn run_live_qemu_test(
    test: Arc<Mutex<TestState>>,
    _preview: Arc<Mutex<Option<egui::ColorImage>>>,
    ctx: Context,
) {
    let mut state = test.lock().unwrap();
    state.running = false;
    state.result = Some(Ok("Live QEMU display test is Windows-only".to_string()));
    drop(state);
    ctx.request_repaint();
}


#[cfg(windows)]
fn run_qemu_stress_test(test: Arc<Mutex<TestState>>, ctx: Context) {
    {
        let mut state = test.lock().unwrap();
        state.running = true;
        state.result = None;
    }

    std::thread::Builder::new()
        .name("cdj3k-qemu-stress-test".into())
        .spawn(move || {
            let result = run_qemu_stress_test_inner();
            {
                let mut state = test.lock().unwrap();
                state.running = false;
                state.result = Some(result);
            }
            ctx.request_repaint();
        })
        .ok();
}

#[cfg(windows)]
fn run_qemu_stress_test_inner() -> Result<String, String> {
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    let qemu = cdj3k_emu_runtime::external_qemu_exe()
        .ok_or_else(|| "bundled qemu-system-aarch64.exe not found".to_string())?;

    let mut cycle_lines = Vec::new();
    let overall = Instant::now();

    for cycle in 1..=5 {
        let dir = std::env::temp_dir().join(format!("cdj3k-emu-stress-{cycle}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cycle {cycle}: create {}: {e}", dir.display()))?;

        let shm = dir.join("main.shm");
        let log = dir.join("qemu.log");
        let stdout_file = std::fs::File::create(&log)
            .map_err(|e| format!("cycle {cycle}: create log: {e}"))?;
        let stderr_file = stdout_file
            .try_clone()
            .map_err(|e| format!("cycle {cycle}: clone log: {e}"))?;

        let display_arg = format!("shm,path={}", shm.display());
        let started = Instant::now();
        let mut child = Command::new(&qemu)
            .args([
                "-machine", "virt",
                "-accel", "tcg",
                "-cpu", "cortex-a72",
                "-m", "128",
                "-nodefaults",
                "-device", "virtio-gpu-pci",
                "-display", &display_arg,
                "-monitor", "none",
                "-serial", "none",
                "-S",
            ])
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file))
            .spawn()
            .map_err(|e| format!("cycle {cycle}: spawn QEMU: {e}"))?;

        thread::sleep(Duration::from_millis(900));

        if let Ok(Some(status)) = child.try_wait() {
            let qemu_log = std::fs::read_to_string(&log).unwrap_or_default();
            return Err(format!(
                "cycle {cycle}: QEMU exited early with {status}\n{qemu_log}"
            ));
        }

        if !shm.is_file() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("cycle {cycle}: main.shm was not created"));
        }

        let qemu_log = std::fs::read_to_string(&log).unwrap_or_default();
        if qemu_log.contains("no graphic console found") || qemu_log.contains("console -1") {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "cycle {cycle}: invalid graphical console\n{qemu_log}"
            ));
        }

        child.kill()
            .map_err(|e| format!("cycle {cycle}: kill QEMU: {e}"))?;
        let status = child.wait()
            .map_err(|e| format!("cycle {cycle}: wait QEMU: {e}"))?;

        thread::sleep(Duration::from_millis(120));

        match child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => return Err(format!("cycle {cycle}: QEMU process remained alive")),
            Err(e) => return Err(format!("cycle {cycle}: process status check failed: {e}")),
        }

        let elapsed = started.elapsed().as_millis();
        cycle_lines.push(format!(
            "Cycle {cycle}: PASS ({} ms, exit {})",
            elapsed,
            status.code().map(|x| x.to_string()).unwrap_or_else(|| "terminated".into())
        ));

        // Remove test runtime directory to validate cleanup is possible.
        std::fs::remove_dir_all(&dir)
            .map_err(|e| format!("cycle {cycle}: cleanup {}: {e}", dir.display()))?;
        if dir.exists() {
            return Err(format!("cycle {cycle}: runtime directory still exists after cleanup"));
        }
    }

    let total_ms = overall.elapsed().as_millis();
    cycle_lines.push(format!("All 5 cycles passed in {total_ms} ms"));
    cycle_lines.push("No orphan child process detected by owned process handles".to_string());
    cycle_lines.push("Runtime directories cleaned successfully".to_string());

    Ok(cycle_lines.join("\n"))
}

#[cfg(not(windows))]
fn run_qemu_stress_test(test: Arc<Mutex<TestState>>, ctx: Context) {
    let mut state = test.lock().unwrap();
    state.running = false;
    state.result = Some(Ok("QEMU stress test is Windows-only".to_string()));
    drop(state);
    ctx.request_repaint();
}
