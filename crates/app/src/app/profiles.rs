//! 配置方案（Profile）选择器与备份管理的界面层。
//!
//! 存储与快照 / 恢复 / 保留策略都在 [`crate::profiles`]（core）；这里只做编排：
//! - 顶栏「方案」下拉：列出方案、当前项高亮；切换需确认；新建 / 删除入口；
//! - 「备份管理」悬浮窗：按时间倒序列出当前目标文件的备份（带 tag 标签），
//!   选中 → 恢复确认（先看「当前文件 → 备份内容」的 diff 再覆盖）；
//! - 保存写盘前挂自动快照（Auto tag，外部修改检测），写盘成功后按默认策略清理。

use super::diff;
use super::save::PageTarget;
use super::App;
use crate::profiles::{ProfileStore, PrunePolicy, SnapshotTag};
use eframe::egui;
use std::path::{Path, PathBuf};

// ---------- 备份文件名的解析与列表（纯函数，配单测） ----------

/// 备份列表条目：文件名与解析出的时间戳 / tag / pin 标记。
struct BackupEntry {
    /// 备份文件名（含 `.bak` 后缀），直接用于展示与选中项标识。
    file_name: String,
    /// `YYYYMMDD_HHMMSS`（UTC）时间戳文本；定长数字，字典序即时间序。
    ts: String,
    tag: SnapshotTag,
    pinned: bool,
}

/// 备份文件名的解析结果（与 core 的命名规则同形，从右侧剥）。
struct BackupName<'a> {
    stem: &'a str,
    ts: &'a str,
    tag: SnapshotTag,
    pinned: bool,
}

/// 解析 `{stem}.{YYYYMMDD_HHMMSS}.{tag}[.pin].bak`；stem 允许带点（目标文件名
/// 整体保留），所以 tag 与时间戳都只从右侧剥。不是这个形状的返回 None。
fn parse_backup_file_name(name: &str) -> Option<BackupName<'_>> {
    let without_bak = name.strip_suffix(".bak")?;
    let (without_pin, pinned) = match without_bak.strip_suffix(".pin") {
        Some(rest) => (rest, true),
        None => (without_bak, false),
    };
    let tag_dot = without_pin.rfind('.')?;
    let tag = match &without_pin[tag_dot + 1..] {
        "auto" => SnapshotTag::Auto,
        "manual" => SnapshotTag::Manual,
        "preswitch" => SnapshotTag::PreSwitch,
        "beforerestore" => SnapshotTag::BeforeRestore,
        _ => return None,
    };
    let before_tag = &without_pin[..tag_dot];
    let ts_dot = before_tag.rfind('.')?;
    if ts_dot == 0 {
        return None; // stem 不能为空
    }
    let ts = &before_tag[ts_dot + 1..];
    if !is_backup_timestamp(ts) {
        return None;
    }
    Some(BackupName {
        stem: &before_tag[..ts_dot],
        ts,
        tag,
        pinned,
    })
}

/// 是否为本约定的时间戳段：`YYYYMMDD_HHMMSS`（15 字节，第 9 位是下划线，其余数字）。
fn is_backup_timestamp(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 15
        && bytes[8] == b'_'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 8 || b.is_ascii_digit())
}

/// tag 的中文标签（备份列表与悬停提示共用）。
fn tag_label(tag: SnapshotTag) -> &'static str {
    match tag {
        SnapshotTag::Auto => "自动",
        SnapshotTag::Manual => "手动",
        SnapshotTag::PreSwitch => "切换前",
        SnapshotTag::BeforeRestore => "恢复前",
    }
}

/// tag 标签的颜色：四类备份一眼可分。
fn tag_color(ui: &egui::Ui, tag: SnapshotTag) -> egui::Color32 {
    let semantics = crate::theme::semantics(ui);
    match tag {
        SnapshotTag::Auto => semantics.info,
        SnapshotTag::Manual => ui.visuals().text_color(),
        SnapshotTag::PreSwitch => semantics.warn,
        SnapshotTag::BeforeRestore => semantics.ok,
    }
}

