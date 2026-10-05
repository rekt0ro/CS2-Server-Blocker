use eframe::egui::{self, Align, Color32, Rect, RichText, ScrollArea, Sense, Stroke, TextEdit};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

const SDR_URL: &str = "https://api.steampowered.com/ISteamApps/GetSDRConfig/v1/?appid=730";
const STATE_FILE: &str = "state.json";
const UFW_COMMENT: &str = "CS2-Server-Blocker";
const FIREWALL_COMMENT: &str = "CS2-Server-Blocker";
const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/rekt0ro/CS2-Server-Blocker/releases/latest";
const LINUX_RELEASE_ASSET: &str = "cs2-server-blocker-x86_64-linux.tar.gz";
const INSTALLED_BINARY: &str = "/usr/local/bin/cs2-server-blocker";
const INSTALLED_DESKTOP_ENTRY: &str = "/usr/share/applications/cs2-server-blocker.desktop";
const INSTALLED_ICON: &str = "/usr/share/icons/hicolor/scalable/apps/cs2-server-blocker.svg";

#[derive(Clone, Debug)]
struct Pop {
    code: String,
    location: String,
    country: String,
    relays: Vec<String>,
}

#[derive(Clone, Debug)]
struct CountryGroup {
    country: String,
    pops: Vec<Pop>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum FirewallBackend {
    Ufw,
    Firewalld,
    Nftables,
    Iptables,
}

impl FirewallBackend {
    fn label(self) -> &'static str {
        match self {
            Self::Ufw => "UFW",
            Self::Firewalld => "Firewalld",
            Self::Nftables => "nftables",
            Self::Iptables => "iptables",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct StoredState {
    backend: Option<FirewallBackend>,
    blocked_by_pop: HashMap<String, Vec<String>>,
}

impl StoredState {
    fn blocked_ips(&self) -> HashSet<String> {
        self.blocked_by_pop.values().flatten().cloned().collect()
    }

    fn ips_for_pop(&self, code: &str) -> Vec<String> {
        self.blocked_by_pop.get(code).cloned().unwrap_or_default()
    }

    fn still_referenced_elsewhere(&self, ip: &str, except: &str) -> bool {
        self.blocked_by_pop
            .iter()
            .any(|(code, ips)| code != except && ips.iter().any(|candidate| candidate == ip))
    }
}

enum WorkerResult {
    Refreshed(Result<Vec<Pop>, String>),
    Updated(Result<UpdateOutcome, String>),
    FirewallAction(Result<ActionReport, String>),
}

#[derive(Debug)]
struct ActionReport {
    action: &'static str,
    pop_count: usize,
    ip_count: usize,
    backend: FirewallBackend,
}

#[derive(Clone, Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Clone, Debug, Deserialize)]
struct GithubReleaseAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct AppVersion {
    major: u64,
    minor: u64,
    patch: u64,
}

impl AppVersion {
    fn parse(value: &str) -> Option<Self> {
        let version = value.trim().strip_prefix('v').unwrap_or(value.trim());
        let mut parts = version.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.split(['-', '+']).next()?.parse().ok()?;

        if parts.next().is_some() {
            return None;
        }

        Some(Self {
            major,
            minor,
            patch,
        })
    }
}

#[derive(Debug)]
enum UpdateOutcome {
    UpToDate { version: AppVersion },
    Updated { version: AppVersion },
}

#[derive(Default)]
struct App {
    pops: Vec<Pop>,
    selected: HashSet<String>,
    stored: StoredState,
    backend: Option<FirewallBackend>,
    search: String,
    log: Vec<String>,
    busy: bool,
    error: Option<String>,
    worker_rx: Option<Receiver<WorkerResult>>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_pixels_per_point(1.0);
        egui_system_fonts::set_auto(&cc.egui_ctx, egui_system_fonts::FontStyle::Sans);
        let mut visuals = egui::Visuals::dark();
        visuals.window_fill = Color32::from_rgb(13, 18, 27);
        visuals.panel_fill = Color32::from_rgb(13, 18, 27);
        visuals.extreme_bg_color = Color32::from_rgb(9, 13, 20);
        visuals.faint_bg_color = Color32::from_rgb(27, 35, 48);
        visuals.text_edit_bg_color = Some(Color32::from_rgb(15, 22, 32));
        visuals.selection.bg_fill = Color32::from_rgb(35, 92, 150);
        visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(77, 166, 255));
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(26, 34, 47);
        visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(22, 30, 42);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(35, 50, 68);
        visuals.widgets.active.bg_fill = Color32::from_rgb(40, 86, 126);
        visuals.widgets.open.bg_fill = Color32::from_rgb(35, 50, 68);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(48, 60, 77));
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(77, 166, 255));
        visuals.window_stroke = Stroke::new(1.0, Color32::from_rgb(43, 54, 70));
        visuals.collapsing_header_frame = false;
        cc.egui_ctx.set_visuals(visuals);

        let mut style = (*cc.egui_ctx.style_of(egui::Theme::Dark)).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 8.0);
        style.spacing.interact_size.y = 34.0;
        cc.egui_ctx.set_style_of(egui::Theme::Dark, style);

        let stored = load_state();
        let backend = detect_firewall();
        let mut app = Self {
            stored,
            backend,
            ..Default::default()
        };
        app.log
            .push("Ready. Fetch the current Steam SDR relay list.".into());
        app.start_refresh();
        app
    }

    fn start_refresh(&mut self) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let (tx, rx) = mpsc::channel();
        self.worker_rx = Some(rx);
        thread::spawn(move || {
            let result = fetch_pops();
            let _ = tx.send(WorkerResult::Refreshed(result));
        });
    }

    fn start_update_check(&mut self) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let (tx, rx) = mpsc::channel();
        self.worker_rx = Some(rx);
        thread::spawn(move || {
            let result = check_for_updates_and_install();
            let _ = tx.send(WorkerResult::Updated(result));
        });
    }

    fn start_action(&mut self, task: ActionTask) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let stored = self.stored.clone();
        let detected = self.backend;
        let pops = self.pops.clone();
        let selected = self.selected.clone();
        let (tx, rx) = mpsc::channel();
        self.worker_rx = Some(rx);
        thread::spawn(move || {
            let result = perform_action(task, stored, detected, pops, selected);
            let _ = tx.send(WorkerResult::FirewallAction(result));
        });
    }

    fn process_worker(&mut self) {
        let Some(rx) = &self.worker_rx else { return };
        let Ok(message) = rx.try_recv() else { return };
        self.busy = false;
        self.worker_rx = None;

        match message {
            WorkerResult::Refreshed(result) => match result {
                Ok(pops) => {
                    self.log
                        .push(format!("Loaded {} Steam SDR PoPs.", pops.len()));
                    self.pops = pops;
                    self.retain_valid_selection();
                }
                Err(err) => {
                    self.error = Some(err.clone());
                    self.log.push(format!("Refresh failed: {err}"));
                }
            },
            WorkerResult::Updated(result) => match result {
                Ok(UpdateOutcome::UpToDate { version }) => {
                    self.log.push(format!(
                        "You're already on the latest version, v{}. ",
                        format_version(version)
                    ));
                }
                Ok(UpdateOutcome::Updated { version }) => {
                    self.log.push(format!(
                        "Updated to v{}. Restart the app to use the new version.",
                        format_version(version)
                    ));
                }
                Err(err) => {
                    self.error = Some(err.clone());
                    self.log.push(format!("Update failed: {err}"));
                }
            },
            WorkerResult::FirewallAction(result) => match result {
                Ok(report) => {
                    self.log.push(format!(
                        "{} {} PoP(s), {} IP(s) via {}.",
                        report.action,
                        report.pop_count,
                        report.ip_count,
                        report.backend.label()
                    ));
                    self.stored = load_state();
                    self.backend = detect_firewall();
                }
                Err(err) => {
                    self.error = Some(err.clone());
                    self.log.push(format!("Firewall action failed: {err}"));
                }
            },
        }
    }

    fn retain_valid_selection(&mut self) {
        let valid: HashSet<String> = self.pops.iter().map(|p| p.code.clone()).collect();
        self.selected.retain(|code| valid.contains(code));
    }

    fn matches_search(&self, pop: &Pop) -> bool {
        let needle = self.search.trim().to_lowercase();
        needle.is_empty()
            || pop.code.to_lowercase().contains(&needle)
            || pop.country.to_lowercase().contains(&needle)
            || pop.location.to_lowercase().contains(&needle)
    }

    fn country_count(&self) -> usize {
        self.pops
            .iter()
            .map(|pop| pop.country.as_str())
            .collect::<HashSet<_>>()
            .len()
    }

    fn visible_country_groups(&self) -> Vec<CountryGroup> {
        let mut groups: BTreeMap<String, CountryGroup> = BTreeMap::new();
        for pop in &self.pops {
            groups
                .entry(pop.country.clone())
                .or_insert_with(|| CountryGroup {
                    country: pop.country.clone(),
                    pops: Vec::new(),
                })
                .pops
                .push(pop.clone());
        }

        let mut groups: Vec<CountryGroup> = groups.into_values().collect();
        for group in &mut groups {
            group.pops.sort_by(|a, b| {
                a.location
                    .cmp(&b.location)
                    .then_with(|| a.code.cmp(&b.code))
            });
        }

        groups
            .into_iter()
            .filter(|group| {
                self.search.trim().is_empty()
                    || group
                        .country
                        .to_lowercase()
                        .contains(&self.search.trim().to_lowercase())
                    || group.pops.iter().any(|pop| self.matches_search(pop))
            })
            .collect()
    }

    fn set_country_selected(&mut self, pops: &[Pop], selected: bool) {
        for pop in pops {
            if selected {
                self.selected.insert(pop.code.clone());
            } else {
                self.selected.remove(&pop.code);
            }
        }
    }

    fn select_all(&mut self) {
        self.selected = self.pops.iter().map(|pop| pop.code.clone()).collect();
    }

    fn unselect_all(&mut self) {
        self.selected.clear();
    }

    fn block_selected(&mut self) {
        if self.selected.is_empty() {
            self.error = Some("Select at least one PoP first.".into());
            return;
        }
        self.start_action(ActionTask::BlockSelected);
    }

    fn unblock_selected(&mut self) {
        if self.selected.is_empty() {
            self.error = Some("Select at least one PoP first.".into());
            return;
        }
        self.start_action(ActionTask::UnblockSelected);
    }

    fn unblock_all(&mut self) {
        if self.stored.blocked_ips().is_empty() {
            self.log.push("Nothing to unblock.".into());
            return;
        }
        self.start_action(ActionTask::UnblockAll);
    }
}

#[derive(Clone, Copy)]
enum ActionTask {
    BlockSelected,
    UnblockSelected,
    UnblockAll,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.process_worker();
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));

        let bg = Color32::from_rgb(10, 14, 22);
        let surface = Color32::from_rgb(17, 24, 35);
        let surface_2 = Color32::from_rgb(22, 31, 45);
        let surface_3 = Color32::from_rgb(27, 38, 54);
        let border = Color32::from_rgb(42, 55, 73);
        let text = Color32::from_rgb(238, 243, 249);
        let muted = Color32::from_rgb(142, 157, 178);
        let accent = Color32::from_rgb(76, 166, 255);
        let success = Color32::from_rgb(68, 204, 140);
        let danger = Color32::from_rgb(240, 92, 106);
        let warning = Color32::from_rgb(244, 181, 72);

        let total_width = ui.available_width().min(1120.0);
        let content_width = (total_width - 32.0).max(0.0);
        let side_margin = ((ui.available_width() - total_width) * 0.5).max(0.0);
        let blocked_ips = self.stored.blocked_ips();
        let groups = self.visible_country_groups();
        let current_log = self.log.last().cloned().unwrap_or_else(|| "Ready.".into());

        egui::Frame::new()
            .fill(bg)
            .inner_margin(16)
            .outer_margin(egui::vec2(side_margin, 0.0))
            .show(ui, |ui| {
                egui::Frame::new()
                    .fill(surface)
                    .stroke(Stroke::new(1.0, border))
                    .corner_radius(16.0)
                    .inner_margin(14)
                    .show(ui, |ui| {
                        ui.set_width((content_width - 28.0).max(0.0));
                        ui.horizontal(|ui| {
                            draw_app_mark(ui, accent);
                            ui.add_space(12.0);
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new("CS2 Server Blocker")
                                            .strong()
                                            .size(21.0)
                                            .color(text),
                                    );
                                    ui.add_space(7.0);
                                    ui.label(
                                        RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                                            .size(11.0)
                                            .strong()
                                            .color(muted),
                                    );
                                });
                                ui.add_space(2.0);
                                ui.label(
                                    RichText::new(
                                        "Steam SDR relay control  ·  Fast region selection and firewall control",
                                    )
                                    .size(12.0)
                                    .color(muted),
                                );
                            });

                            ui.with_layout(
                                egui::Layout::right_to_left(Align::Center),
                                |ui| {
                                    ui.add_enabled_ui(!self.busy, |ui| {
                                        ui.vertical(|ui| {
                                            if ui
                                                .add_sized(
                                                    egui::vec2(152.0, 26.0),
                                                    egui::Button::new(
                                                        RichText::new("Refresh Data").strong().size(12.0),
                                                    ),
                                                )
                                                .clicked()
                                            {
                                                self.start_refresh();
                                            }
                                            ui.add_space(4.0);
                                            if ui
                                                .add_sized(
                                                    egui::vec2(152.0, 26.0),
                                                    egui::Button::new(
                                                        RichText::new("Check for Updates")
                                                            .strong()
                                                            .size(12.0),
                                                    ),
                                                )
                                                .clicked()
                                            {
                                                self.start_update_check();
                                            }
                                        });
                                    });
                                },
                            );
                        });
                    });

                ui.add_space(10.0);

                ui.allocate_ui_with_layout(
                    egui::vec2(content_width, 66.0),
                    egui::Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.columns(4, |columns| {
                            let width = columns[0].available_width();
                            metric_card(
                                &mut columns[0],
                                "FIREWALL",
                                self.backend.map(|b| b.label()).unwrap_or("Not detected"),
                                self.backend.is_some(),
                                success,
                                width,
                            );

                            let width = columns[1].available_width();
                            metric_card(
                                &mut columns[1],
                                "SELECTED",
                                format!("{} PoPs", self.selected.len()).as_str(),
                                !self.selected.is_empty(),
                                accent,
                                width,
                            );

                            let width = columns[2].available_width();
                            metric_card(
                                &mut columns[2],
                                "BLOCKED IPS",
                                blocked_ips.len().to_string().as_str(),
                                !blocked_ips.is_empty(),
                                danger,
                                width,
                            );

                            let width = columns[3].available_width();
                            metric_card(
                                &mut columns[3],
                                "NETWORK DATA",
                                format!(
                                    "{} countries  ·  {} PoPs",
                                    self.country_count(),
                                    self.pops.len()
                                )
                                .as_str(),
                                !self.pops.is_empty(),
                                accent,
                                width,
                            );
                        });
                    },
                );

                ui.add_space(10.0);

                egui::Frame::new()
                    .fill(surface)
                    .stroke(Stroke::new(1.0, border))
                    .corner_radius(14.0)
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.set_width((content_width - 24.0).max(0.0));

                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new("SERVER REGIONS")
                                        .strong()
                                        .size(11.0)
                                        .color(muted),
                                );
                                let status_text = if self.busy {
                                    "Working..."
                                } else if self.pops.is_empty() {
                                    "Waiting for relay data"
                                } else {
                                    "Live relay configuration"
                                };
                                let status_color = if self.busy {
                                    warning
                                } else if self.pops.is_empty() {
                                    muted
                                } else {
                                    success
                                };
                                ui.add_space(2.0);
                                status_badge(ui, status_text, status_color);
                            });

                            ui.with_layout(
                                egui::Layout::right_to_left(Align::Center),
                                |ui| {
                                    let selected_text = format!("{} selected", self.selected.len());
                                    let blocked_text = format!("{} blocked", blocked_ips.len());
                                    status_badge(ui, &selected_text, accent);
                                    ui.add_space(6.0);
                                    status_badge(ui, &blocked_text, danger);
                                },
                            );
                        });

                        ui.add_space(10.0);

                        ui.horizontal(|ui| {
                            let search_width = (ui.available_width() - 98.0).max(160.0);
                            ui.add_sized(
                                egui::vec2(search_width, 34.0),
                                TextEdit::singleline(&mut self.search)
                                    .hint_text("Search country, city, or PoP code"),
                            );
                            ui.add_space(6.0);
                            if ui
                                .add_enabled(
                                    !self.search.is_empty(),
                                    egui::Button::new(RichText::new("Clear").size(12.0)),
                                )
                                .clicked()
                            {
                                self.search.clear();
                            }
                        });

                        ui.add_space(9.0);

                        ui.horizontal_wrapped(|ui| {
                            ui.add_enabled_ui(!self.busy && !self.pops.is_empty(), |ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("Select All").strong().size(12.0),
                                        )
                                        .min_size(egui::vec2(106.0, 30.0)),
                                    )
                                    .clicked()
                                {
                                    self.select_all();
                                }
                                if ui
                                    .add_sized(
                                        egui::vec2(106.0, 30.0),
                                        egui::Button::new(
                                            RichText::new("Unselect All").size(12.0),
                                        ),
                                    )
                                    .clicked()
                                {
                                    self.unselect_all();
                                }
                            });

                            ui.add_space(6.0);
                            ui.separator();
                            ui.add_space(6.0);

                            ui.add_enabled_ui(!self.busy, |ui| {
                                if ui
                                    .add_sized(
                                        egui::vec2(122.0, 30.0),
                                        egui::Button::new(
                                            RichText::new("Block Selected")
                                                .strong()
                                                .color(accent)
                                                .size(12.0),
                                        ),
                                    )
                                    .clicked()
                                {
                                    self.block_selected();
                                }
                                if ui
                                    .add_sized(
                                        egui::vec2(132.0, 30.0),
                                        egui::Button::new(
                                            RichText::new("Unblock Selected")
                                                .strong()
                                                .color(danger)
                                                .size(12.0),
                                        ),
                                    )
                                    .clicked()
                                {
                                    self.unblock_selected();
                                }
                                if ui
                                    .add_sized(
                                        egui::vec2(108.0, 30.0),
                                        egui::Button::new(
                                            RichText::new("Unblock All")
                                                .strong()
                                                .color(danger)
                                                .size(12.0),
                                        ),
                                    )
                                    .clicked()
                                {
                                    self.unblock_all();
                                }
                            });
                        });
                    });

                ui.add_space(10.0);

                egui::Frame::new()
                    .fill(surface)
                    .stroke(Stroke::new(1.0, border))
                    .corner_radius(14.0)
                    .inner_margin(8)
                    .show(ui, |ui| {
                        ui.set_width((content_width - 16.0).max(0.0));

                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("COUNTRIES")
                                    .strong()
                                    .size(11.0)
                                    .color(muted),
                            );
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new(format!("{} matches", groups.len()))
                                    .size(11.0)
                                    .color(muted),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(Align::Center),
                                |ui| {
                                    if !self.selected.is_empty() {
                                        status_badge(
                                            ui,
                                            &format!("{} selected", self.selected.len()),
                                            accent,
                                        );
                                    }
                                },
                            );
                        });

                        ui.add_space(6.0);

                        if groups.is_empty() {
                            ui.add_space(26.0);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new("No matching regions")
                                        .strong()
                                        .size(17.0)
                                        .color(text),
                                );
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new("Try a country, city, or PoP code.")
                                        .size(12.0)
                                        .color(muted),
                                );
                            });
                            ui.add_space(26.0);
                        } else {
                            ScrollArea::vertical()
                                .id_salt("country_pop_tree")
                                .auto_shrink([false, false])
                                .max_height(390.0)
                                .show(ui, |ui| {
                                    for (index, group) in groups.iter().enumerate() {
                                        let total = group.pops.len();
                                        let selected_count = group
                                            .pops
                                            .iter()
                                            .filter(|pop| self.selected.contains(&pop.code))
                                            .count();
                                        let blocked_count = group
                                            .pops
                                            .iter()
                                            .filter(|pop| {
                                                pop.relays.iter().any(|ip| blocked_ips.contains(ip))
                                            })
                                            .count();
                                        let fully_selected = selected_count == total;
                                        let partially_selected =
                                            selected_count > 0 && selected_count < total;

                                        let state =
                                            egui::collapsing_header::CollapsingState::load_with_default_open(
                                                ui.ctx(),
                                                ui.make_persistent_id(("country", &group.country)),
                                                false,
                                            );

                                        state
                                            .show_header(ui, |ui| {
                                                let row_rect = ui.available_rect_before_wrap();
                                                let row_response = ui.interact(
                                                    row_rect,
                                                    ui.make_persistent_id((
                                                        "country-row",
                                                        &group.country,
                                                    )),
                                                    Sense::hover(),
                                                );
                                                if row_response.hovered() {
                                                    ui.painter().rect_filled(
                                                        row_rect,
                                                        9.0,
                                                        surface_3,
                                                    );
                                                }

                                                let mut select_country = fully_selected;
                                                let checkbox = ui.checkbox(&mut select_country, "");
                                                if checkbox.changed() {
                                                    self.set_country_selected(
                                                        &group.pops,
                                                        select_country,
                                                    );
                                                }

                                                draw_flag(ui, &group.country);
                                                ui.add_space(7.0);
                                                ui.vertical(|ui| {
                                                    ui.horizontal(|ui| {
                                                        ui.label(
                                                            RichText::new(&group.country)
                                                                .strong()
                                                                .size(15.0)
                                                                .color(text),
                                                        );
                                                        ui.add_space(6.0);
                                                        ui.label(
                                                            RichText::new(format!("{} PoPs", total))
                                                                .size(11.0)
                                                                .color(muted),
                                                        );
                                                    });
                                                    if partially_selected {
                                                        ui.label(
                                                            RichText::new("Partially selected")
                                                                .size(10.0)
                                                                .color(warning),
                                                        );
                                                    }
                                                });

                                                ui.with_layout(
                                                    egui::Layout::right_to_left(Align::Center),
                                                    |ui| {
                                                        if blocked_count > 0 {
                                                            status_badge(
                                                                ui,
                                                                &format!("{} blocked", blocked_count),
                                                                danger,
                                                            );
                                                        }
                                                        if selected_count > 0 {
                                                            ui.add_space(5.0);
                                                            status_badge(
                                                                ui,
                                                                &format!("{} selected", selected_count),
                                                                accent,
                                                            );
                                                        }
                                                    },
                                                );
                                            })
                                            .body(|ui| {
                                                ui.add_space(2.0);
                                                for pop in &group.pops {
                                                    if !self.matches_search(pop) {
                                                        continue;
                                                    }
                                                    let is_selected =
                                                        self.selected.contains(&pop.code);
                                                    let is_blocked = pop
                                                        .relays
                                                        .iter()
                                                        .any(|ip| blocked_ips.contains(ip));

                                                    let fill = if is_blocked {
                                                        Color32::from_rgb(40, 29, 36)
                                                    } else if is_selected {
                                                        Color32::from_rgb(27, 43, 60)
                                                    } else {
                                                        surface_2
                                                    };

                                                    egui::Frame::new()
                                                        .fill(fill)
                                                        .stroke(Stroke::new(
                                                            1.0,
                                                            if is_blocked {
                                                                Color32::from_rgb(95, 49, 61)
                                                            } else {
                                                                border
                                                            },
                                                        ))
                                                        .corner_radius(10.0)
                                                        .inner_margin(
                                                            egui::Margin::symmetric(9, 7),
                                                        )
                                                        .show(ui, |ui| {
                                                            ui.horizontal(|ui| {
                                                                ui.add_space(14.0);
                                                                let mut selected = is_selected;
                                                                if ui.checkbox(&mut selected, "").changed()
                                                                {
                                                                    if selected {
                                                                        self.selected.insert(
                                                                            pop.code.clone(),
                                                                        );
                                                                    } else {
                                                                        self.selected.remove(&pop.code);
                                                                    }
                                                                }

                                                                ui.vertical(|ui| {
                                                                    ui.horizontal(|ui| {
                                                                        ui.label(
                                                                            RichText::new(&pop.location)
                                                                                .strong()
                                                                                .size(13.0)
                                                                                .color(text),
                                                                        );
                                                                        ui.add_space(7.0);
                                                                        ui.label(
                                                                            RichText::new(
                                                                                pop.code.to_uppercase(),
                                                                            )
                                                                            .size(10.0)
                                                                            .strong()
                                                                            .color(accent),
                                                                        );
                                                                    });
                                                                    ui.add_space(2.0);
                                                                    ui.label(
                                                                        RichText::new(format!(
                                                                            "{} relay address{}",
                                                                            pop.relays.len(),
                                                                            if pop.relays.len() == 1 {
                                                                                ""
                                                                            } else {
                                                                                "es"
                                                                            }
                                                                        ))
                                                                        .size(10.0)
                                                                        .color(muted),
                                                                    );
                                                                });

                                                                ui.with_layout(
                                                                    egui::Layout::right_to_left(Align::Center),
                                                                    |ui| {
                                                                        if is_blocked {
                                                                            status_badge(
                                                                                ui,
                                                                                "BLOCKED",
                                                                                danger,
                                                                            );
                                                                        }
                                                                    },
                                                                );
                                                            });
                                                        });
                                                    ui.add_space(4.0);
                                                }
                                            });

                                        if index + 1 < groups.len() {
                                            ui.add_space(3.0);
                                        }
                                    }
                                });
                        }
                    });

                ui.add_space(10.0);

                if let Some(error) = &self.error {
                    egui::Frame::new()
                        .fill(Color32::from_rgb(55, 25, 32))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(124, 49, 61)))
                        .corner_radius(11.0)
                        .inner_margin(10)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("ERROR")
                                        .strong()
                                        .size(10.0)
                                        .color(danger),
                                );
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(error)
                                        .size(11.0)
                                        .color(Color32::from_rgb(255, 182, 190)),
                                );
                            });
                        });
                    ui.add_space(8.0);
                }

                egui::Frame::new()
                    .fill(Color32::from_rgb(14, 20, 29))
                    .stroke(Stroke::new(1.0, border))
                    .corner_radius(11.0)
                    .inner_margin(9)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("ACTIVITY")
                                    .strong()
                                    .size(10.0)
                                    .color(muted),
                            );
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new(current_log)
                                    .size(11.0)
                                    .color(text),
                            );
                        });
                    });
            });
    }
}