/// 列出属于 `target_name`（如 `opencode.json`）的备份，按时间倒序（新的在前）。
/// 目录不存在 / 读不出时返回空表。
fn list_backups_for(backups_root: &Path, target_name: &str) -> Vec<BackupEntry> {
    let Ok(entries) = std::fs::read_dir(backups_root) else {
        return Vec::new();
    };
    let mut out: Vec<BackupEntry> = entries
        .flatten()
        .filter_map(|entry| {
            let file_name = entry.file_name().to_str()?.to_string();
            let parsed = parse_backup_file_name(&file_name)?;
            if parsed.stem != target_name {
                return None;
            }
            // 先拷出借用字段，最后才 move 文件名。
            Some(BackupEntry {
                ts: parsed.ts.to_string(),
                tag: parsed.tag,
                pinned: parsed.pinned,
                file_name,
            })
        })
        .collect();
    // 定长数字时间戳：字典序 = 时间序，倒序即新的在前。
    out.sort_by(|a, b| b.ts.cmp(&a.ts));
    out
}

// ---------- 界面状态 ----------

/// 恢复确认框的内容：选中的备份、目标文件与缓存的「当前文件 → 备份内容」对比。
#[derive(Clone)]
struct RestoreConfirm {
    backup: String,
    target: String,
    lines: Vec<diff::DiffLine>,
    summary: diff::DiffSummary,
}

/// Profile / 备份管理的界面状态（全部是易失状态，重启清零）。
#[derive(Default)]
pub(in crate::app) struct ProfilesUi {
    /// 「切换到 X？」确认框的目标方案名。
    confirm_switch: Option<String>,
    /// 「新建方案」窗口开关与名字草稿。
    new_open: bool,
    new_name: String,
    /// 「删除方案」窗口开关、选中项与确认勾选。
    delete_open: bool,
    delete_name: String,
    delete_checked: bool,
    /// 备份管理窗口开关、选中备份与恢复确认框。
    show_backups: bool,
    selected: Option<String>,
    confirm_restore: Option<RestoreConfirm>,
    /// 窗口内的操作结果（清理了几条 / 已恢复）。
    note: Option<String>,
    /// 窗口内的错误提示（创建 / 删除 / 切换 / 恢复 / 清理失败）。
    error: Option<String>,
}

impl App {
    // ---------- 顶栏入口 ----------

    /// 顶栏「方案」按钮 + 下拉：方案列表（当前项高亮）、切换 / 新建 / 删除与备份管理。
    pub(in crate::app) fn ui_profile_button(&mut self, ui: &mut egui::Ui) {
        let current = ProfileStore::open().current();
        let label = current.clone().unwrap_or_else(|| "未选择".to_string());
        let btn = ui
            .button(format!("方案: {label}"))
            .on_hover_text("配置方案：列出 / 切换 / 新建 / 删除，以及当前文件的备份管理")
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        egui::Popup::menu(&btn)
            .layout(egui::Layout::top_down(egui::Align::LEFT))
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width(220.0);
                let store = ProfileStore::open();
                ui.label(egui::RichText::new("配置方案").small().weak());
                let metas = store.list();
                if metas.is_empty() {
                    ui.label(
                        egui::RichText::new("还没有方案，用下方「新建方案…」创建")
                            .small()
                            .weak(),
                    );
                }
                for meta in &metas {
                    let is_current = current.as_deref() == Some(meta.name.as_str());
                    let text = if is_current {
                        egui::RichText::new(format!("{}（当前）", meta.name)).strong()
                    } else {
                        egui::RichText::new(&meta.name)
                    };
                    if ui.add(egui::Button::selectable(is_current, text)).clicked() && !is_current {
                        self.profiles.error = None;
                        self.profiles.confirm_switch = Some(meta.name.clone());
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("新建方案…").clicked() {
                    self.profiles.new_name.clear();
                    self.profiles.new_open = true;
                    ui.close();
                }
                if ui.button("删除方案…").clicked() {
                    // 预选第一个非当前项，降低手滑删掉当前方案的概率。
                    self.profiles.delete_name = metas
                        .iter()
                        .map(|m| m.name.clone())
                        .find(|name| current.as_deref() != Some(name.as_str()))
                        .unwrap_or_default();
                    self.profiles.delete_checked = false;
                    self.profiles.delete_open = true;
                    ui.close();
                }
                ui.separator();
                if ui.button("备份管理…").clicked() {
                    self.profiles.selected = None;
                    self.profiles.note = None;
                    self.profiles.error = None;
                    self.profiles.confirm_restore = None;
                    self.profiles.show_backups = true;
                    ui.close();
                }
            });
    }