fn status_badge(ui: &mut egui::Ui, label: &str, accent: Color32) {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(
            accent.r(),
            accent.g(),
            accent.b(),
            28,
        ))
        .stroke(Stroke::new(
            1.0,
            Color32::from_rgba_unmultiplied(
                accent.r(),
                accent.g(),
                accent.b(),
                85,
            ),
        ))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(7, 3))
        .show(ui, |ui| {
            ui.label(
                RichText::new(label)
                    .strong()
                    .size(10.0)
                    .color(accent),
            );
        });
}

fn draw_app_mark(ui: &mut egui::Ui, accent: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(46.0, 46.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 11.0, Color32::from_rgb(28, 58, 88));
    painter.rect_filled(
        Rect::from_min_max(
            rect.min + egui::vec2(11.0, 9.0),
            rect.max - egui::vec2(11.0, 11.0),
        ),
        5.0,
        accent,
    );
    painter.rect_filled(
        Rect::from_min_max(
            rect.min + egui::vec2(15.0, 14.0),
            rect.max - egui::vec2(15.0, 25.0),
        ),
        2.0,
        Color32::from_rgb(13, 18, 27),
    );
    painter.rect_filled(
        Rect::from_min_max(
            rect.min + egui::vec2(15.0, 24.0),
            rect.max - egui::vec2(15.0, 15.0),
        ),
        2.0,
        Color32::from_rgb(13, 18, 27),
    );
}

fn metric_card(
    ui: &mut egui::Ui,
    title: &str,
    value: &str,
    emphasized: bool,
    accent: Color32,
    width: f32,
) {
    let bg = if emphasized {
        Color32::from_rgb(23, 33, 47)
    } else {
        Color32::from_rgb(17, 24, 35)
    };

    egui::Frame::new()
        .fill(bg)
        .stroke(Stroke::new(1.0, Color32::from_rgb(39, 51, 69)))
        .corner_radius(11.0)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_width((width - 8.0).max(0.0));
            ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                let (dot, _) = ui.allocate_exact_size(egui::vec2(5.0, 34.0), Sense::hover());
                ui.painter().rect_filled(dot, 3.0, accent);
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(title)
                            .size(9.0)
                            .strong()
                            .color(Color32::from_rgb(125, 142, 163)),
                    );
                    ui.add_space(1.0);
                    ui.label(
                        RichText::new(value)
                            .size(if value.len() > 22 { 13.0 } else { 16.0 })
                            .strong()
                            .color(Color32::from_rgb(237, 242, 248)),
                    );
                });
            });
        });
}

fn draw_flag(ui: &mut egui::Ui, country: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(30.0, 20.0), Sense::hover());
    let outer = rect;
    let r = rect.shrink(1.0);
    let painter = ui.painter();
    painter.rect_filled(outer, 4.0, Color32::from_rgb(7, 10, 15));

    match country {
        "Germany" => flag_h3(
            painter,
            r,
            Color32::BLACK,
            Color32::from_rgb(221, 0, 0),
            Color32::from_rgb(255, 206, 0),
        ),
        "Netherlands" => flag_h3(
            painter,
            r,
            Color32::from_rgb(174, 28, 40),
            Color32::WHITE,
            Color32::from_rgb(33, 70, 139),
        ),
        "France" => flag_v3(
            painter,
            r,
            Color32::from_rgb(35, 72, 138),
            Color32::WHITE,
            Color32::from_rgb(239, 65, 66),
        ),
        "Italy" => flag_v3(
            painter,
            r,
            Color32::from_rgb(0, 146, 70),
            Color32::WHITE,
            Color32::from_rgb(206, 43, 55),
        ),
        "Ireland" => flag_v3(
            painter,
            r,
            Color32::from_rgb(22, 155, 98),
            Color32::WHITE,
            Color32::from_rgb(255, 136, 62),
        ),
        "Romania" => flag_v3(
            painter,
            r,
            Color32::from_rgb(0, 43, 127),
            Color32::from_rgb(252, 209, 22),
            Color32::from_rgb(206, 17, 38),
        ),
        "Belgium" => flag_v3(
            painter,
            r,
            Color32::BLACK,
            Color32::from_rgb(250, 205, 8),
            Color32::from_rgb(239, 51, 64),
        ),
        "United Kingdom" => flag_uk(painter, r),
        "Spain" => flag_h3(
            painter,
            r,
            Color32::from_rgb(198, 0, 43),
            Color32::from_rgb(255, 196, 0),
            Color32::from_rgb(198, 0, 43),
        ),
        "Portugal" => flag_portugal(painter, r),
        "Poland" => flag_h2(painter, r, Color32::WHITE, Color32::from_rgb(220, 20, 60)),
        "Czechia" => flag_czech(painter, r),
        "Austria" => flag_h3(
            painter,
            r,
            Color32::from_rgb(237, 41, 57),
            Color32::WHITE,
            Color32::from_rgb(237, 41, 57),
        ),
        "Switzerland" => flag_cross(painter, r, Color32::from_rgb(218, 41, 28), Color32::WHITE),
        "Denmark" => flag_cross(painter, r, Color32::from_rgb(198, 12, 48), Color32::WHITE),
        "Sweden" => flag_cross(
            painter,
            r,
            Color32::from_rgb(0, 106, 167),
            Color32::from_rgb(254, 204, 0),
        ),
        "Finland" => flag_cross(painter, r, Color32::WHITE, Color32::from_rgb(0, 53, 128)),
        "Norway" => flag_norway(painter, r),
        "Bulgaria" => flag_h3(
            painter,
            r,
            Color32::WHITE,
            Color32::from_rgb(0, 150, 110),
            Color32::from_rgb(210, 38, 48),
        ),
        "Greece" => flag_greece(painter, r),
        "Türkiye" => flag_turkey(painter, r),
        "United States" => flag_usa(painter, r),
        "Canada" => flag_canada(painter, r),
        "Mexico" => flag_v3(
            painter,
            r,
            Color32::from_rgb(0, 104, 71),
            Color32::WHITE,
            Color32::from_rgb(206, 17, 38),
        ),
        "Brazil" => flag_brazil(painter, r),
        "Chile" => flag_chile(painter, r),
        "Argentina" => flag_h3(
            painter,
            r,
            Color32::from_rgb(116, 172, 223),
            Color32::WHITE,
            Color32::from_rgb(116, 172, 223),
        ),
        "Peru" => flag_v3(
            painter,
            r,
            Color32::from_rgb(217, 16, 35),
            Color32::WHITE,
            Color32::from_rgb(217, 16, 35),
        ),
        "South Africa" => flag_south_africa(painter, r),
        "Egypt" => flag_h3(
            painter,
            r,
            Color32::from_rgb(206, 17, 38),
            Color32::WHITE,
            Color32::BLACK,
        ),
        "United Arab Emirates" => flag_uae(painter, r),
        "Bahrain" => flag_bahrain(painter, r),
        "India" => flag_india(painter, r),
        "Singapore" => flag_singapore(painter, r),
        "Hong Kong" => flag_hk(painter, r),
        "Taiwan" => flag_taiwan(painter, r),
        "Japan" => flag_japan(painter, r),
        "South Korea" => flag_korea(painter, r),
        "Australia" => flag_australia(painter, r),
        "New Zealand" => flag_new_zealand(painter, r),
        "Luxembourg" => flag_h3(
            painter,
            r,
            Color32::from_rgb(239, 51, 64),
            Color32::WHITE,
            Color32::from_rgb(81, 167, 232),
        ),
        "Philippines" => flag_philippines(painter, r),
        "China" => flag_china(painter, r),
        "Russia" => flag_h3(
            painter,
            r,
            Color32::WHITE,
            Color32::from_rgb(0, 57, 166),
            Color32::from_rgb(213, 43, 30),
        ),
        "Ukraine" => flag_h2(
            painter,
            r,
            Color32::from_rgb(0, 91, 187),
            Color32::from_rgb(255, 213, 48),
        ),
        "Israel" => flag_israel(painter, r),
        "Saudi Arabia" => flag_saudi(painter, r),
        "Qatar" => flag_qatar(painter, r),
        "Kuwait" => flag_kuwait(painter, r),
        "Thailand" => flag_thailand(painter, r),
        "Vietnam" => flag_vietnam(painter, r),
        "Malaysia" => flag_malaysia(painter, r),
        "Indonesia" => flag_h2(painter, r, Color32::from_rgb(206, 17, 38), Color32::WHITE),
        _ => {
            painter.rect_filled(r, 3.0, Color32::from_rgb(67, 83, 102));
            painter.circle_filled(r.center(), 4.0, Color32::from_rgb(190, 204, 222));
        }
    }
}

fn flag_h2(p: &egui::Painter, r: Rect, top: Color32, bottom: Color32) {
    let h = r.height() / 2.0;
    p.rect_filled(
        Rect::from_min_max(r.min, egui::pos2(r.max.x, r.min.y + h)),
        0.0,
        top,
    );
    p.rect_filled(
        Rect::from_min_max(egui::pos2(r.min.x, r.min.y + h), r.max),
        0.0,
        bottom,
    );
}

fn flag_h3(p: &egui::Painter, r: Rect, top: Color32, mid: Color32, bottom: Color32) {
    let h = r.height() / 3.0;
    p.rect_filled(
        Rect::from_min_max(r.min, egui::pos2(r.max.x, r.min.y + h)),
        0.0,
        top,
    );
    p.rect_filled(
        Rect::from_min_max(
            egui::pos2(r.min.x, r.min.y + h),
            egui::pos2(r.max.x, r.min.y + 2.0 * h),
        ),
        0.0,
        mid,
    );
    p.rect_filled(
        Rect::from_min_max(egui::pos2(r.min.x, r.min.y + 2.0 * h), r.max),
        0.0,
        bottom,
    );
}

fn flag_v3(p: &egui::Painter, r: Rect, left: Color32, mid: Color32, right: Color32) {
    let w = r.width() / 3.0;
    p.rect_filled(
        Rect::from_min_max(r.min, egui::pos2(r.min.x + w, r.max.y)),
        0.0,
        left,
    );
    p.rect_filled(
        Rect::from_min_max(
            egui::pos2(r.min.x + w, r.min.y),
            egui::pos2(r.min.x + 2.0 * w, r.max.y),
        ),
        0.0,
        mid,
    );
    p.rect_filled(
        Rect::from_min_max(egui::pos2(r.min.x + 2.0 * w, r.min.y), r.max),
        0.0,
        right,
    );
}

fn flag_cross(p: &egui::Painter, r: Rect, base: Color32, cross: Color32) {
    p.rect_filled(r, 0.0, base);
    let v = r.min.x + r.width() * 0.42;
    let h = r.min.y + r.height() * 0.50;
    p.rect_filled(
        Rect::from_min_max(egui::pos2(v - 2.0, r.min.y), egui::pos2(v + 2.0, r.max.y)),
        0.0,
        cross,
    );
    p.rect_filled(
        Rect::from_min_max(egui::pos2(r.min.x, h - 2.0), egui::pos2(r.max.x, h + 2.0)),
        0.0,
        cross,
    );
}

fn flag_uk(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 0.0, Color32::from_rgb(18, 53, 128));
    p.rect_filled(
        Rect::from_min_max(
            egui::pos2(r.center().x - 2.0, r.min.y),
            egui::pos2(r.center().x + 2.0, r.max.y),
        ),
        0.0,
        Color32::WHITE,
    );
    p.rect_filled(
        Rect::from_min_max(
            egui::pos2(r.min.x, r.center().y - 2.0),
            egui::pos2(r.max.x, r.center().y + 2.0),
        ),
        0.0,
        Color32::WHITE,
    );
    p.rect_filled(
        Rect::from_min_max(
            egui::pos2(r.center().x - 1.0, r.min.y),
            egui::pos2(r.center().x + 1.0, r.max.y),
        ),
        0.0,
        Color32::from_rgb(200, 16, 46),
    );
    p.rect_filled(
        Rect::from_min_max(
            egui::pos2(r.min.x, r.center().y - 1.0),
            egui::pos2(r.max.x, r.center().y + 1.0),
        ),
        0.0,
        Color32::from_rgb(200, 16, 46),
    );
}

fn flag_portugal(p: &egui::Painter, r: Rect) {
    let split = r.min.x + r.width() * 0.42;
    p.rect_filled(
        Rect::from_min_max(r.min, egui::pos2(split, r.max.y)),
        0.0,
        Color32::from_rgb(0, 102, 0),
    );
    p.rect_filled(
        Rect::from_min_max(egui::pos2(split, r.min.y), r.max),
        0.0,
        Color32::from_rgb(206, 17, 38),
    );
    p.circle_filled(
        egui::pos2(split, r.center().y),
        3.0,
        Color32::from_rgb(255, 205, 0),
    );
}

fn flag_czech(p: &egui::Painter, r: Rect) {
    flag_h2(p, r, Color32::WHITE, Color32::from_rgb(215, 38, 61));
    let pts = vec![
        r.min,
        egui::pos2(r.min.x + r.width() * 0.48, r.center().y),
        egui::pos2(r.min.x, r.max.y),
    ];
    p.add(egui::Shape::convex_polygon(
        pts,
        Color32::from_rgb(17, 69, 126),
        Stroke::NONE,
    ));
}

fn flag_norway(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 0.0, Color32::from_rgb(186, 12, 47));
    let vx = r.min.x + r.width() * 0.38;
    let hy = r.min.y + r.height() * 0.50;
    p.rect_filled(
        Rect::from_min_max(egui::pos2(vx - 4.0, r.min.y), egui::pos2(vx + 4.0, r.max.y)),
        0.0,
        Color32::WHITE,
    );
    p.rect_filled(
        Rect::from_min_max(egui::pos2(r.min.x, hy - 3.0), egui::pos2(r.max.x, hy + 3.0)),
        0.0,
        Color32::WHITE,
    );
    p.rect_filled(
        Rect::from_min_max(egui::pos2(vx - 2.0, r.min.y), egui::pos2(vx + 2.0, r.max.y)),
        0.0,
        Color32::from_rgb(0, 32, 91),
    );
    p.rect_filled(
        Rect::from_min_max(egui::pos2(r.min.x, hy - 1.5), egui::pos2(r.max.x, hy + 1.5)),
        0.0,
        Color32::from_rgb(0, 32, 91),
    );
}

fn flag_greece(p: &egui::Painter, r: Rect) {
    for i in 0..9 {
        let y0 = r.min.y + r.height() * i as f32 / 9.0;
        let y1 = r.min.y + r.height() * (i + 1) as f32 / 9.0;
        let c = if i % 2 == 0 {
            Color32::from_rgb(13, 94, 175)
        } else {
            Color32::WHITE
        };
        p.rect_filled(
            Rect::from_min_max(egui::pos2(r.min.x, y0), egui::pos2(r.max.x, y1)),
            0.0,
            c,
        );
    }
    let canton = Rect::from_min_max(
        r.min,
        egui::pos2(r.min.x + r.width() * 0.45, r.min.y + r.height() * 0.55),
    );
    p.rect_filled(canton, 0.0, Color32::from_rgb(13, 94, 175));
    p.rect_filled(
        Rect::from_min_max(
            egui::pos2(canton.center().x - 1.5, canton.min.y),
            egui::pos2(canton.center().x + 1.5, canton.max.y),
        ),
        0.0,
        Color32::WHITE,
    );
    p.rect_filled(
        Rect::from_min_max(
            egui::pos2(canton.min.x, canton.center().y - 1.5),
            egui::pos2(canton.max.x, canton.center().y + 1.5),
        ),
        0.0,
        Color32::WHITE,
    );
}

fn flag_turkey(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 0.0, Color32::from_rgb(227, 10, 39));
    p.circle_filled(
        egui::pos2(r.min.x + r.width() * 0.45, r.center().y),
        5.0,
        Color32::WHITE,
    );
    p.circle_filled(
        egui::pos2(r.min.x + r.width() * 0.50, r.center().y),
        4.0,
        Color32::from_rgb(227, 10, 39),
    );
    p.circle_filled(
        egui::pos2(r.min.x + r.width() * 0.59, r.center().y),
        1.8,
        Color32::WHITE,
    );
}

fn flag_usa(p: &egui::Painter, r: Rect) {
    let h = r.height() / 7.0;
    for i in 0..7 {
        let y0 = r.min.y + i as f32 * h;
        let y1 = r.min.y + (i + 1) as f32 * h;
        p.rect_filled(
            Rect::from_min_max(egui::pos2(r.min.x, y0), egui::pos2(r.max.x, y1)),
            0.0,
            if i % 2 == 0 {
                Color32::from_rgb(191, 10, 48)
            } else {
                Color32::WHITE
            },
        );
    }
    let canton = Rect::from_min_max(
        r.min,
        egui::pos2(r.min.x + r.width() * 0.44, r.min.y + r.height() * 0.56),
    );
    p.rect_filled(canton, 0.0, Color32::from_rgb(26, 44, 96));
    p.circle_filled(
        egui::pos2(canton.min.x + 5.0, canton.min.y + 4.0),
        1.2,
        Color32::WHITE,
    );
    p.circle_filled(
        egui::pos2(canton.min.x + 10.0, canton.min.y + 7.0),
        1.2,
        Color32::WHITE,
    );
    p.circle_filled(
        egui::pos2(canton.min.x + 15.0, canton.min.y + 4.0),
        1.2,
        Color32::WHITE,
    );
}

fn flag_canada(p: &egui::Painter, r: Rect) {
    flag_v3(
        p,
        r,
        Color32::from_rgb(213, 43, 30),
        Color32::WHITE,
        Color32::from_rgb(213, 43, 30),
    );
    p.circle_filled(r.center(), 3.2, Color32::from_rgb(213, 43, 30));
}