    // ---------- 悬浮窗 ----------

    /// Profile / 备份相关的全部悬浮窗：切换确认、新建、删除与备份管理（含恢复确认）。
    pub(in crate::app) fn ui_profiles_windows(&mut self, ctx: &egui::Context) {
        self.ui_confirm_switch_window(ctx);
        self.ui_new_profile_window(ctx);
        self.ui_delete_profile_window(ctx);
        self.ui_backups_window(ctx);
        self.ui_restore_confirm_window(ctx);
    }

    /// 「切换到 X？」确认框：切换只记录激活项，不动任何配置文件内容。
    fn ui_confirm_switch_window(&mut self, ctx: &egui::Context) {
        let Some(name) = self.profiles.confirm_switch.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new("切换配置方案")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .order(Self::TOKENS_WINDOW_ORDER)
            .constrain_to(ctx.content_rect())
            .frame(self.floating_window_frame(ctx))
            .show(ctx, |ui| {
                ui.label(format!("切换到「{name}」？"));
                ui.label(
                    egui::RichText::new("切换只改变「当前激活」的记录，不改动任何配置文件内容。")
                        .small()
                        .weak(),
                );
                ui.add_space(crate::theme::SPACE_2);
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::new(egui::RichText::new("切换").strong()))
                        .clicked()
                    {
                        match ProfileStore::open().switch(&name) {
                            Ok(()) => {
                                self.profiles.error = None;
                                self.profiles.confirm_switch = None;
                                self.status = format!("已切换到配置方案「{name}」");
                            }
                            Err(e) => self.profiles.error = Some(e),
                        }
                    }
                    if ui.button("取消").clicked() {
                        self.profiles.confirm_switch = None;
                    }
                });
                if let Some(e) = &self.profiles.error {
                    ui.colored_label(crate::theme::semantics(ui).err, e);
                }
            });
        if !open {
            self.profiles.confirm_switch = None;
        }
    }

    /// 「新建方案」窗口：输入名字，回车或点「创建」。
    fn ui_new_profile_window(&mut self, ctx: &egui::Context) {
        if !self.profiles.new_open {
            return;
        }
        let mut open = true;
        egui::Window::new("新建配置方案")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .order(Self::TOKENS_WINDOW_ORDER)
            .constrain_to(ctx.content_rect())
            .frame(self.floating_window_frame(ctx))
            .show(ctx, |ui| {
                ui.label("方案名会作为配置目录下的子目录名：不能为空、不能以点开头、不能含路径分隔符等非法字符。");
                ui.add_space(crate::theme::SPACE_2);
                let name_resp = ui.add(
                    egui::TextEdit::singleline(&mut self.profiles.new_name)
                        .hint_text("例如：work")
                        .desired_width(220.0),
                );
                // 回车直接创建（egui 单行编辑回车即失焦）。
                let enter = name_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(crate::theme::SPACE_2);
                ui.horizontal(|ui| {
                    let name = self.profiles.new_name.trim().to_string();
                    let create = ui.add_enabled(
                        !name.is_empty(),
                        egui::Button::new(egui::RichText::new("创建").strong()),
                    );
                    if (create.clicked() || enter) && !name.is_empty() {
                        match ProfileStore::open().create(&name) {
                            Ok(meta) => {
                                self.profiles.error = None;
                                self.profiles.new_open = false;
                                self.profiles.new_name.clear();
                                self.status = format!("已创建配置方案「{}」", meta.name);
                            }
                            Err(e) => self.profiles.error = Some(e),
                        }
                    }
                    if ui.button("取消").clicked() {
                        self.profiles.new_open = false;
                        self.profiles.new_name.clear();
                    }
                });
                if let Some(e) = &self.profiles.error {
                    ui.colored_label(crate::theme::semantics(ui).err, e);
                }
            });
        if !open {
            self.profiles.new_open = false;
            self.profiles.new_name.clear();
        }
    }

    /// 「删除方案」窗口：下拉选择 + 勾选确认，防止手滑。
    fn ui_delete_profile_window(&mut self, ctx: &egui::Context) {
        if !self.profiles.delete_open {
            return;
        }
        let mut open = true;
        egui::Window::new("删除配置方案")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .order(Self::TOKENS_WINDOW_ORDER)
            .constrain_to(ctx.content_rect())
            .frame(self.floating_window_frame(ctx))
            .show(ctx, |ui| {
                let store = ProfileStore::open();
                let names: Vec<String> = store.list().into_iter().map(|m| m.name).collect();
                if names.is_empty() {
                    ui.label("还没有可删除的方案。");
                } else {
                    let pick = egui::ComboBox::from_id_salt("profiles_delete_pick").selected_text(
                        if self.profiles.delete_name.is_empty() {
                            "选择方案…".to_string()
                        } else {
                            self.profiles.delete_name.clone()
                        },
                    );
                    let changed = pick.show_ui(ui, |ui| {
                        for name in &names {
                            ui.selectable_value(&mut self.profiles.delete_name, name.clone(), name);
                        }
                    });
                    if changed.response.changed() {
                        // 换了目标就重置确认勾选。
                        self.profiles.delete_checked = false;
                    }
                    let checked_text = if self.profiles.delete_name.is_empty() {
                        "先选择要删除的方案".to_string()
                    } else {
                        format!(
                            "我确认删除「{}」（连同其内容目录）",
                            self.profiles.delete_name
                        )
                    };
                    ui.add_enabled(
                        !self.profiles.delete_name.is_empty(),
                        egui::Checkbox::new(&mut self.profiles.delete_checked, checked_text),
                    );
                    ui.add_space(crate::theme::SPACE_2);
                    ui.horizontal(|ui| {
                        let can_delete =
                            !self.profiles.delete_name.is_empty() && self.profiles.delete_checked;
                        if ui
                            .add_enabled(
                                can_delete,
                                egui::Button::new(egui::RichText::new("删除").strong()),
                            )
                            .clicked()
                        {
                            match store.delete(&self.profiles.delete_name, true) {
                                Ok(()) => {
                                    let name = std::mem::take(&mut self.profiles.delete_name);
                                    self.profiles.error = None;
                                    self.profiles.delete_open = false;
                                    self.profiles.delete_checked = false;
                                    self.status = format!("已删除配置方案「{name}」");
                                }
                                Err(e) => self.profiles.error = Some(e),
                            }
                        }
                        if ui.button("取消").clicked() {
                            self.profiles.delete_open = false;
                            self.profiles.delete_name.clear();
                            self.profiles.delete_checked = false;
                        }
                    });
                }
                if let Some(e) = &self.profiles.error {
                    ui.colored_label(crate::theme::semantics(ui).err, e);
                }
            });
        if !open {
            self.profiles.delete_open = false;
            self.profiles.delete_name.clear();
            self.profiles.delete_checked = false;
        }
    }

    /// 「备份管理」悬浮窗：当前目标文件的备份列表 + 恢复 / 清理。
    fn ui_backups_window(&mut self, ctx: &egui::Context) {
        if !self.profiles.show_backups {
            return;
        }
        let area = ctx.content_rect();
        let height = (area.height() - 140.0).clamp(240.0, 520.0);
        let mut open = true;
        let size = egui::vec2(620.0, height);
        let centered = area.center() - size / 2.0;
        egui::Window::new("备份管理")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .order(Self::TOKENS_WINDOW_ORDER)
            .fixed_size(size)
            .constrain_to(area)
            .current_pos(centered)
            .frame(self.floating_window_frame(ctx))
            .show(ctx, |ui| {
                self.ui_backups_panel(ui);
            });
        if !open {
            self.close_backups();
        }
    }

    /// 关闭备份管理窗口并清空临时选中态。
    fn close_backups(&mut self) {
        self.profiles.show_backups = false;
        self.profiles.selected = None;
        self.profiles.note = None;
        self.profiles.error = None;
    }

    /// 备份管理窗口的内容：目标文件、备份列表、恢复与清理。
    fn ui_backups_panel(&mut self, ui: &mut egui::Ui) {
        let target = self.page_save_path(self.current_page);
        let target_path = match &target {
            PageTarget::Current(p) | PageTarget::Modified(p) | PageTarget::Default(p) => p.clone(),
        };
        ui.horizontal(|ui| {
            ui.add(
                egui::Label::new(egui::RichText::new(format!("目标文件：{target_path}")).weak())
                    .truncate(),
            );
            if ui
                .button("清理")
                .on_hover_text(
                    "按默认保留策略清理备份：自动快照留最近 20 条、\
                     恢复前备份留 2 条、手动备份留 30 天。",
                )
                .clicked()
            {
                match crate::profiles::prune(&self.backups_root(), PrunePolicy::default()) {
                    Ok(0) => self.profiles.note = Some("没有需要清理的备份".to_string()),
                    Ok(n) => self.profiles.note = Some(format!("已清理 {n} 个过期备份")),
                    Err(e) => self.profiles.error = Some(format!("清理失败：{e}")),
                }
            }
        });
        ui.separator();
        let Some(target_name) = Path::new(&target_path)
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
        else {
            ui.label("当前页面没有写入目标：先在顶栏填好配置文件路径。");
            return;
        };
        let backups = list_backups_for(&self.backups_root(), &target_name);
        if backups.is_empty() {
            ui.label(
                egui::RichText::new(format!(
                    "「{target_name}」还没有备份。保存时会自动生成「自动」备份；\
                     恢复前也会先给当前内容留一份「恢复前」备份。"
                ))
                .weak(),
            );
        }
        // 备份列表：时间倒序，tag 标签着色。
        egui::ScrollArea::vertical()
            .id_salt("profiles_backups_list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for entry in &backups {
                    ui.horizontal(|ui| {
                        let is_selected =
                            self.profiles.selected.as_deref() == Some(entry.file_name.as_str());
                        let tag_text = if entry.pinned {
                            format!("{}（固定）", tag_label(entry.tag))
                        } else {
                            tag_label(entry.tag).to_string()
                        };
                        if ui
                            .selectable_label(is_selected, &entry.file_name)
                            .on_hover_text(&entry.file_name)
                            .clicked()
                        {
                            self.profiles.selected = Some(entry.file_name.clone());
                        }
                        ui.label(
                            egui::RichText::new(format!("[{tag_text}]"))
                                .small()
                                .color(tag_color(ui, entry.tag)),
                        );
                    });
                }
            });
        ui.separator();
        ui.horizontal(|ui| {
            let has_selection = self.profiles.selected.is_some();
            if ui
                .add_enabled(
                    has_selection,
                    egui::Button::new(egui::RichText::new("恢复").strong()),
                )
                .on_hover_text("选中一条备份后恢复：会先弹出「当前文件 → 备份内容」的对比预览")
                .clicked()
            {
                self.open_restore_confirm();
            }
            ui.label(
                egui::RichText::new("恢复前会自动把当前内容备份成「恢复前」，可再次恢复撤销。")
                    .small()
                    .weak(),
            );
        });
        if let Some(note) = &self.profiles.note {
            ui.colored_label(crate::theme::semantics(ui).ok, note);
        }
        if let Some(e) = &self.profiles.error {
            ui.colored_label(crate::theme::semantics(ui).err, e);
        }
    }

    /// 打开恢复确认框：先算好「当前文件 → 备份内容」的 diff 缓存。
    fn open_restore_confirm(&mut self) {
        let Some(backup) = self.profiles.selected.clone() else {
            return;
        };
        let target = self.page_save_path(self.current_page);
        let target = match &target {
            PageTarget::Current(p) | PageTarget::Modified(p) | PageTarget::Default(p) => p.clone(),
        };
        let backup_path = self.backups_root().join(&backup);
        let Ok(backup_content) = std::fs::read_to_string(&backup_path) else {
            self.profiles
                .error
                .replace(format!("读取备份失败（{}）", backup_path.display()));
            return;
        };
        // 当前文件读不出（还不存在 / 无权限）按空内容比。
        let current_content = crate::util::read_config_content(&target).unwrap_or_default();
        let (lines, summary) =
            diff::diff_hunks(&current_content, &backup_content, diff::CONTEXT_LINES);
        self.profiles.confirm_restore = Some(RestoreConfirm {
            backup,
            target,
            lines,
            summary,
        });
    }

    /// 「从备份恢复」确认框：diff 预览 + 明确告知会覆盖当前文件。
    fn ui_restore_confirm_window(&mut self, ctx: &egui::Context) {
        let Some(confirm) = self.profiles.confirm_restore.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new("从备份恢复")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .order(Self::TOKENS_WINDOW_ORDER)
            .constrain_to(ctx.content_rect())
            .frame(self.floating_window_frame(ctx))
            .show(ctx, |ui| {
                ui.label(format!("备份：{}", confirm.backup));
                ui.label(format!("将覆盖当前文件：{}", confirm.target));
                ui.add_space(crate::theme::SPACE_2);
                ui.label(
                    egui::RichText::new(
                        "下面是「当前文件 → 备份内容」的差异，确认后当前文件会变成备份内容：",
                    )
                    .small()
                    .weak(),
                );
                draw_diff_lines(ui, &confirm.lines, confirm.summary);
                ui.add_space(crate::theme::SPACE_2);
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::new(egui::RichText::new("确认恢复").strong()))
                        .clicked()
                    {
                        self.do_restore(&confirm);
                    }
                    if ui.button("取消").clicked() {
                        self.profiles.confirm_restore = None;
                    }
                });
                if let Some(e) = &self.profiles.error {
                    ui.colored_label(crate::theme::semantics(ui).err, e);
                }
            });
        if !open {
            self.profiles.confirm_restore = None;
        }
    }

    /// 执行恢复：core::restore 在恢复前会自动生成 BeforeRestore 保险快照。
    fn do_restore(&mut self, confirm: &RestoreConfirm) {
        let backup_path = self.backups_root().join(&confirm.backup);
        match crate::profiles::restore(&backup_path, Path::new(&confirm.target)) {
            Ok(()) => {
                // 磁盘内容变了：对比视图缓存作废（签名含写盘计数）。
                self.save_serial = self.save_serial.wrapping_add(1);
                self.profiles.error = None;
                self.profiles.confirm_restore = None;
                self.profiles.selected = None;
                let note = format!(
                    "已从「{}」恢复。恢复前的内容已自动存为「恢复前」备份，\
                     选中它再恢复一次即可撤销。",
                    confirm.backup
                );
                self.profiles.note = Some(note.clone());
                self.status = note;
            }
            Err(e) => self.profiles.error = Some(format!("恢复失败：{e}")),
        }
    }

    // ---------- 保存挂点 ----------

    /// 自动快照目录：单测可注入 tempdir，生产用配置目录下的 backups/。
    pub(in crate::app) fn backups_root(&self) -> PathBuf {
        self.snapshots_root
            .clone()
            .unwrap_or_else(crate::profiles::default_backups_root)
    }

    /// 保存写盘前的自动快照（Auto tag）。
    ///
    /// 走 core 的外部修改检测：磁盘内容与上次记录的 hash 不同（或本会话从没
    /// 快照过）才备份——我们自己刚写入的内容不重复留底，只有被外部改过的
    /// 文件才在被覆盖前备份一份。快照失败一律静默：备份是保险措施，不能挡住
    /// 保存主流程。WSL 路径由 wsl 命令读写、core 快照只走本地 fs，直接跳过。
    pub(in crate::app) fn auto_snapshot_before_save(&mut self, path: &str) {
        if path.trim().is_empty() || crate::util::is_wsl_path(path) {
            return;
        }
        let target = Path::new(path);
        let last_hash = self.snapshot_hashes.get(path).cloned().unwrap_or_default();
        let backups_root = self.backups_root();
        if crate::profiles::snapshot_if_changed_into(&backups_root, target, &last_hash).is_some() {
            self.note_snapshot_hash(path);
        }
    }

    /// 记录目标文件当前内容的 hash（写盘成功后调用），供下次保存判定「外部改动」。
    pub(in crate::app) fn note_snapshot_hash(&mut self, path: &str) {
        if path.trim().is_empty() || crate::util::is_wsl_path(path) {
            return;
        }
        if let Some(hash) = crate::profiles::content_hash(Path::new(path)) {
            self.snapshot_hashes.insert(path.to_string(), hash);
        }
    }

    /// 保存成功后的备份清理：按默认保留策略（Auto 留最近 20 条等）。失败静默。
    pub(in crate::app) fn prune_backups_after_save(&self) {
        let _ = crate::profiles::prune(&self.backups_root(), PrunePolicy::default());
    }
}