fn flag_brazil(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 0.0, Color32::from_rgb(0, 156, 59));
    let pts = vec![
        egui::pos2(r.center().x, r.min.y + 2.0),
        egui::pos2(r.max.x - 3.0, r.center().y),
        egui::pos2(r.center().x, r.max.y - 2.0),
        egui::pos2(r.min.x + 3.0, r.center().y),
    ];
    p.add(egui::Shape::convex_polygon(
        pts,
        Color32::from_rgb(255, 223, 0),
        Stroke::NONE,
    ));
    p.circle_filled(r.center(), 4.0, Color32::from_rgb(0, 39, 118));
}

fn flag_chile(p: &egui::Painter, r: Rect) {
    flag_h2(p, r, Color32::WHITE, Color32::from_rgb(213, 43, 30));
    let blue = Rect::from_min_max(
        r.min,
        egui::pos2(r.min.x + r.width() * 0.38, r.min.y + r.height() * 0.5),
    );
    p.rect_filled(blue, 0.0, Color32::from_rgb(0, 57, 166));
    p.circle_filled(blue.center(), 2.0, Color32::WHITE);
}

fn flag_south_africa(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 0.0, Color32::from_rgb(222, 56, 49));
    let lower = Rect::from_min_max(egui::pos2(r.min.x, r.center().y), r.max);
    p.rect_filled(lower, 0.0, Color32::from_rgb(0, 122, 77));
    let black = egui::pos2(r.min.x + 5.0, r.center().y);
    let pts = vec![
        r.min,
        black,
        egui::pos2(r.min.x + 5.0, r.max.y),
        egui::pos2(r.min.x + 11.0, r.max.y),
        egui::pos2(r.min.x + 11.0, r.min.y),
        r.max,
    ];
    p.add(egui::Shape::convex_polygon(
        pts,
        Color32::from_rgb(0, 0, 0),
        Stroke::NONE,
    ));
    p.line_segment(
        [
            egui::pos2(r.min.x + 1.0, r.center().y),
            egui::pos2(r.max.x - 1.0, r.center().y),
        ],
        Stroke::new(2.0, Color32::from_rgb(255, 184, 28)),
    );
}

fn flag_uae(p: &egui::Painter, r: Rect) {
    let redw = r.width() * 0.22;
    p.rect_filled(
        Rect::from_min_max(r.min, egui::pos2(r.min.x + redw, r.max.y)),
        0.0,
        Color32::from_rgb(206, 17, 38),
    );
    let rest = Rect::from_min_max(egui::pos2(r.min.x + redw, r.min.y), r.max);
    flag_h3(
        p,
        rest,
        Color32::from_rgb(0, 122, 61),
        Color32::WHITE,
        Color32::BLACK,
    );
}

fn flag_bahrain(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 0.0, Color32::WHITE);
    let red = Rect::from_min_max(egui::pos2(r.min.x + r.width() * 0.32, r.min.y), r.max);
    p.rect_filled(red, 0.0, Color32::from_rgb(206, 17, 38));
    for i in 0..4 {
        let x = r.min.x + r.width() * 0.32 + i as f32 * 2.0;
        p.rect_filled(
            Rect::from_min_max(
                egui::pos2(x, r.min.y + 2.0),
                egui::pos2(x + 2.0, r.min.y + 6.0),
            ),
            0.0,
            Color32::WHITE,
        );
    }
}

fn flag_india(p: &egui::Painter, r: Rect) {
    flag_h3(
        p,
        r,
        Color32::from_rgb(255, 153, 51),
        Color32::WHITE,
        Color32::from_rgb(19, 136, 8),
    );
    p.circle_stroke(
        r.center(),
        3.0,
        Stroke::new(1.0, Color32::from_rgb(0, 0, 128)),
    );
}

fn flag_singapore(p: &egui::Painter, r: Rect) {
    flag_h2(p, r, Color32::from_rgb(239, 51, 64), Color32::WHITE);
    p.circle_filled(
        egui::pos2(r.min.x + 6.0, r.min.y + 5.0),
        3.5,
        Color32::WHITE,
    );
    p.circle_filled(
        egui::pos2(r.min.x + 7.5, r.min.y + 5.0),
        2.7,
        Color32::from_rgb(239, 51, 64),
    );
}

fn flag_hk(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 0.0, Color32::from_rgb(222, 41, 16));
    p.circle_filled(r.center(), 3.4, Color32::WHITE);
}

fn flag_taiwan(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 0.0, Color32::from_rgb(254, 0, 0));
    let canton = Rect::from_min_max(
        r.min,
        egui::pos2(r.min.x + r.width() * 0.5, r.min.y + r.height() * 0.55),
    );
    p.rect_filled(canton, 0.0, Color32::from_rgb(0, 56, 168));
    p.circle_filled(canton.center(), 3.0, Color32::WHITE);
}

fn flag_japan(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 0.0, Color32::WHITE);
    p.circle_filled(r.center(), 5.0, Color32::from_rgb(188, 0, 45));
}

fn flag_korea(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 0.0, Color32::WHITE);
    p.circle_filled(r.center(), 4.5, Color32::from_rgb(205, 46, 58));
    p.circle_filled(
        egui::pos2(r.center().x + 1.8, r.center().y + 1.0),
        3.2,
        Color32::from_rgb(0, 71, 160),
    );
}

fn flag_australia(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 0.0, Color32::from_rgb(0, 43, 92));
    p.rect_filled(
        Rect::from_min_max(
            r.min,
            egui::pos2(r.min.x + r.width() * 0.5, r.min.y + r.height() * 0.55),
        ),
        0.0,
        Color32::from_rgb(18, 53, 128),
    );
    p.circle_filled(
        egui::pos2(r.max.x - 7.0, r.min.y + 6.0),
        2.0,
        Color32::WHITE,
    );
    p.circle_filled(
        egui::pos2(r.max.x - 14.0, r.max.y - 6.0),
        1.7,
        Color32::WHITE,
    );
    p.circle_filled(
        egui::pos2(r.max.x - 5.0, r.max.y - 3.0),
        1.3,
        Color32::WHITE,
    );
}

fn flag_new_zealand(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 0.0, Color32::from_rgb(0, 43, 92));
    p.rect_filled(
        Rect::from_min_max(
            r.min,
            egui::pos2(r.min.x + r.width() * 0.45, r.min.y + r.height() * 0.55),
        ),
        0.0,
        Color32::from_rgb(18, 53, 128),
    );
    p.circle_filled(
        egui::pos2(r.max.x - 5.0, r.min.y + 6.0),
        1.8,
        Color32::from_rgb(220, 20, 60),
    );
    p.circle_filled(
        egui::pos2(r.max.x - 10.0, r.max.y - 5.0),
        1.6,
        Color32::from_rgb(220, 20, 60),
    );
}

fn flag_philippines(p: &egui::Painter, r: Rect) {
    flag_h2(
        p,
        r,
        Color32::from_rgb(0, 56, 168),
        Color32::from_rgb(206, 17, 38),
    );
    let pts = vec![
        r.min,
        egui::pos2(r.min.x, r.max.y),
        egui::pos2(r.min.x + r.width() * 0.46, r.center().y),
    ];
    p.add(egui::Shape::convex_polygon(
        pts,
        Color32::WHITE,
        Stroke::NONE,
    ));
    p.circle_filled(
        egui::pos2(r.min.x + r.width() * 0.19, r.center().y),
        2.0,
        Color32::from_rgb(255, 205, 0),
    );
}

fn flag_china(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 0.0, Color32::from_rgb(222, 41, 16));
    p.circle_filled(
        egui::pos2(r.min.x + 6.0, r.min.y + 5.0),
        2.4,
        Color32::from_rgb(255, 222, 0),
    );
}

fn flag_israel(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 0.0, Color32::WHITE);
    p.rect_filled(
        Rect::from_min_max(r.min, egui::pos2(r.max.x, r.min.y + 3.0)),
        0.0,
        Color32::from_rgb(0, 56, 168),
    );
    p.rect_filled(
        Rect::from_min_max(egui::pos2(r.min.x, r.max.y - 3.0), r.max),
        0.0,
        Color32::from_rgb(0, 56, 168),
    );
    p.circle_stroke(
        r.center(),
        4.0,
        Stroke::new(1.2, Color32::from_rgb(0, 56, 168)),
    );
}

fn flag_saudi(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 0.0, Color32::from_rgb(0, 122, 61));
    p.line_segment(
        [
            egui::pos2(r.min.x + 5.0, r.center().y),
            egui::pos2(r.max.x - 5.0, r.center().y),
        ],
        Stroke::new(2.0, Color32::WHITE),
    );
}

fn flag_qatar(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 0.0, Color32::from_rgb(139, 0, 58));
    let whitew = r.width() * 0.34;
    let poly = vec![
        r.min,
        egui::pos2(r.min.x + whitew, r.min.y + r.height() * 0.12),
        egui::pos2(r.min.x + whitew, r.max.y - r.height() * 0.12),
        egui::pos2(r.min.x, r.max.y),
    ];
    p.add(egui::Shape::convex_polygon(
        poly,
        Color32::WHITE,
        Stroke::NONE,
    ));
}

fn flag_kuwait(p: &egui::Painter, r: Rect) {
    flag_h3(
        p,
        r,
        Color32::from_rgb(0, 122, 61),
        Color32::WHITE,
        Color32::BLACK,
    );
    let pts = vec![
        r.min,
        egui::pos2(r.min.x + 7.0, r.min.y + r.height() * 0.5),
        egui::pos2(r.min.x, r.max.y),
    ];
    p.add(egui::Shape::convex_polygon(
        pts,
        Color32::from_rgb(206, 17, 38),
        Stroke::NONE,
    ));
}

fn flag_thailand(p: &egui::Painter, r: Rect) {
    flag_h3(
        p,
        r,
        Color32::from_rgb(165, 25, 49),
        Color32::WHITE,
        Color32::from_rgb(165, 25, 49),
    );
    let mid = Rect::from_min_max(
        egui::pos2(r.min.x, r.center().y - 3.0),
        egui::pos2(r.max.x, r.center().y + 3.0),
    );
    p.rect_filled(mid, 0.0, Color32::from_rgb(45, 42, 74));
}

fn flag_vietnam(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 0.0, Color32::from_rgb(218, 37, 29));
    p.circle_filled(r.center(), 3.2, Color32::from_rgb(255, 222, 0));
}

fn flag_malaysia(p: &egui::Painter, r: Rect) {
    for i in 0..7 {
        let y0 = r.min.y + i as f32 * r.height() / 7.0;
        let y1 = r.min.y + (i + 1) as f32 * r.height() / 7.0;
        p.rect_filled(
            Rect::from_min_max(egui::pos2(r.min.x, y0), egui::pos2(r.max.x, y1)),
            0.0,
            if i % 2 == 0 {
                Color32::from_rgb(204, 34, 50)
            } else {
                Color32::WHITE
            },
        );
    }
    let canton = Rect::from_min_max(
        r.min,
        egui::pos2(r.min.x + r.width() * 0.45, r.min.y + r.height() * 0.58),
    );
    p.rect_filled(canton, 0.0, Color32::from_rgb(1, 40, 104));
    p.circle_stroke(
        egui::pos2(canton.min.x + 7.0, canton.center().y),
        2.4,
        Stroke::new(1.0, Color32::from_rgb(255, 205, 0)),
    );
}

fn fetch_pops() -> Result<Vec<Pop>, String> {
    let client = Client::builder()
        .user_agent(format!("CS2-Server-Blocker/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let json: Value = client
        .get(SDR_URL)
        .send()
        .map_err(|e| format!("Could not reach Steam SDR API: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Steam SDR API returned an error: {e}"))?
        .json()
        .map_err(|e| format!("Invalid Steam SDR JSON: {e}"))?;

    let pops_obj = json
        .get("pops")
        .and_then(Value::as_object)
        .ok_or_else(|| "Steam SDR response does not contain a 'pops' object.".to_string())?;

    let mut pops = Vec::new();
    for (code, pop_value) in pops_obj {
        let relays = pop_value
            .get("relays")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|relay| relay.get("ipv4").and_then(Value::as_str))
                    .filter(|ip| is_ipv4(ip))
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if relays.is_empty() {
            continue;
        }

        let description = pop_value
            .get("desc")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(code)
            .to_string();
        let (country, _flag) = country_for_pop(code, &description);
        pops.push(Pop {
            code: code.clone(),
            location: description,
            country,
            relays,
        });
    }

    pops.sort_by(|a, b| a.country.cmp(&b.country).then_with(|| a.code.cmp(&b.code)));
    Ok(pops)
}

fn is_ipv4(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() == 4 && parts.iter().all(|p| p.parse::<u8>().is_ok())
}

fn country_for_pop(code: &str, description: &str) -> (String, String) {
    if let Some(country) = country_from_code(code) {
        return country;
    }
    if let Some(country) = country_from_description(description) {
        return country;
    }
    (format!("Other region ({})", code.to_uppercase()), "".into())
}

fn country_from_code(code: &str) -> Option<(String, String)> {
    Some(match code.to_ascii_lowercase().as_str() {
        "fra" | "ber" | "fsn" => ("Germany".into(), "🇩🇪".into()),
        "ams" | "ams2" | "ams4" => ("Netherlands".into(), "🇳🇱".into()),
        "lhr" | "lon" => ("United Kingdom".into(), "🇬🇧".into()),
        "par" => ("France".into(), "🇫🇷".into()),
        "mad" => ("Spain".into(), "🇪🇸".into()),
        "lis" => ("Portugal".into(), "🇵🇹".into()),
        "waw" => ("Poland".into(), "🇵🇱".into()),
        "prg" => ("Czechia".into(), "🇨🇿".into()),
        "vie" => ("Austria".into(), "🇦🇹".into()),
        "zrh" => ("Switzerland".into(), "🇨🇭".into()),
        "cph" => ("Denmark".into(), "🇩🇰".into()),
        "sto" | "sto2" => ("Sweden".into(), "🇸🇪".into()),
        "hel" => ("Finland".into(), "🇫🇮".into()),
        "osl" => ("Norway".into(), "🇳🇴".into()),
        "mil" | "rom" => ("Italy".into(), "🇮🇹".into()),
        "sof" => ("Bulgaria".into(), "🇧🇬".into()),
        "buc" | "otp" => ("Romania".into(), "🇷🇴".into()),
        "ath" => ("Greece".into(), "🇬🇷".into()),
        "ist" => ("Türkiye".into(), "🇹🇷".into()),
        "iad" | "atl" | "dfw" | "lax" | "nyc" | "ewr" | "ord" | "okc" | "sea" | "sjc" | "eat"
        | "mwh" | "tuk" | "gum" => ("United States".into(), "🇺🇸".into()),
        "yyz" => ("Canada".into(), "🇨🇦".into()),
        "mex" => ("Mexico".into(), "🇲🇽".into()),
        "gru" => ("Brazil".into(), "🇧🇷".into()),
        "scl" => ("Chile".into(), "🇨🇱".into()),
        "eze" | "bue" => ("Argentina".into(), "🇦🇷".into()),
        "lim" => ("Peru".into(), "🇵🇪".into()),
        "jnb" => ("South Africa".into(), "🇿🇦".into()),
        "cai" => ("Egypt".into(), "🇪🇬".into()),
        "dxb" | "dub" => ("United Arab Emirates".into(), "🇦🇪".into()),
        "bah" => ("Bahrain".into(), "🇧🇭".into()),
        "bom" | "bom2" | "maa" | "maa2" | "blr" => ("India".into(), "🇮🇳".into()),
        "sgp" | "sin" => ("Singapore".into(), "🇸🇬".into()),
        "hkg" | "hkg4" => ("Hong Kong".into(), "🇭🇰".into()),
        "tpe" => ("Taiwan".into(), "🇹🇼".into()),
        "tyo" | "ty1" | "nrt" => ("Japan".into(), "🇯🇵".into()),
        "sel" | "seo" => ("South Korea".into(), "🇰🇷".into()),
        "syd" | "mel" => ("Australia".into(), "🇦🇺".into()),
        "akl" => ("New Zealand".into(), "🇳🇿".into()),
        "lux" => ("Luxembourg".into(), "🇱🇺".into()),
        "man" => ("Philippines".into(), "🇵🇭".into()),
        "sha" | "sham" | "shat" | "shau" | "tsn" | "tsnm" | "tsnt" | "tsnu" | "canm" | "cant"
        | "canu" | "pwg" | "pwj" | "pwt" | "pwu" | "pww" | "pwz" | "pvg" | "pvgm" | "pvgt"
        | "pvgu" | "pek" | "pekm" | "pekt" | "peku" | "ctu" | "ctum" | "ctut" | "ctuu" | "tgd"
        | "tgdm" | "tgdt" | "tgdu" => ("China".into(), "🇨🇳".into()),
        _ => return None,
    })
}

fn country_from_description(description: &str) -> Option<(String, String)> {
    let normalized = description.to_ascii_lowercase();

    let country_tokens = [
        ("germany", "Germany", "🇩🇪"),
        ("netherlands", "Netherlands", "🇳🇱"),
        ("united kingdom", "United Kingdom", "🇬🇧"),
        ("great britain", "United Kingdom", "🇬🇧"),
        ("france", "France", "🇫🇷"),
        ("spain", "Spain", "🇪🇸"),
        ("portugal", "Portugal", "🇵🇹"),
        ("poland", "Poland", "🇵🇱"),
        ("czechia", "Czechia", "🇨🇿"),
        ("czech republic", "Czechia", "🇨🇿"),
        ("austria", "Austria", "🇦🇹"),
        ("switzerland", "Switzerland", "🇨🇭"),
        ("denmark", "Denmark", "🇩🇰"),
        ("sweden", "Sweden", "🇸🇪"),
        ("finland", "Finland", "🇫🇮"),
        ("norway", "Norway", "🇳🇴"),
        ("italy", "Italy", "🇮🇹"),
        ("bulgaria", "Bulgaria", "🇧🇬"),
        ("romania", "Romania", "🇷🇴"),
        ("greece", "Greece", "🇬🇷"),
        ("turkey", "Türkiye", "🇹🇷"),
        ("türkiye", "Türkiye", "🇹🇷"),
        ("united states", "United States", "🇺🇸"),
        ("usa", "United States", "🇺🇸"),
        ("canada", "Canada", "🇨🇦"),
        ("mexico", "Mexico", "🇲🇽"),
        ("brazil", "Brazil", "🇧🇷"),
        ("chile", "Chile", "🇨🇱"),
        ("argentina", "Argentina", "🇦🇷"),
        ("peru", "Peru", "🇵🇪"),
        ("south africa", "South Africa", "🇿🇦"),
        ("egypt", "Egypt", "🇪🇬"),
        ("united arab emirates", "United Arab Emirates", "🇦🇪"),
        ("uae", "United Arab Emirates", "🇦🇪"),
        ("bahrain", "Bahrain", "🇧🇭"),
        ("india", "India", "🇮🇳"),
        ("singapore", "Singapore", "🇸🇬"),
        ("hong kong", "Hong Kong", "🇭🇰"),
        ("taiwan", "Taiwan", "🇹🇼"),
        ("japan", "Japan", "🇯🇵"),
        ("south korea", "South Korea", "🇰🇷"),
        ("australia", "Australia", "🇦🇺"),
        ("new zealand", "New Zealand", "🇳🇿"),
        ("luxembourg", "Luxembourg", "🇱🇺"),
        ("philippines", "Philippines", "🇵🇭"),
        ("china", "China", "🇨🇳"),
        ("russia", "Russia", "🇷🇺"),
        ("ukraine", "Ukraine", "🇺🇦"),
        ("israel", "Israel", "🇮🇱"),
        ("saudi arabia", "Saudi Arabia", "🇸🇦"),
        ("qatar", "Qatar", "🇶🇦"),
        ("kuwait", "Kuwait", "🇰🇼"),
        ("thailand", "Thailand", "🇹🇭"),
        ("vietnam", "Vietnam", "🇻🇳"),
        ("malaysia", "Malaysia", "🇲🇾"),
        ("indonesia", "Indonesia", "🇮🇩"),
    ];

    for (needle, name, flag) in country_tokens {
        if normalized.contains(needle) {
            return Some((name.into(), flag.into()));
        }
    }

    let regional_tokens = [
        ("alabama", "United States", "🇺🇸"),
        ("alaska", "United States", "🇺🇸"),
        ("arizona", "United States", "🇺🇸"),
        ("arkansas", "United States", "🇺🇸"),
        ("california", "United States", "🇺🇸"),
        ("colorado", "United States", "🇺🇸"),
        ("connecticut", "United States", "🇺🇸"),
        ("delaware", "United States", "🇺🇸"),
        ("florida", "United States", "🇺🇸"),
        ("georgia", "United States", "🇺🇸"),
        ("hawaii", "United States", "🇺🇸"),
        ("illinois", "United States", "🇺🇸"),
        ("indiana", "United States", "🇺🇸"),
        ("maryland", "United States", "🇺🇸"),
        ("massachusetts", "United States", "🇺🇸"),
        ("michigan", "United States", "🇺🇸"),
        ("minnesota", "United States", "🇺🇸"),
        ("missouri", "United States", "🇺🇸"),
        ("nevada", "United States", "🇺🇸"),
        ("new jersey", "United States", "🇺🇸"),
        ("new york", "United States", "🇺🇸"),
        ("north carolina", "United States", "🇺🇸"),
        ("ohio", "United States", "🇺🇸"),
        ("oklahoma", "United States", "🇺🇸"),
        ("oregon", "United States", "🇺🇸"),
        ("pennsylvania", "United States", "🇺🇸"),
        ("texas", "United States", "🇺🇸"),
        ("virginia", "United States", "🇺🇸"),
        ("washington", "United States", "🇺🇸"),
        ("wisconsin", "United States", "🇺🇸"),
        ("ontario", "Canada", "🇨🇦"),
        ("quebec", "Canada", "🇨🇦"),
        ("british columbia", "Canada", "🇨🇦"),
        ("alberta", "Canada", "🇨🇦"),
        ("new south wales", "Australia", "🇦🇺"),
        ("victoria", "Australia", "🇦🇺"),
        ("queensland", "Australia", "🇦🇺"),
        ("western australia", "Australia", "🇦🇺"),
    ];
    for (needle, name, flag) in regional_tokens {
        if normalized.contains(needle) {
            return Some((name.into(), flag.into()));
        }
    }

    let city_tokens = [
        ("frankfurt", "Germany", "🇩🇪"),
        ("berlin", "Germany", "🇩🇪"),
        ("amsterdam", "Netherlands", "🇳🇱"),
        ("london", "United Kingdom", "🇬🇧"),
        ("paris", "France", "🇫🇷"),
        ("madrid", "Spain", "🇪🇸"),
        ("lisbon", "Portugal", "🇵🇹"),
        ("warsaw", "Poland", "🇵🇱"),
        ("prague", "Czechia", "🇨🇿"),
        ("vienna", "Austria", "🇦🇹"),
        ("zurich", "Switzerland", "🇨🇭"),
        ("copenhagen", "Denmark", "🇩🇰"),
        ("stockholm", "Sweden", "🇸🇪"),
        ("helsinki", "Finland", "🇫🇮"),
        ("oslo", "Norway", "🇳🇴"),
        ("milan", "Italy", "🇮🇹"),
        ("rome", "Italy", "🇮🇹"),
        ("sofia", "Bulgaria", "🇧🇬"),
        ("bucharest", "Romania", "🇷🇴"),
        ("athens", "Greece", "🇬🇷"),
        ("istanbul", "Türkiye", "🇹🇷"),
        ("atlanta", "United States", "🇺🇸"),
        ("chicago", "United States", "🇺🇸"),
        ("dallas", "United States", "🇺🇸"),
        ("los angeles", "United States", "🇺🇸"),
        ("new york", "United States", "🇺🇸"),
        ("seattle", "United States", "🇺🇸"),
        ("washington", "United States", "🇺🇸"),
        ("toronto", "Canada", "🇨🇦"),
        ("mexico city", "Mexico", "🇲🇽"),
        ("sao paulo", "Brazil", "🇧🇷"),
        ("santiago", "Chile", "🇨🇱"),
        ("buenos aires", "Argentina", "🇦🇷"),
        ("lima", "Peru", "🇵🇪"),
        ("johannesburg", "South Africa", "🇿🇦"),
        ("cairo", "Egypt", "🇪🇬"),
        ("dubai", "United Arab Emirates", "🇦🇪"),
        ("manama", "Bahrain", "🇧🇭"),
        ("mumbai", "India", "🇮🇳"),
        ("chennai", "India", "🇮🇳"),
        ("bengaluru", "India", "🇮🇳"),
        ("singapore", "Singapore", "🇸🇬"),
        ("hong kong", "Hong Kong", "🇭🇰"),
        ("taipei", "Taiwan", "🇹🇼"),
        ("tokyo", "Japan", "🇯🇵"),
        ("seoul", "South Korea", "🇰🇷"),
        ("sydney", "Australia", "🇦🇺"),
        ("melbourne", "Australia", "🇦🇺"),
        ("auckland", "New Zealand", "🇳🇿"),
        ("manila", "Philippines", "🇵🇭"),
        ("shanghai", "China", "🇨🇳"),
        ("beijing", "China", "🇨🇳"),
        ("guangzhou", "China", "🇨🇳"),
        ("shenzhen", "China", "🇨🇳"),
        ("tianjin", "China", "🇨🇳"),
        ("wuhan", "China", "🇨🇳"),
        ("moscow", "Russia", "🇷🇺"),
        ("kyiv", "Ukraine", "🇺🇦"),
        ("tel aviv", "Israel", "🇮🇱"),
        ("riyadh", "Saudi Arabia", "🇸🇦"),
        ("doha", "Qatar", "🇶🇦"),
        ("kuwait city", "Kuwait", "🇰🇼"),
        ("bangkok", "Thailand", "🇹🇭"),
        ("hanoi", "Vietnam", "🇻🇳"),
        ("kuala lumpur", "Malaysia", "🇲🇾"),
        ("jakarta", "Indonesia", "🇮🇩"),
    ];
    for (needle, name, flag) in city_tokens {
        if normalized.contains(needle) {
            return Some((name.into(), flag.into()));
        }
    }

    None
}
fn state_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".local").join("state").join("cs2-server-blocker")
}