/// 渲染 diff 显示行：配色与标记和预览面板的对比视图一致。
fn draw_diff_lines(ui: &mut egui::Ui, lines: &[diff::DiffLine], summary: diff::DiffSummary) {
    let semantics = crate::theme::semantics(ui);
    if summary.is_empty() {
        ui.colored_label(semantics.ok, "备份内容与当前文件一致。");
        return;
    }
    ui.horizontal(|ui| {
        ui.colored_label(
            semantics.ok,
            egui::RichText::new(format!("+{}", summary.added)).monospace(),
        )
        .on_hover_text("恢复会新增的行数");
        ui.colored_label(
            semantics.err,
            egui::RichText::new(format!("-{}", summary.removed)).monospace(),
        )
        .on_hover_text("恢复会移除的行数");
    });
    let weak = ui.visuals().weak_text_color();
    egui::ScrollArea::vertical()
        .id_salt("profiles_restore_diff")
        .max_height(260.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            for line in lines {
                let (color, text) = match line.kind {
                    diff::LineKind::Added => (semantics.ok, format!("+{}", line.text)),
                    diff::LineKind::Removed => (semantics.err, format!("-{}", line.text)),
                    diff::LineKind::Hunk => (semantics.info, line.text.clone()),
                    diff::LineKind::Context => (weak, format!(" {}", line.text)),
                };
                ui.label(egui::RichText::new(text).monospace().color(color));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 独立 tempdir：不碰真实用户目录，先清后建。
    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "modelharbor-app-profiles-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn backup_file_name_is_parsed_from_the_right() {
        let parsed = parse_backup_file_name("opencode.json.20240101_120000.auto.bak").unwrap();
        assert_eq!(parsed.stem, "opencode.json");
        assert_eq!(parsed.ts, "20240101_120000");
        assert_eq!(parsed.tag, SnapshotTag::Auto);
        assert!(!parsed.pinned);

        // stem 带点 + pin 后缀。
        let parsed = parse_backup_file_name("a.b.json.20240101_120000.manual.pin.bak").unwrap();
        assert_eq!(parsed.stem, "a.b.json");
        assert!(parsed.pinned);
        assert_eq!(parsed.tag, SnapshotTag::Manual);

        // 四种 tag 都认识。
        for (text, tag) in [
            ("auto", SnapshotTag::Auto),
            ("manual", SnapshotTag::Manual),
            ("preswitch", SnapshotTag::PreSwitch),
            ("beforerestore", SnapshotTag::BeforeRestore),
        ] {
            let name = format!("cfg.json.20240101_120000.{text}.bak");
            assert_eq!(
                parse_backup_file_name(&name).map(|p| p.tag),
                Some(tag),
                "{name}"
            );
        }

        // 非本约定命名的文件一律拒收。
        assert!(parse_backup_file_name("notes.txt").is_none());
        assert!(parse_backup_file_name("cfg.auto.bak").is_none(), "缺时间戳");
        assert!(
            parse_backup_file_name("cfg.20240101_120000.bogus.bak").is_none(),
            "tag 不认识"
        );
        assert!(
            parse_backup_file_name(".20240101_120000.auto.bak").is_none(),
            "stem 不能为空"
        );
        assert!(
            parse_backup_file_name("cfg.2024-101_000000.auto.bak").is_none(),
            "时间戳夹了非数字"
        );
        assert!(
            parse_backup_file_name("cfg.20240101_1200.auto.bak").is_none(),
            "时间戳长度不对"
        );
    }

    #[test]
    fn backups_are_filtered_by_target_and_sorted_newest_first() {
        let dir = scratch_dir("list");
        std::fs::create_dir_all(&dir).expect("建备份目录");
        let write = |name: &str| std::fs::write(dir.join(name), "x").expect("写备份");
        write("cfg.json.20240101_120000.auto.bak");
        write("cfg.json.20240102_000000.auto.bak");
        write("cfg.json.20240101_110000.manual.pin.bak");
        write("cfg.json.20240101_000000.beforerestore.bak");
        // 其他目标 / 非备份文件都不进列表。
        write("other.json.20240103_000000.auto.bak");
        write("readme.txt");

        let entries = list_backups_for(&dir, "cfg.json");
        let names: Vec<&str> = entries.iter().map(|e| e.file_name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "cfg.json.20240102_000000.auto.bak",
                "cfg.json.20240101_120000.auto.bak",
                "cfg.json.20240101_110000.manual.pin.bak",
                "cfg.json.20240101_000000.beforerestore.bak",
            ],
            "按时间倒序（新的在前）"
        );
        assert!(entries[2].pinned, "pin 标记要保留下来");

        assert!(
            list_backups_for(&dir, "missing.json").is_empty(),
            "目标名不同不进列表"
        );
        assert!(
            list_backups_for(&dir.join("nope"), "cfg.json").is_empty(),
            "目录不存在返回空表"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