fn load_state() -> StoredState {
    let path = state_path().join(STATE_FILE);
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_state(state: &StoredState) -> Result<(), String> {
    let dir = state_path();
    fs::create_dir_all(&dir).map_err(|e| format!("Could not create state directory: {e}"))?;
    let path = dir.join(STATE_FILE);
    let json =
        serde_json::to_string_pretty(state).map_err(|e| format!("Could not encode state: {e}"))?;
    fs::write(path, json).map_err(|e| format!("Could not write state: {e}"))
}

fn detect_firewall() -> Option<FirewallBackend> {
    [
        (FirewallBackend::Ufw, "ufw"),
        (FirewallBackend::Firewalld, "firewall-cmd"),
        (FirewallBackend::Nftables, "nft"),
        (FirewallBackend::Iptables, "iptables"),
    ]
    .into_iter()
    .find_map(|(backend, command)| command_exists(command).then_some(backend))
}

fn command_exists(command: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {command} >/dev/null 2>&1"))
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn privileged_script(commands: &[String]) -> Result<(), String> {
    if commands.is_empty() {
        return Ok(());
    }

    let mut script = String::from("set -e\n");
    for command in commands {
        script.push_str(command);
        script.push('\n');
    }

    let output = if Command::new("id")
        .arg("-u")
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim() == "0")
        .unwrap_or(false)
    {
        Command::new("/bin/sh")
            .args(["-c", &script])
            .output()
            .map_err(|e| format!("Could not run privileged firewall batch: {e}"))?
    } else {
        if !command_exists("pkexec") {
            return Err("Root privileges are required, but pkexec is not installed.".into());
        }
        Command::new("pkexec")
            .args(["/bin/sh", "-c", &script])
            .output()
            .map_err(|e| format!("Could not run privileged firewall batch: {e}"))?
    };

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Err(if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!(
            "Privileged firewall batch exited with status {}",
            output.status
        )
    })
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn append_backend_initialization(commands: &mut Vec<String>, backend: FirewallBackend) {
    match backend {
        FirewallBackend::Ufw | FirewallBackend::Firewalld => {}
        FirewallBackend::Iptables => {
            commands.push(
                "iptables -S CS2_SERVER_BLOCKER >/dev/null 2>&1 || iptables -N CS2_SERVER_BLOCKER"
                    .into(),
            );
            commands.push(
                "iptables -C INPUT -j CS2_SERVER_BLOCKER >/dev/null 2>&1 || iptables -I INPUT -j CS2_SERVER_BLOCKER"
                    .into(),
            );
            commands.push(
                "iptables -C OUTPUT -j CS2_SERVER_BLOCKER >/dev/null 2>&1 || iptables -I OUTPUT -j CS2_SERVER_BLOCKER"
                    .into(),
            );
        }
        FirewallBackend::Nftables => {
            commands
                .push("if ! nft list table inet cs2_server_blocker >/dev/null 2>&1; then".into());
            commands.push("nft add table inet cs2_server_blocker".into());
            commands.push(
                "nft add chain inet cs2_server_blocker input '{' type filter hook input priority 0 ';' policy accept ';' '}'"
                    .into(),
            );
            commands.push(
                "nft add chain inet cs2_server_blocker output '{' type filter hook output priority 0 ';' policy accept ';' '}'"
                    .into(),
            );
            commands.push("fi".into());
        }
    }
}

fn append_block_ip(commands: &mut Vec<String>, backend: FirewallBackend, ip: &str) {
    let ip = shell_quote(ip);
    match backend {
        FirewallBackend::Ufw => {
            commands.push(format!(
                "ufw deny from {ip} to any proto udp comment {}",
                shell_quote(UFW_COMMENT)
            ));
            commands.push(format!(
                "ufw deny out to {ip} proto udp comment {}",
                shell_quote(UFW_COMMENT)
            ));
        }
        FirewallBackend::Firewalld => {
            commands.push(format!(
                "firewall-cmd --direct --add-rule ipv4 filter INPUT 0 -s {ip} -p udp -j REJECT -m comment --comment {}",
                shell_quote(FIREWALL_COMMENT)
            ));
            commands.push(format!(
                "firewall-cmd --direct --add-rule ipv4 filter OUTPUT 0 -d {ip} -p udp -j REJECT -m comment --comment {}",
                shell_quote(FIREWALL_COMMENT)
            ));
        }
        FirewallBackend::Iptables => {
            commands.push(format!(
                "iptables -A CS2_SERVER_BLOCKER -s {ip} -p udp -j DROP"
            ));
            commands.push(format!(
                "iptables -A CS2_SERVER_BLOCKER -d {ip} -p udp -j DROP"
            ));
        }
        FirewallBackend::Nftables => {
            commands.push(format!(
                "nft add rule inet cs2_server_blocker input ip saddr {ip} counter drop"
            ));
            commands.push(format!(
                "nft add rule inet cs2_server_blocker output ip daddr {ip} counter drop"
            ));
        }
    }
}

fn append_unblock_ip(commands: &mut Vec<String>, backend: FirewallBackend, ip: &str) {
    let raw_ip = ip;
    let ip = shell_quote(raw_ip);
    match backend {
        FirewallBackend::Ufw => {
            commands.push(format!(
                "ufw --force delete deny from {ip} to any proto udp comment {} || true",
                shell_quote(UFW_COMMENT)
            ));
            commands.push(format!(
                "ufw --force delete deny out to {ip} proto udp comment {} || true",
                shell_quote(UFW_COMMENT)
            ));
        }
        FirewallBackend::Firewalld => {
            commands.push(format!(
                "firewall-cmd --direct --remove-rule ipv4 filter INPUT 0 -s {ip} -p udp -j REJECT -m comment --comment {} || true",
                shell_quote(FIREWALL_COMMENT)
            ));
            commands.push(format!(
                "firewall-cmd --direct --remove-rule ipv4 filter OUTPUT 0 -d {ip} -p udp -j REJECT -m comment --comment {} || true",
                shell_quote(FIREWALL_COMMENT)
            ));
        }
        FirewallBackend::Iptables => {
            commands.push(format!(
                "iptables -D CS2_SERVER_BLOCKER -s {ip} -p udp -j DROP || true"
            ));
            commands.push(format!(
                "iptables -D CS2_SERVER_BLOCKER -d {ip} -p udp -j DROP || true"
            ));
        }
        FirewallBackend::Nftables => {
            let input_needle = shell_quote(&format!("ip saddr {raw_ip}"));
            let output_needle = shell_quote(&format!("ip daddr {raw_ip}"));
            commands.push(format!(
                "nft -a list chain inet cs2_server_blocker input | grep -F -- {input_needle} | awk '{{print $NF}}' | while read -r handle; do nft delete rule inet cs2_server_blocker input handle \"$handle\"; done"
            ));
            commands.push(format!(
                "nft -a list chain inet cs2_server_blocker output | grep -F -- {output_needle} | awk '{{print $NF}}' | while read -r handle; do nft delete rule inet cs2_server_blocker output handle \"$handle\"; done"
            ));
        }
    }
}

fn append_backend_cleanup(commands: &mut Vec<String>, backend: FirewallBackend) {
    match backend {
        FirewallBackend::Iptables => {
            commands.push("iptables -D INPUT -j CS2_SERVER_BLOCKER >/dev/null 2>&1 || true".into());
            commands
                .push("iptables -D OUTPUT -j CS2_SERVER_BLOCKER >/dev/null 2>&1 || true".into());
            commands.push("iptables -F CS2_SERVER_BLOCKER >/dev/null 2>&1 || true".into());
            commands.push("iptables -X CS2_SERVER_BLOCKER >/dev/null 2>&1 || true".into());
        }
        FirewallBackend::Nftables => {
            commands
                .push("nft delete table inet cs2_server_blocker >/dev/null 2>&1 || true".into());
        }
        FirewallBackend::Ufw | FirewallBackend::Firewalld => {}
    }
}

fn format_version(version: AppVersion) -> String {
    format!("{}.{}.{}", version.major, version.minor, version.patch)
}

fn check_for_updates_and_install() -> Result<UpdateOutcome, String> {
    if env::consts::OS != "linux" {
        return Err("In-app updates are currently supported on Linux only.".into());
    }

    if env::consts::ARCH != "x86_64" {
        return Err("In-app updates are currently supported on x86_64 Linux only.".into());
    }

    let current = AppVersion::parse(env!("CARGO_PKG_VERSION"))
        .ok_or_else(|| "The current app version is invalid.".to_string())?;

    let client = Client::builder()
        .user_agent(format!("CS2-Server-Blocker/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|err| format!("Failed to create update client: {err}"))?;

    let release = client
        .get(GITHUB_LATEST_RELEASE_URL)
        .send()
        .map_err(|err| format!("Failed to check GitHub for updates: {err}"))?
        .error_for_status()
        .map_err(|err| format!("GitHub update check failed: {err}"))?
        .json::<GithubRelease>()
        .map_err(|err| format!("Failed to parse the latest GitHub release: {err}"))?;

    let latest = AppVersion::parse(&release.tag_name).ok_or_else(|| {
        format!(
            "GitHub returned an invalid release tag: {}",
            release.tag_name
        )
    })?;

    if latest <= current {
        return Ok(UpdateOutcome::UpToDate { version: current });
    }

    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == LINUX_RELEASE_ASSET)
        .ok_or_else(|| {
            format!(
                "Latest release {} has no Linux x86_64 package.",
                release.tag_name
            )
        })?;

    install_release_package(&client, &asset.browser_download_url)?;

    Ok(UpdateOutcome::Updated { version: latest })
}

fn install_release_package(client: &Client, asset_url: &str) -> Result<(), String> {
    let update_dir = env::temp_dir().join(format!(
        "cs2-server-blocker-update-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "System clock is before UNIX epoch.")?
            .as_nanos()
    ));

    fs::create_dir_all(&update_dir)
        .map_err(|err| format!("Failed to create update directory: {err}"))?;

    let result = (|| {
        let archive = update_dir.join(LINUX_RELEASE_ASSET);
        let binary = update_dir.join("cs2-server-blocker");
        let desktop = update_dir.join("cs2-server-blocker.desktop");
        let icon = update_dir.join("cs2-server-blocker.svg");
        let script = update_dir.join("install-update.sh");

        let bytes = client
            .get(asset_url)
            .send()
            .map_err(|err| format!("Failed to download the latest release: {err}"))?
            .error_for_status()
            .map_err(|err| format!("Release download failed: {err}"))?
            .bytes()
            .map_err(|err| format!("Failed to read the release package: {err}"))?;

        fs::write(&archive, &bytes)
            .map_err(|err| format!("Failed to save the release package: {err}"))?;

        let status = Command::new("tar")
            .args(["-xzf"])
            .arg(&archive)
            .arg("-C")
            .arg(&update_dir)
            .status()
            .map_err(|err| format!("Failed to extract the release package: {err}"))?;

        if !status.success() {
            return Err("Failed to extract the latest release package.".into());
        }

        for path in [&binary, &desktop, &icon] {
            if !path.is_file() {
                return Err(format!(
                    "The downloaded release package is missing {}.",
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("a required file")
                ));
            }
        }

        let script_body = format!(
            "#!/bin/sh
set -eu
install -Dm755 {} {}
install -Dm644 {} {}
install -Dm644 {} {}
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database /usr/share/applications >/dev/null 2>&1 || true
fi
",
            shell_quote(binary.to_string_lossy().as_ref()),
            shell_quote(INSTALLED_BINARY),
            shell_quote(desktop.to_string_lossy().as_ref()),
            shell_quote(INSTALLED_DESKTOP_ENTRY),
            shell_quote(icon.to_string_lossy().as_ref()),
            shell_quote(INSTALLED_ICON),
        );

        fs::write(&script, script_body)
            .map_err(|err| format!("Failed to prepare the installer: {err}"))?;

        let status = Command::new("pkexec")
            .arg("sh")
            .arg(&script)
            .status()
            .map_err(|err| format!("Failed to start the privileged updater: {err}"))?;

        if !status.success() {
            return Err(
                "Update installation was cancelled or failed. Root privileges are required.".into(),
            );
        }

        Ok(())
    })();

    let _ = fs::remove_dir_all(&update_dir);
    result
}

fn perform_action(
    task: ActionTask,
    mut stored: StoredState,
    detected: Option<FirewallBackend>,
    pops: Vec<Pop>,
    selected: HashSet<String>,
) -> Result<ActionReport, String> {
    let backend = stored.backend.or(detected).ok_or_else(|| {
        "No supported firewall was detected (UFW, firewalld, nftables, or iptables).".to_string()
    })?;

    if let Some(detected_backend) = detected
        && detected_backend != backend
        && !stored.blocked_ips().is_empty()
    {
        return Err(format!(
            "Existing rules belong to {}, but {} is currently detected. Unblock them with the same firewall backend first.",
            backend.label(),
            detected_backend.label()
        ));
    }

    match task {
        ActionTask::BlockSelected => {
            let mut touched_pops = 0;
            let mut new_ips = HashSet::new();
            let mut pending: Vec<(String, Vec<String>)> = Vec::new();

            for pop in pops.iter().filter(|p| selected.contains(&p.code)) {
                let existing: HashSet<String> = stored
                    .blocked_by_pop
                    .get(&pop.code)
                    .into_iter()
                    .flatten()
                    .cloned()
                    .collect();
                let mut added_for_pop = Vec::new();

                for ip in &pop.relays {
                    if !existing.contains(ip) && new_ips.insert(ip.clone()) {
                        added_for_pop.push(ip.clone());
                    }
                }

                if !added_for_pop.is_empty() || stored.blocked_by_pop.contains_key(&pop.code) {
                    pending.push((pop.code.clone(), added_for_pop));
                }
                touched_pops += 1;
            }

            if !new_ips.is_empty() {
                let mut commands = Vec::new();
                append_backend_initialization(&mut commands, backend);
                let mut sorted_ips: Vec<String> = new_ips.iter().cloned().collect();
                sorted_ips.sort();
                for ip in &sorted_ips {
                    append_block_ip(&mut commands, backend, ip);
                }
                privileged_script(&commands)?;
            }

            for (code, ips) in pending {
                let entry = stored.blocked_by_pop.entry(code).or_default();
                for ip in ips {
                    if !entry.contains(&ip) {
                        entry.push(ip);
                    }
                }
            }

            stored.backend = if stored.blocked_ips().is_empty() {
                None
            } else {
                Some(backend)
            };
            save_state(&stored)?;
            Ok(ActionReport {
                action: "Blocked",
                pop_count: touched_pops,
                ip_count: new_ips.len(),
                backend,
            })
        }
        ActionTask::UnblockSelected => {
            let selected_codes: Vec<String> = selected.into_iter().collect();
            let mut removed_ips = HashSet::new();
            let mut touched_pops = 0;
            let mut next_stored = stored.clone();

            for code in &selected_codes {
                let ips = stored.ips_for_pop(code);
                if ips.is_empty() {
                    continue;
                }
                touched_pops += 1;
                for ip in &ips {
                    if !stored.still_referenced_elsewhere(ip, code) {
                        removed_ips.insert(ip.clone());
                    }
                }
                next_stored.blocked_by_pop.remove(code);
            }

            let will_be_empty = next_stored.blocked_ips().is_empty();
            if !removed_ips.is_empty() || will_be_empty {
                let mut commands = Vec::new();
                if will_be_empty
                    && matches!(
                        backend,
                        FirewallBackend::Iptables | FirewallBackend::Nftables
                    )
                {
                    append_backend_cleanup(&mut commands, backend);
                } else {
                    let mut sorted_ips: Vec<String> = removed_ips.iter().cloned().collect();
                    sorted_ips.sort();
                    for ip in &sorted_ips {
                        append_unblock_ip(&mut commands, backend, ip);
                    }
                    if will_be_empty {
                        append_backend_cleanup(&mut commands, backend);
                    }
                }
                if !commands.is_empty() {
                    privileged_script(&commands)?;
                }
            }

            stored = next_stored;
            if stored.blocked_ips().is_empty() {
                stored.backend = None;
            }
            save_state(&stored)?;
            Ok(ActionReport {
                action: "Unblocked",
                pop_count: touched_pops,
                ip_count: removed_ips.len(),
                backend,
            })
        }
        ActionTask::UnblockAll => {
            let all_ips = stored.blocked_ips();
            let mut commands = Vec::new();
            if matches!(
                backend,
                FirewallBackend::Iptables | FirewallBackend::Nftables
            ) {
                append_backend_cleanup(&mut commands, backend);
            } else {
                let mut sorted_ips: Vec<String> = all_ips.iter().cloned().collect();
                sorted_ips.sort();
                for ip in &sorted_ips {
                    append_unblock_ip(&mut commands, backend, ip);
                }
            }
            if !commands.is_empty() {
                privileged_script(&commands)?;
            }
            let count = all_ips.len();
            stored = StoredState::default();
            save_state(&stored)?;
            Ok(ActionReport {
                action: "Unblocked all",
                pop_count: 0,
                ip_count: count,
                backend,
            })
        }
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 700.0])
            .with_min_inner_size([800.0, 700.0]),
        ..Default::default()
    };

    eframe::run_native(
        "CS2 Server Blocker",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_release_versions() {
        assert_eq!(
            AppVersion::parse("v1.2.3"),
            Some(AppVersion {
                major: 1,
                minor: 2,
                patch: 3
            })
        );
        assert_eq!(
            AppVersion::parse("1.2.3-beta"),
            Some(AppVersion {
                major: 1,
                minor: 2,
                patch: 3
            })
        );
        assert_eq!(AppVersion::parse("1.2"), None);
    }

    #[test]
    fn compares_release_versions() {
        let current = AppVersion::parse("1.0.1").unwrap();
        let newer = AppVersion::parse("1.0.2").unwrap();
        assert!(newer > current);
        assert!(current <= current);
    }
}
