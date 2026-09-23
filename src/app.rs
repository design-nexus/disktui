use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config::Config;
use crate::fstab::{self, Change, Share, ShareDraft};
use crate::model::{
    self, ActionId, Block, Caps, Model, PropVal, Row, RowKind, PART_TYPES,
};
use crate::msg::{Msg, Progress};
use crate::op::{self, Op};
use crate::privilege::SudoJob;
use crate::theme::{self, Theme};
use crate::util::{self, human_size, kernel_name, parse_size};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Devices,
    Actions,
    Shares,
}

pub enum Effect {
    None,
    Call(Op),
    Sudo(SudoJob),
}

pub struct Field {
    pub label: String,
    pub value: String,
    pub choices: Vec<String>,
    pub secret: bool,
    pub toggle: bool,
}

pub struct Form {
    pub title: String,
    pub blurb: String,
    pub fields: Vec<Field>,
    pub focus: usize,
    pub id: FormId,
}

pub enum FormId {
    Format { path: String },
    FormatDisk { path: String },
    Create {
        disk: String,
        offset: u64,
        max: u64,
        table: String,
    },
    Resize {
        path: String,
        current: u64,
        filesystem: bool,
        mounted: bool,
        fstype: String,
    },
    Label { path: String, swap: bool },
    MountOptions { path: String },
    Unlock { path: String },
    ChangePass { path: String },
    EditPartition { path: String, table: String },
    Image { path: String, size: u64, restore: bool },
    Benchmark { path: String, size: u64 },
    DriveSettings { path: String },
    RaidCreate,
    Share,
    LoopAttach,
    Header { path: String, restore: bool },
    SmartTest { path: String, nvme: bool },
    Sanitize { path: String },
    SecureErase { path: String },
    Convert { path: String },
}

pub struct Confirm {
    pub title: String,
    pub body: String,
    pub kernel: String,
    pub system: bool,
    pub typed: String,
    pub ack: String,
    pub which: u8,
    pub op: Op,
}

pub enum Dialog {
    None,
    Form(Form),
    Confirm(Confirm),
    Text { title: String, body: String, scroll: u16 },
}

pub struct App {
    pub config: Config,
    pub config_path: PathBuf,
    pub theme: Theme,
    pub omarchy: Option<Theme>,
    pub custom: Option<Theme>,
    pub model: Model,
    pub caps: Caps,
    pub rows: Vec<Row>,
    pub selected: usize,
    pub action_index: usize,
    pub focus: Focus,
    pub shares: Vec<Share>,
    pub share_index: usize,
    pub fstab: String,
    pub dialog: Dialog,
    pub help: bool,
    pub picking_theme: bool,
    pub theme_index: usize,
    pub info: String,
    pub error: String,
    pub progress: Option<Progress>,
    pub samples: Vec<(f64, f64)>,
    pub quit: bool,
    pub busy: bool,
    pub cancel: Arc<AtomicBool>,
    pub pending: Effect,
    pub attach: Option<String>,
    pub select_path: Option<String>,
    pub smart_target: Option<(String, bool)>,
    uid: u32,
    gid: u32,
}

impl App {
    pub fn new(
        config: Config,
        config_path: PathBuf,
        theme: Theme,
        omarchy: Option<Theme>,
        custom: Option<Theme>,
        attach: Option<String>,
    ) -> Self {
        let (uid, gid) = util::current_ids();
        let mut app = Self {
            config,
            config_path,
            theme,
            omarchy,
            custom,
            model: Model::default(),
            caps: Caps::default(),
            rows: Vec::new(),
            selected: 0,
            action_index: 0,
            focus: Focus::Devices,
            shares: Vec::new(),
            share_index: 0,
            fstab: String::new(),
            dialog: Dialog::None,
            help: false,
            picking_theme: false,
            theme_index: 0,
            info: "Reading disks…".into(),
            error: String::new(),
            progress: None,
            samples: Vec::new(),
            quit: false,
            busy: false,
            cancel: Arc::new(AtomicBool::new(false)),
            pending: Effect::None,
            attach,
            select_path: None,
            smart_target: None,
            uid,
            gid,
        };
        app.reload_fstab();
        app.rebuild_rows(None);
        app
    }

    pub fn take_effect(&mut self) -> Effect {
        std::mem::replace(&mut self.pending, Effect::None)
    }

    pub fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Model(model) => {
                // The empty model only has the Shares row. Don't stick to it once disks arrive.
                let had_devices = !self.model.drives.is_empty()
                    || !self.model.blocks.is_empty()
                    || !self.model.raids.is_empty();
                let keep = if had_devices {
                    self.rows.get(self.selected).map(row_key)
                } else {
                    None
                };
                let jump = self.select_path.take();
                self.model = model;
                self.rebuild_rows(keep);
                if let Some(path) = jump {
                    if let Some(index) = self.rows.iter().position(|row| match &row.kind {
                        RowKind::Volume { block } => block == &path,
                        _ => false,
                    }) {
                        self.selected = index;
                    }
                }
                if let Some(file) = self.attach.take() {
                    self.queue(Op::LoopSetup {
                        file,
                        read_only: false,
                    });
                }
                if self.info == "Reading disks…" {
                    self.info.clear();
                }
            }
            Msg::Caps(caps) => self.caps = caps,
            Msg::Info(text) => {
                self.error.clear();
                self.info = text;
            }
            Msg::Error(text) => self.error = text,
            Msg::Progress(progress) => self.progress = progress,
            Msg::Sample { pos, mbps } => self.samples.push((pos, mbps)),
            Msg::Smart(text) => {
                self.dialog = Dialog::Text {
                    title: "SMART".into(),
                    body: text,
                    scroll: 0,
                };
            }
            Msg::ThemeReload => self.reload_theme(),
            Msg::Busy(busy) => self.busy = busy,
            Msg::Attached(path) => self.select_path = Some(path),
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.help {
            self.help = false;
            return;
        }
        if self.picking_theme {
            self.key_theme(key);
            return;
        }
        match &self.dialog {
            Dialog::Text { .. } => self.key_text(key),
            Dialog::Confirm(_) => self.key_confirm(key),
            Dialog::Form(_) => self.key_form(key),
            Dialog::None => self.key_main(key),
        }
    }

    fn key_main(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc && self.progress.is_some() {
            self.cancel.store(true, Ordering::Relaxed);
            self.info = "Cancelling…".into();
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('t') => self.open_themes(),
            KeyCode::Char('1') => {
                self.focus = Focus::Devices;
                if matches!(self.current_kind(), Some(RowKind::Shares)) {
                    self.selected = 0;
                }
            }
            KeyCode::Char('2') => self.focus_shares(),
            KeyCode::Tab => self.cycle(1),
            KeyCode::BackTab => self.cycle(-1),
            KeyCode::Char('j') | KeyCode::Down => self.move_sel(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_sel(-1),
            KeyCode::Enter => self.activate(),
            KeyCode::Char('x') => self.cancel_job(),
            KeyCode::Char('U') => self.force_unmount(),
            other => {
                if let KeyCode::Char(ch) = other {
                    self.shortcut(ch);
                }
            }
        }
    }

    fn cycle(&mut self, dir: i32) {
        self.focus = match (self.focus, dir > 0) {
            (Focus::Devices, true) => {
                if matches!(self.current_kind(), Some(RowKind::Shares)) {
                    Focus::Shares
                } else {
                    Focus::Actions
                }
            }
            (Focus::Actions, true) => Focus::Devices,
            (Focus::Shares, true) => Focus::Devices,
            (Focus::Devices, false) => {
                if matches!(self.current_kind(), Some(RowKind::Shares)) {
                    Focus::Shares
                } else {
                    Focus::Actions
                }
            }
            (Focus::Actions, false) => Focus::Devices,
            (Focus::Shares, false) => Focus::Devices,
        };
    }

    fn move_sel(&mut self, dir: i32) {
        match self.focus {
            Focus::Actions => {
                let len = self.actions().len();
                if len == 0 {
                    return;
                }
                self.action_index = slide(self.action_index, len, dir);
            }
            Focus::Shares => {
                if self.shares.is_empty() {
                    return;
                }
                self.share_index = slide(self.share_index, self.shares.len(), dir);
            }
            Focus::Devices => {
                if self.rows.is_empty() {
                    return;
                }
                self.selected = slide(self.selected, self.rows.len(), dir);
                self.action_index = 0;
                if matches!(self.current_kind(), Some(RowKind::Shares)) {
                    self.focus = Focus::Shares;
                }
            }
        }
    }

    fn activate(&mut self) {
        match self.focus {
            Focus::Actions => {
                let action = self.actions().get(self.action_index).cloned();
                if let Some(action) = action {
                    if action.enabled {
                        self.open_action(action.id);
                    } else {
                        self.error = action.reason;
                    }
                }
            }
            Focus::Shares => self.open_share(false),
            Focus::Devices => match self.current_kind() {
                Some(RowKind::Shares) => self.focus = Focus::Shares,
                Some(RowKind::Free { .. }) => self.open_action(ActionId::CreatePartition),
                Some(RowKind::Volume { .. }) => {
                    if self.action_enabled(ActionId::Unlock) {
                        self.open_action(ActionId::Unlock);
                    } else if self.action_enabled(ActionId::Mount) {
                        self.open_action(ActionId::Mount);
                    } else if self.action_enabled(ActionId::Unmount) {
                        self.open_action(ActionId::Unmount);
                    }
                }
                _ => self.focus = Focus::Actions,
            },
        }
    }

    fn shortcut(&mut self, ch: char) {
        if self.focus == Focus::Shares {
            match ch {
                'n' => self.open_share(true),
                'e' => self.open_share(false),
                'd' => self.delete_share(),
                'm' => self.mount_share(true),
                'u' => self.mount_share(false),
                _ => {}
            }
            return;
        }
        let Some(action) = self.actions().into_iter().find(|action| action.id.key() == ch.to_string() && !action.id.key().is_empty()) else {
            return;
        };
        if action.enabled {
            self.open_action(action.id);
        } else if !action.reason.is_empty() {
            self.error = action.reason;
        }
    }

    fn key_form(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.dialog = Dialog::None,
            KeyCode::Up | KeyCode::BackTab => self.form_move(-1),
            KeyCode::Down | KeyCode::Tab => self.form_move(1),
            KeyCode::Left => self.form_cycle(-1),
            KeyCode::Right => self.form_cycle(1),
            KeyCode::Char(' ') => self.form_space(),
            KeyCode::Enter => self.submit_form(),
            KeyCode::Backspace => self.form_backspace(),
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.form_insert(ch);
            }
            _ => {}
        }
    }

    fn key_confirm(&mut self, key: KeyEvent) {
        let Dialog::Confirm(confirm) = &mut self.dialog else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.dialog = Dialog::None,
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                if confirm.system {
                    confirm.which = 1 - confirm.which;
                }
            }
            KeyCode::Backspace => {
                let field = if confirm.which == 0 { &mut confirm.typed } else { &mut confirm.ack };
                field.pop();
            }
            KeyCode::Enter => self.submit_confirm(),
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let field = if confirm.which == 0 { &mut confirm.typed } else { &mut confirm.ack };
                field.push(ch);
            }
            _ => {}
        }
    }

    fn key_text(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => self.dialog = Dialog::None,
            KeyCode::Char('j') | KeyCode::Down => self.text_scroll(1),
            KeyCode::Char('k') | KeyCode::Up => self.text_scroll(-1),
            KeyCode::Char('g') => self.text_scroll(0),
            KeyCode::Char('a') => {
                if let Some((path, nvme)) = self.smart_target.clone() {
                    self.dialog = Dialog::None;
                    self.queue(Op::SmartAbort { path, nvme });
                }
            }
            _ => {}
        }
    }

    fn text_scroll(&mut self, dir: i32) {
        let Dialog::Text { scroll, .. } = &mut self.dialog else {
            return;
        };
        if dir == 0 {
            *scroll = 0;
        } else if dir < 0 {
            *scroll = scroll.saturating_sub(1);
        } else {
            *scroll = scroll.saturating_add(1);
        }
    }

    fn key_theme(&mut self, key: KeyEvent) {
        let len = theme::CHOICES.len();
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.picking_theme = false,
            KeyCode::Char('j') | KeyCode::Down => self.theme_index = slide(self.theme_index, len, 1),
            KeyCode::Char('k') | KeyCode::Up => self.theme_index = slide(self.theme_index, len, -1),
            KeyCode::Enter => {
                if let Some(choice) = theme::CHOICES.get(self.theme_index) {
                    self.apply_theme(choice.id);
                }
            }
            _ => {}
        }
    }

    fn open_themes(&mut self) {
        self.theme_index = theme::CHOICES
            .iter()
            .position(|c| c.id == self.config.theme)
            .unwrap_or(0);
        self.picking_theme = true;
    }

    fn apply_theme(&mut self, id: &str) {
        if id == "omarchy" && self.omarchy.is_none() {
            self.error = "Omarchy theme was not found.".into();
            return;
        }
        if id == "custom" && self.custom.is_none() {
            self.error = "No ~/.config/disktui/theme.toml yet.".into();
            return;
        }
        self.config.theme = id.to_string();
        self.theme = theme::resolve(id, self.omarchy.as_ref(), self.custom.as_ref());
        if let Err(err) = self.config.save(&self.config_path) {
            self.error = format!("Could not save the theme: {err}");
        }
        self.info = format!("Theme: {}", self.theme.name);
        self.picking_theme = false;
    }

    fn reload_theme(&mut self) {
        if let Some((name, theme, _)) = theme::find_omarchy() {
            let mut theme = theme;
            theme.name = format!("Omarchy · {name}");
            self.omarchy = Some(theme);
        }
        self.custom = theme::load_user_theme();
        self.theme = theme::resolve(&self.config.theme, self.omarchy.as_ref(), self.custom.as_ref());
    }

    pub fn actions(&self) -> Vec<model::Action> {
        let Some(row) = self.rows.get(self.selected) else {
            return Vec::new();
        };
        self.model.actions(&self.caps, &row.kind)
    }

    fn action_enabled(&self, id: ActionId) -> bool {
        self.actions().iter().any(|a| a.id == id && a.enabled)
    }

    fn current_kind(&self) -> Option<RowKind> {
        self.rows.get(self.selected).map(|row| row.kind.clone())
    }

    fn open_action(&mut self, id: ActionId) {
        if self.busy {
            self.error = "Wait for the current operation to finish.".into();
            return;
        }
        let kind = self.current_kind();
        let block = self.current_block();
        let drive = self.current_drive();
        match id {
            ActionId::Mount => {
                if let Some(block) = block {
                    self.queue(Op::Mount(block.path));
                }
            }
            ActionId::Unmount => {
                if let Some(block) = block {
                    self.queue(Op::Unmount { path: block.path, force: false });
                }
            }
            ActionId::SwapOn => {
                if let Some(block) = block {
                    self.queue(Op::SwapStart(block.path));
                }
            }
            ActionId::SwapOff => {
                if let Some(block) = block {
                    self.queue(Op::SwapStop(block.path));
                }
            }
            ActionId::Lock => {
                if let Some(block) = block {
                    self.queue(Op::Lock(block.path));
                }
            }
            ActionId::TakeOwnership => {
                if let Some(block) = block {
                    self.queue(Op::TakeOwnership(block.path));
                }
            }
            ActionId::Check => {
                if let Some(block) = block {
                    self.queue(Op::Check(block.path));
                }
            }
            ActionId::Repair => {
                if let Some(block) = &block {
                    self.confirm_op(
                        "Repair filesystem",
                        "Repair can change data on the filesystem.",
                        Op::Repair(block.path.clone()),
                    );
                }
            }
            ActionId::Standby => {
                if let Some(drive) = drive {
                    self.queue(Op::Standby(drive.path));
                }
            }
            ActionId::Wakeup => {
                if let Some(drive) = drive {
                    self.queue(Op::Wakeup(drive.path));
                }
            }
            ActionId::Eject => {
                if let Some(drive) = drive {
                    self.queue(Op::Eject(drive.path));
                }
            }
            ActionId::Rescan => {
                if let Some(block) = block {
                    self.queue(Op::Rescan(block.path));
                }
            }
            ActionId::DetachLoop => {
                if let Some(block) = block {
                    self.queue(Op::LoopDelete(block.path));
                }
            }
            ActionId::RaidStart => {
                if let Some(RowKind::Raid { raid }) = kind {
                    self.queue(Op::RaidStart { path: raid, degraded: true });
                }
            }
            ActionId::RaidStop => {
                if let Some(RowKind::Raid { raid }) = kind {
                    self.queue(Op::RaidStop(raid));
                }
            }
            ActionId::PowerOff => {
                if let Some(drive) = drive {
                    self.confirm_op(
                        "Power off",
                        "The drive disappears until it is plugged in again.",
                        Op::PowerOff(drive.path),
                    );
                }
            }
            ActionId::Delete | ActionId::RaidDelete => self.confirm_current(id),
            ActionId::Format => self.form_format(false),
            ActionId::FormatDisk => self.form_format(true),
            ActionId::CreatePartition => self.form_create(),
            ActionId::Resize => self.form_resize(),
            ActionId::Label => self.form_label(),
            ActionId::MountOptions => self.form_mount_options(),
            ActionId::Unlock => self.form_unlock(),
            ActionId::ChangePassphrase => self.form_pass(),
            ActionId::EditPartition => self.form_edit_partition(),
            ActionId::Benchmark => self.form_benchmark(),
            ActionId::CreateImage => self.form_image(false),
            ActionId::RestoreImage => self.form_image(true),
            ActionId::Smart => self.open_smart(false),
            ActionId::SmartTest => self.form_smart_test(),
            ActionId::DriveSettings => self.form_drive(),
            ActionId::AttachImage => self.form_loop(),
            ActionId::SecureErase => self.form_secure_erase(),
            ActionId::Sanitize => self.form_sanitize(),
            ActionId::RaidCreate => self.form_raid(),
            ActionId::HeaderBackup => self.form_header(false),
            ActionId::RestoreHeader => self.form_header(true),
            ActionId::ConvertLuks => self.form_convert(),
        }
    }

    fn confirm_current(&mut self, id: ActionId) {
        let op = match id {
            ActionId::Delete => {
                let Some(block) = self.current_block() else { return };
                Op::DeletePartition { path: block.path }
            }
            ActionId::RaidDelete => {
                let Some(RowKind::Raid { raid }) = self.current_kind() else { return };
                Op::RaidDelete(raid)
            }
            _ => return,
        };
        self.confirm_op("Delete", "This destroys the data on the selection.", op);
    }

    fn confirm_op(&mut self, title: &str, body: &str, op: Op) {
        let kind = self.current_kind();
        let kernel = kind.as_ref().map(|k| self.model.confirm_name(k)).unwrap_or_default();
        let system = kind.as_ref().is_some_and(|k| self.model.row_is_system(k));
        let kernel = if kernel.is_empty() { "raid".into() } else { kernel };
        self.dialog = Dialog::Confirm(Confirm {
            title: title.into(),
            body: body.into(),
            kernel,
            system,
            typed: String::new(),
            ack: String::new(),
            which: 0,
            op,
        });
    }

    fn submit_confirm(&mut self) {
        let Dialog::Confirm(confirm) = &self.dialog else { return };
        let ok = model::confirm_ok(&confirm.kernel, &confirm.typed, confirm.system, &confirm.ack);
        if !ok {
            self.error = if confirm.system {
                format!("Type {} and then system.", confirm.kernel)
            } else {
                format!("Type {} to confirm.", confirm.kernel)
            };
            return;
        }
        let Dialog::Confirm(confirm) = std::mem::replace(&mut self.dialog, Dialog::None) else {
            return;
        };
        self.queue(confirm.op);
    }

    fn form_format(&mut self, disk: bool) {
        let Some(block) = self.current_block() else {
            self.error = "No disk selected.".into();
            return;
        };
        let kinds: Vec<String> = if disk {
            ["gpt", "dos", "empty"].into_iter().map(str::to_string).collect()
        } else {
            model::FS_TYPES.iter().map(|s| (*s).to_string()).collect()
        };
        self.dialog = Dialog::Form(Form {
            title: if disk { "Format disk".into() } else { "Format".into() },
            blurb: format!(
                "{}  {}  {}",
                kernel_name(block.device_path()),
                human_size(block.size),
                if self.model.is_system_tree(&block.path) { "This is a system disk." } else { "" }
            ),
            fields: vec![
                choice("Type", kinds, if disk { "gpt" } else { "ext4" }),
                text("Label", &block.id_label),
                choice("Erase", ["none", "zero", "ata-secure-erase", "ata-secure-erase-enhanced"].map(str::to_string).to_vec(), "none"),
                toggle("Encrypt with LUKS2", false),
                secret("Passphrase"),
                toggle("Take ownership", true),
            ],
            focus: 0,
            id: if disk { FormId::FormatDisk { path: block.path } } else { FormId::Format { path: block.path } },
        });
    }

    fn form_create(&mut self) {
        let (disk, offset, max) = match self.current_kind() {
            Some(RowKind::Free { disk, start, size }) => (disk, start, size),
            Some(RowKind::Drive { drive }) => {
                let Some(block) = self.model.disk_for_drive(&drive) else {
                    self.error = "This drive has no whole-disk device.".into();
                    return;
                };
                let Some(free) = self.model.segments_for(&block.path).into_iter().find(|s| s.free) else {
                    self.error = "No free space. Format a partition table first if the disk is empty.".into();
                    return;
                };
                (block.path.clone(), free.start, free.size)
            }
            _ => {
                self.error = "Select free space or a drive.".into();
                return;
            }
        };
        let table = self.model.block(&disk).and_then(|b| b.table.as_ref().map(|t| t.kind.clone())).unwrap_or_else(|| "gpt".into());
        let labels: Vec<String> = PART_TYPES.iter().map(|t| t.label.to_string()).collect();
        let mut fs: Vec<String> = model::FS_TYPES.iter().map(|s| (*s).to_string()).collect();
        fs.insert(0, "none".into());
        self.dialog = Dialog::Form(Form {
            title: "Create partition".into(),
            blurb: format!("Free space {} at offset {}.", human_size(max), human_size(offset)),
            fields: vec![
                text("Size (blank = all)", ""),
                text("Name", ""),
                choice("Partition type", labels, PART_TYPES[0].label),
                choice("Filesystem", fs, "ext4"),
                text("Label", ""),
                toggle("Encrypt with LUKS2", false),
                secret("Passphrase"),
            ],
            focus: 0,
            id: FormId::Create { disk, offset, max, table },
        });
    }

    fn form_resize(&mut self) {
        let Some(block) = self.current_block() else { return };
        let current = block.partition.as_ref().map(|p| p.size).unwrap_or(block.size);
        self.dialog = Dialog::Form(Form {
            title: "Resize".into(),
            blurb: format!("Current size {}. 0 or max uses the largest fit. Shrinking a filesystem moves data.", human_size(current)),
            fields: vec![text("New size", "")],
            focus: 0,
            id: FormId::Resize {
                path: block.path,
                current,
                filesystem: block.filesystem,
                mounted: !block.mount_points.is_empty(),
                fstype: block.id_type,
            },
        });
    }

    fn form_label(&mut self) {
        let Some(block) = self.current_block() else { return };
        self.dialog = Dialog::Form(Form {
            title: "Label".into(),
            blurb: String::new(),
            fields: vec![text("Label", &block.id_label)],
            focus: 0,
            id: FormId::Label { path: block.path, swap: block.swap },
        });
    }

    fn form_mount_options(&mut self) {
        let Some(block) = self.current_block() else { return };
        let existing = block.fstab_items().into_iter().next();
        let dir = existing.map(|item| model::fstab_field(item, "dir")).unwrap_or_default();
        let opts = existing.map(|item| model::fstab_field(item, "opts")).unwrap_or_else(|| "defaults".into());
        let pass = existing.map(|item| model::fstab_field(item, "passno")).unwrap_or_else(|| "2".into());
        let startup = existing.is_some();
        self.dialog = Dialog::Form(Form {
            title: "Mount options".into(),
            blurb: "Writes an fstab item through UDisks. Turn startup off to remove it.".into(),
            fields: vec![
                toggle("Mount at startup", startup),
                text("Mount point", &dir),
                text("Options", &opts),
                choice("Identify by", ["UUID", "LABEL"].map(str::to_string).to_vec(), "UUID"),
                text("fsck pass", &pass),
            ],
            focus: 0,
            id: FormId::MountOptions { path: block.path },
        });
    }

    fn form_unlock(&mut self) {
        let Some(block) = self.current_block() else { return };
        self.dialog = Dialog::Form(Form {
            title: "Unlock".into(),
            blurb: kernel_name(block.device_path()).into(),
            fields: vec![secret("Passphrase")],
            focus: 0,
            id: FormId::Unlock { path: block.path },
        });
    }

    fn form_pass(&mut self) {
        let Some(block) = self.current_block() else { return };
        self.dialog = Dialog::Form(Form {
            title: "Change passphrase".into(),
            blurb: String::new(),
            fields: vec![secret("Current"), secret("New"), secret("Repeat")],
            focus: 0,
            id: FormId::ChangePass { path: block.path },
        });
    }

    fn form_edit_partition(&mut self) {
        let Some(block) = self.current_block() else { return };
        let Some(part) = &block.partition else { return };
        let table = self.model.block(&part.table).and_then(|d| d.table.as_ref().map(|t| t.kind.clone())).unwrap_or_else(|| "gpt".into());
        let labels: Vec<String> = PART_TYPES.iter().map(|t| t.label.to_string()).collect();
        let current = model::partition_type_label(&part.type_code);
        let mut fields = vec![
            choice("Type", labels, &current),
            text("Name", &part.name),
        ];
        if table == "dos" {
            fields.push(toggle("Bootable", part.flags & (1 << 7) != 0));
        } else {
            fields.push(toggle("System partition", part.flags & 1 != 0));
            fields.push(toggle("Hidden", part.flags & (1 << 62) != 0));
            fields.push(toggle("Do not automount", part.flags & (1 << 63) != 0));
            fields.push(toggle("Read-only", part.flags & (1 << 60) != 0));
        }
        self.dialog = Dialog::Form(Form {
            title: "Edit partition".into(),
            blurb: format!("Current type {}", part.type_code),
            fields,
            focus: 0,
            id: FormId::EditPartition { path: block.path, table },
        });
    }

    fn form_benchmark(&mut self) {
        let Some(block) = self.current_block() else { return };
        self.dialog = Dialog::Form(Form {
            title: "Benchmark".into(),
            blurb: "Read walks the device. Write destroys its contents.".into(),
            fields: vec![choice("Mode", ["read", "write"].map(str::to_string).to_vec(), "read")],
            focus: 0,
            id: FormId::Benchmark { path: block.path, size: block.size },
        });
    }

    fn form_image(&mut self, restore: bool) {
        let Some(block) = self.current_block() else { return };
        self.dialog = Dialog::Form(Form {
            title: if restore { "Restore disk image".into() } else { "Create disk image".into() },
            blurb: format!("{}  {}", kernel_name(block.device_path()), human_size(block.size)),
            fields: vec![text("Image file", "")],
            focus: 0,
            id: FormId::Image { path: block.path, size: block.size, restore },
        });
    }

    fn open_smart(&mut self, _test: bool) {
        let Some(drive) = self.current_drive() else {
            self.error = "Select a drive.".into();
            return;
        };
        self.smart_target = Some((drive.path.clone(), drive.nvme));
        self.queue(Op::Smart { path: drive.nvme_path, nvme: drive.nvme });
    }

    fn form_smart_test(&mut self) {
        let Some(drive) = self.current_drive() else { return };
        let kinds: Vec<String> = if drive.nvme {
            ["short", "extended", "vendor-specific"].map(str::to_string).to_vec()
        } else {
            ["short", "extended", "conveyance", "offline"].map(str::to_string).to_vec()
        };
        self.dialog = Dialog::Form(Form {
            title: "SMART self-test".into(),
            blurb: "The test runs on the drive and returns immediately.".into(),
            fields: vec![choice("Test", kinds, "short")],
            focus: 0,
            id: FormId::SmartTest { path: drive.path, nvme: drive.nvme },
        });
    }

    fn form_drive(&mut self) {
        let Some(drive) = self.current_drive() else { return };
        let standby = drive_int(&drive.config, "ata-pm-standby");
        let apm = drive_int(&drive.config, "ata-apm-level");
        let cache = drive_bool(&drive.config, "ata-write-cache-enabled").unwrap_or(drive.ata.as_ref().is_some_and(|a| a.write_cache_enabled));
        let look = drive_bool(&drive.config, "ata-read-lookahead-enabled").unwrap_or(drive.ata.as_ref().is_some_and(|a| a.lookahead_enabled));
        self.dialog = Dialog::Form(Form {
            title: "Drive settings".into(),
            blurb: "Standby is in seconds. 0 disables spindown. APM is 1–254.".into(),
            fields: vec![
                text("Standby seconds", &standby),
                text("APM level", &apm),
                toggle("Write cache", cache),
                toggle("Read look-ahead", look),
            ],
            focus: 0,
            id: FormId::DriveSettings { path: drive.path },
        });
    }

    fn form_loop(&mut self) {
        self.dialog = Dialog::Form(Form {
            title: "Attach disk image".into(),
            blurb: "The image is attached as a loop device.".into(),
            fields: vec![text("Image file", ""), toggle("Read only", true)],
            focus: 0,
            id: FormId::LoopAttach,
        });
    }

    fn form_secure_erase(&mut self) {
        let Some(drive) = self.current_drive() else { return };
        let minutes = drive.ata.as_ref().map(|a| a.secure_erase_minutes).unwrap_or(0);
        self.dialog = Dialog::Form(Form {
            title: "Secure erase".into(),
            blurb: format!("ATA secure erase takes about {minutes} minutes and destroys every byte."),
            fields: vec![toggle("Enhanced erase", false)],
            focus: 0,
            id: FormId::SecureErase { path: drive.path },
        });
    }

    fn form_sanitize(&mut self) {
        let Some(drive) = self.current_drive() else { return };
        self.dialog = Dialog::Form(Form {
            title: "NVMe sanitize".into(),
            blurb: "Sanitize cannot be aborted once it starts.".into(),
            fields: vec![choice(
                "Action",
                ["block-erase", "crypto-erase", "overwrite"].map(str::to_string).to_vec(),
                "block-erase",
            )],
            focus: 0,
            id: FormId::Sanitize { path: drive.nvme_path },
        });
    }

    fn form_raid(&mut self) {
        self.dialog = Dialog::Form(Form {
            title: "Create RAID".into(),
            blurb: "Member devices are erased. List kernel names separated by spaces, for example: sdb sdc".into(),
            fields: vec![
                text("Name", "data"),
                choice("Level", ["raid0", "raid1", "raid5", "raid6", "raid10"].map(str::to_string).to_vec(), "raid1"),
                text("Devices", ""),
                text("Chunk", "512K"),
            ],
            focus: 0,
            id: FormId::RaidCreate,
        });
    }

    fn form_header(&mut self, restore: bool) {
        let Some(block) = self.current_block() else { return };
        self.dialog = Dialog::Form(Form {
            title: if restore { "Restore LUKS header".into() } else { "Back up LUKS header".into() },
            blurb: "The file holds the header and keyslots.".into(),
            fields: vec![text("File", "")],
            focus: 0,
            id: FormId::Header { path: block.path, restore },
        });
    }

    fn form_convert(&mut self) {
        let Some(block) = self.current_block() else { return };
        self.dialog = Dialog::Form(Form {
            title: "Convert LUKS".into(),
            blurb: "The volume must be locked.".into(),
            fields: vec![choice("Version", ["luks2", "luks1"].map(str::to_string).to_vec(), "luks2")],
            focus: 0,
            id: FormId::Convert { path: block.path },
        });
    }

    fn submit_form(&mut self) {
        let Dialog::Form(form) = &self.dialog else { return };
        let built = build(form, &self.model, &self.caps, self.uid, self.gid, &self.fstab);
        match built {
            Build::Err(text) => self.error = text,
            Build::Op(op) => {
                self.dialog = Dialog::None;
                self.queue(op);
            }
            Build::Confirm { title, body, op, kernel, system } => {
                let kind = self.current_kind();
                let mut kernel = if kernel.is_empty() {
                    kind.as_ref().map(|k| self.model.confirm_name(k)).unwrap_or_default()
                } else {
                    kernel
                };
                let system = if kernel == "raid" {
                    system
                } else {
                    kind.as_ref().is_some_and(|k| self.model.row_is_system(k)) || system
                };
                if kernel.is_empty() {
                    kernel = "confirm".into();
                }
                self.dialog = Dialog::Confirm(Confirm {
                    title,
                    body,
                    kernel,
                    system,
                    typed: String::new(),
                    ack: String::new(),
                    which: 0,
                    op,
                });
            }
            Build::Sudo(job) => {
                self.dialog = Dialog::None;
                self.pending = Effect::Sudo(job);
            }
        }
    }

    fn queue(&mut self, op: Op) {
        self.error.clear();
        self.busy = true;
        self.cancel = Arc::new(AtomicBool::new(false));
        let op = match op {
            Op::Benchmark { path, write, size, .. } => {
                self.samples.clear();
                Op::Benchmark { path, write, size, cancel: Arc::clone(&self.cancel) }
            }
            Op::Backup { path, file, size, .. } => {
                self.samples.clear();
                Op::Backup { path, file, size, cancel: Arc::clone(&self.cancel) }
            }
            Op::Restore { path, file, size, .. } => {
                self.samples.clear();
                Op::Restore { path, file, size, cancel: Arc::clone(&self.cancel) }
            }
            other => other,
        };
        self.pending = Effect::Call(op);
    }

    fn form_move(&mut self, dir: i32) {
        let Dialog::Form(form) = &mut self.dialog else { return };
        if form.fields.is_empty() { return; }
        form.focus = slide(form.focus, form.fields.len(), dir);
    }

    fn form_cycle(&mut self, dir: i32) {
        let Dialog::Form(form) = &mut self.dialog else { return };
        let Some(field) = form.fields.get_mut(form.focus) else { return };
        if field.toggle {
            field.value = if field.value == "yes" { "no".into() } else { "yes".into() };
        } else if !field.choices.is_empty() {
            let index = field.choices.iter().position(|c| c == &field.value).unwrap_or(0);
            let next = slide(index, field.choices.len(), dir);
            field.value = field.choices[next].clone();
        }
    }

    fn form_space(&mut self) {
        let choice = match &self.dialog {
            Dialog::Form(form) => form.fields.get(form.focus).is_some_and(|field| field.toggle || !field.choices.is_empty()),
            _ => false,
        };
        if choice {
            self.form_cycle(1);
        } else {
            self.form_insert(' ');
        }
    }

    fn form_insert(&mut self, ch: char) {
        let Dialog::Form(form) = &mut self.dialog else { return };
        let Some(field) = form.fields.get_mut(form.focus) else { return };
        if field.toggle || !field.choices.is_empty() {
            return;
        }
        field.value.push(ch);
    }

    fn form_backspace(&mut self) {
        let Dialog::Form(form) = &mut self.dialog else { return };
        let Some(field) = form.fields.get_mut(form.focus) else { return };
        if field.toggle || !field.choices.is_empty() {
            return;
        }
        field.value.pop();
    }

    fn focus_shares(&mut self) {
        if let Some(index) = self.rows.iter().position(|r| matches!(r.kind, RowKind::Shares)) {
            self.selected = index;
        }
        self.focus = Focus::Shares;
    }

    fn open_share(&mut self, fresh: bool) {
        let share = if fresh { None } else { self.shares.get(self.share_index).cloned() };
        let protocol = share.as_ref().map(|s| s.fstype.clone()).unwrap_or_else(|| "cifs".into());
        let extra = share.as_ref().map(|s| s.options.clone()).unwrap_or_default();
        self.dialog = Dialog::Form(Form {
            title: if fresh { "Add network share".into() } else { "Edit network share".into() },
            blurb: "The password is stored in a root-only credentials file, not in fstab.".into(),
            fields: vec![
                choice("Protocol", ["cifs", "nfs", "nfs4", "fuse.sshfs", "custom"].map(str::to_string).to_vec(), if ["cifs", "nfs", "nfs4", "fuse.sshfs"].contains(&protocol.as_str()) { &protocol } else { "custom" }),
                text("Type", if ["cifs", "nfs", "nfs4", "fuse.sshfs"].contains(&protocol.as_str()) { "" } else { &protocol }),
                text("Source", share.as_ref().map(|s| s.source.as_str()).unwrap_or("")),
                text("Mount point", share.as_ref().map(|s| s.target.as_str()).unwrap_or("/mnt/")),
                toggle("Guest", extra.split(',').any(|o| o == "guest")),
                text("Username", ""),
                secret("Password"),
                text("Credentials file", opt_of(&extra, "credentials").unwrap_or_default().as_str()),
                text("SSH identity", opt_of(&extra, "IdentityFile").unwrap_or_default().as_str()),
                text("Extra options", &extra),
                toggle("_netdev", extra.contains("_netdev") || fresh),
                toggle("nofail", extra.contains("nofail") || fresh),
                toggle("Automount", extra.contains("x-systemd.automount")),
                text("Idle timeout", &opt_of(&extra, "x-systemd.idle-timeout").unwrap_or_default()),
                choice("SMB version", ["default", "3.1.1", "3.0", "2.1"].map(str::to_string).to_vec(), &opt_of(&extra, "vers").unwrap_or_else(|| "default".into())),
                toggle("Mount now", false),
                text("Id", share.as_ref().map(|s| s.id.as_str()).unwrap_or("")),
            ],
            focus: 0,
            id: FormId::Share,
        });
    }

    fn delete_share(&mut self) {
        let Some(share) = self.shares.get(self.share_index).cloned() else {
            self.error = "No share selected.".into();
            return;
        };
        let next = fstab::transform(&self.fstab, &Change::Delete {
            id: share.id,
            source: share.source,
            target: share.target.clone(),
        });
        self.pending = Effect::Sudo(SudoJob::ApplyFstab {
            contents: next,
            creds: None,
            mount: None,
        });
        self.info = format!("Removing {}", share.target);
    }

    fn mount_share(&mut self, mount: bool) {
        let Some(share) = self.shares.get(self.share_index).cloned() else { return };
        if mount {
            if let Some(missing) = fstab::missing_helper(&share.fstype) {
                self.error = missing;
                return;
            }
            self.pending = Effect::Sudo(SudoJob::Mount(share.target));
        } else {
            self.pending = Effect::Sudo(SudoJob::Umount(share.target));
        }
    }

    fn force_unmount(&mut self) {
        let Some(block) = self.current_block() else { return };
        self.queue(Op::Unmount { path: block.path, force: true });
    }

    fn cancel_job(&mut self) {
        if let Some(job) = self.model.jobs.iter().find(|j| j.cancelable).cloned() {
            self.queue(Op::CancelJob(job.path));
            return;
        }
        if self.progress.is_some() {
            self.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn current_block(&self) -> Option<Block> {
        match self.current_kind()? {
            RowKind::Volume { block } => self.model.block(&block).cloned(),
            RowKind::Drive { drive } => self.model.disk_for_drive(&drive).cloned(),
            RowKind::Free { disk, .. } => self.model.block(&disk).cloned(),
            RowKind::Raid { raid } => self.model.blocks.iter().find(|b| b.mdraid == raid).cloned(),
            RowKind::Shares => None,
        }
    }

    fn current_drive(&self) -> Option<model::Drive> {
        let drive = match self.current_kind()? {
            RowKind::Drive { drive } => drive,
            RowKind::Volume { block } | RowKind::Free { disk: block, .. } => {
                self.model.block(&block)?.drive.clone()
            }
            _ => return None,
        };
        self.model.drive(&drive).cloned()
    }

    fn rebuild_rows(&mut self, keep: Option<String>) {
        self.rows = self.model.rows();
        if let Some(keep) = keep {
            if let Some(index) = self.rows.iter().position(|row| row_key(row) == keep) {
                self.selected = index;
            }
        }
        if self.selected >= self.rows.len() {
            self.selected = self.rows.len().saturating_sub(1);
        }
    }

    pub fn reload_fstab(&mut self) {
        self.fstab = std::fs::read_to_string("/etc/fstab").unwrap_or_default();
        self.shares = fstab::network_shares(&self.fstab);
        if self.share_index >= self.shares.len() {
            self.share_index = self.shares.len().saturating_sub(1);
        }
    }
}

fn slide(index: usize, len: usize, dir: i32) -> usize {
    if len == 0 {
        return 0;
    }
    if dir < 0 {
        index.saturating_sub(1)
    } else {
        (index + 1).min(len - 1)
    }
}

fn row_key(row: &Row) -> String {
    match &row.kind {
        RowKind::Drive { drive } => format!("d:{drive}"),
        RowKind::Volume { block } => format!("v:{block}"),
        RowKind::Free { disk, start, .. } => format!("f:{disk}:{start}"),
        RowKind::Raid { raid } => format!("r:{raid}"),
        RowKind::Shares => "shares".into(),
    }
}

fn text(label: &str, value: &str) -> Field {
    Field { label: label.into(), value: value.into(), choices: Vec::new(), secret: false, toggle: false }
}
fn secret(label: &str) -> Field {
    Field { label: label.into(), value: String::new(), choices: Vec::new(), secret: true, toggle: false }
}
fn toggle(label: &str, on: bool) -> Field {
    Field { label: label.into(), value: if on { "yes".into() } else { "no".into() }, choices: Vec::new(), secret: false, toggle: true }
}
fn choice(label: &str, choices: Vec<String>, current: &str) -> Field {
    let value = if choices.iter().any(|c| c == current) { current.to_string() } else { choices.first().cloned().unwrap_or_default() };
    Field { label: label.into(), value, choices, secret: false, toggle: false }
}

fn yes(form: &Form, label: &str) -> bool {
    form.fields.iter().any(|f| f.label == label && f.value == "yes")
}
fn val<'a>(form: &'a Form, label: &str) -> &'a str {
    form.fields.iter().find(|f| f.label == label).map(|f| f.value.as_str()).unwrap_or("")
}

enum Build {
    Op(Op),
    Confirm {
        title: String,
        body: String,
        op: Op,
        kernel: String,
        system: bool,
    },
    Sudo(SudoJob),
    Err(String),
}

fn build(form: &Form, model: &Model, caps: &Caps, uid: u32, gid: u32, fstab_text: &str) -> Build {
    match &form.id {
        FormId::Format { path } | FormId::FormatDisk { path } => {
            let kind = val(form, "Type").to_string();
            if let Some(tool) = caps.format.get(&kind) {
                if !tool.available {
                    return Build::Err(format!("install {}", tool.missing));
                }
            }
            let op = Op::Format(op::FormatReq {
                path: path.clone(),
                kind,
                label: val(form, "Label").into(),
                erase: val(form, "Erase").into(),
                encrypt: yes(form, "Encrypt with LUKS2"),
                passphrase: val(form, "Passphrase").into(),
                take_ownership: yes(form, "Take ownership"),
            });
            Build::Confirm { title: "Format".into(), body: "Formatting erases what is on this device.".into(), op, kernel: String::new(), system: false }
        }
        FormId::Create { disk, offset, max, table } => {
            let size = match parse_size(val(form, "Size (blank = all)")) {
                Some(0) => 0,
                Some(size) if size > *max => return Build::Err(format!("Size is larger than the free space ({}).", human_size(*max))),
                Some(size) => size,
                None => return Build::Err("Could not read the size. Try 20G or max.".into()),
            };
            let fstype = val(form, "Filesystem");
            let type_label = val(form, "Partition type");
            let type_code = if fstype == "swap" {
                model::type_code_for(table, "Linux swap")
            } else {
                model::type_code_for(table, type_label)
            };
            Build::Op(Op::CreatePartition(op::CreateReq {
                disk: disk.clone(),
                offset: *offset,
                size,
                type_code,
                name: val(form, "Name").into(),
                fstype: if fstype == "none" { String::new() } else { fstype.into() },
                label: val(form, "Label").into(),
                encrypt: yes(form, "Encrypt with LUKS2"),
                passphrase: val(form, "Passphrase").into(),
            }))
        }
        FormId::Resize { path, current, filesystem, mounted, fstype } => {
            let Some(new_size) = parse_size(val(form, "New size")) else {
                return Build::Err("Could not read the size.".into());
            };
            if *filesystem {
                if let Some(cap) = caps.resize.get(fstype.as_str()) {
                    let grow = new_size == 0 || new_size > *current;
                    if !cap.allows(*mounted, grow) {
                        let why = if cap.missing.is_empty() { "this filesystem cannot be resized that way".into() } else { format!("install {}", cap.missing) };
                        return Build::Err(why);
                    }
                }
            }
            Build::Op(Op::Resize { path: path.clone(), current: *current, new_size, filesystem: *filesystem })
        }
        FormId::Label { path, swap } => Build::Op(Op::SetLabel { path: path.clone(), label: val(form, "Label").into(), swap: *swap }),
        FormId::MountOptions { path } => {
            let Some(block) = model.block(path) else {
                return Build::Err("Device disappeared.".into());
            };
            let existing: Vec<_> = block.config.iter().filter(|c| c.kind == "fstab").cloned().collect();
            if !yes(form, "Mount at startup") {
                return Build::Op(Op::ClearFstab { path: path.clone(), items: existing });
            }
            let dir = val(form, "Mount point");
            if !dir.starts_with('/') {
                return Build::Err("Mount point must be an absolute path.".into());
            }
            let by_label = val(form, "Identify by") == "LABEL";
            let fsname = block.fstab_name(by_label);
            let passno = val(form, "fsck pass").parse().unwrap_or(2);
            Build::Op(Op::MountOptions(op::MountOptReq {
                path: path.clone(),
                existing,
                fsname,
                dir: dir.into(),
                fstype: if block.id_type.is_empty() { "auto".into() } else { block.id_type.clone() },
                opts: {
                    let opts = val(form, "Options");
                    if opts.is_empty() { "defaults".into() } else { opts.into() }
                },
                passno,
            }))
        }
        FormId::Unlock { path } => Build::Op(Op::Unlock { path: path.clone(), passphrase: val(form, "Passphrase").into() }),
        FormId::ChangePass { path } => {
            let new = val(form, "New");
            if new != val(form, "Repeat") {
                return Build::Err("The new passphrases do not match.".into());
            }
            Build::Op(Op::ChangePassphrase { path: path.clone(), old: val(form, "Current").into(), new: new.into() })
        }
        FormId::EditPartition { path, table } => {
            let Some(block) = model.block(path) else { return Build::Err("Device disappeared.".into()); };
            let mut flags = block.partition.as_ref().map(|p| p.flags).unwrap_or(0);
            if table == "dos" {
                flags &= !(1 << 7);
                if yes(form, "Bootable") { flags |= 1 << 7; }
            } else {
                for (label, bit) in [("System partition", 0u32), ("Hidden", 62), ("Do not automount", 63), ("Read-only", 60)] {
                    flags &= !(1 << bit);
                    if yes(form, label) { flags |= 1 << bit; }
                }
            }
            Build::Op(Op::EditPartition {
                path: path.clone(),
                type_code: model::type_code_for(table, val(form, "Type")),
                name: val(form, "Name").into(),
                flags,
            })
        }
        FormId::Image { path, size, restore } => {
            let file = val(form, "Image file");
            if file.is_empty() { return Build::Err("Choose an image file.".into()); }
            let op = if *restore {
                Op::Restore { path: path.clone(), file: file.into(), size: *size, cancel: Arc::new(AtomicBool::new(false)) }
            } else {
                Op::Backup { path: path.clone(), file: file.into(), size: *size, cancel: Arc::new(AtomicBool::new(false)) }
            };
            if *restore {
                Build::Confirm { title: "Restore image".into(), body: "Restoring overwrites the device.".into(), op, kernel: String::new(), system: false }
            } else {
                Build::Op(op)
            }
        }
        FormId::Benchmark { path, size } => {
            let write = val(form, "Mode") == "write";
            let op = Op::Benchmark { path: path.clone(), write, size: *size, cancel: Arc::new(AtomicBool::new(false)) };
            if write {
                Build::Confirm { title: "Write benchmark".into(), body: "A write benchmark destroys the contents of the device.".into(), op, kernel: String::new(), system: false }
            } else {
                Build::Op(op)
            }
        }
        FormId::DriveSettings { path } => {
            let Some(drive) = model.drive(path) else { return Build::Err("Drive disappeared.".into()); };
            let mut pairs = drive.config.clone();
            set_i32(&mut pairs, "ata-pm-standby", val(form, "Standby seconds"));
            set_i32(&mut pairs, "ata-apm-level", val(form, "APM level"));
            set_bool(&mut pairs, "ata-write-cache-enabled", yes(form, "Write cache"));
            set_bool(&mut pairs, "ata-read-lookahead-enabled", yes(form, "Read look-ahead"));
            Build::Op(Op::DriveConfig { path: path.clone(), pairs })
        }
        FormId::RaidCreate => {
            let name = val(form, "Name").to_string();
            if name.is_empty() { return Build::Err("Give the array a name.".into()); }
            let devices: Vec<String> = val(form, "Devices").split_whitespace().map(|name| {
                let kernel = name.trim_start_matches("/dev/");
                model.blocks.iter().find(|b| kernel_name(b.device_path()) == kernel).map(|b| b.path.clone())
            }).collect::<Option<Vec<_>>>().unwrap_or_default();
            if devices.len() < 2 {
                return Build::Err("Name at least two devices that are visible in the list.".into());
            }
            let chunk = parse_size(val(form, "Chunk")).unwrap_or(0);
            let system = devices.iter().any(|path| model.is_system_tree(path));
            let op = Op::RaidCreate { devices, level: val(form, "Level").into(), name: name.clone(), chunk };
            Build::Confirm { title: "Create RAID".into(), body: "Creating an array erases every member device. Type raid to confirm.".into(), op, kernel: "raid".into(), system }
        }
        FormId::Share => share_build(form, uid, gid, fstab_text),
        FormId::LoopAttach => {
            let file = val(form, "Image file");
            if file.is_empty() { return Build::Err("Choose an image file.".into()); }
            Build::Op(Op::LoopSetup { file: file.into(), read_only: yes(form, "Read only") })
        }
        FormId::Header { path, restore } => {
            let file = val(form, "File");
            if file.is_empty() { return Build::Err("Choose a file.".into()); }
            if *restore {
                Build::Confirm {
                    title: "Restore header".into(),
                    body: "Restoring a header replaces the LUKS header on the device.".into(),
                    op: Op::RestoreHeader { path: path.clone(), file: file.into() },
                    kernel: String::new(),
                    system: false,
                }
            } else {
                Build::Op(Op::HeaderBackup { path: path.clone(), file: file.into() })
            }
        }
        FormId::SmartTest { path, nvme } => Build::Op(Op::SmartTest { path: path.clone(), kind: val(form, "Test").into(), nvme: *nvme }),
        FormId::Sanitize { path } => Build::Confirm {
            title: "NVMe sanitize".into(),
            body: "Sanitize destroys all data on the controller and cannot be cancelled.".into(),
            op: Op::Sanitize { path: path.clone(), action: val(form, "Action").into() },
            kernel: String::new(),
            system: false,
        },
        FormId::SecureErase { path } => Build::Confirm {
            title: "Secure erase".into(),
            body: "Secure erase destroys all data on the drive.".into(),
            op: Op::SecureErase { path: path.clone(), enhanced: yes(form, "Enhanced erase") },
            kernel: String::new(),
            system: false,
        },
        FormId::Convert { path } => Build::Op(Op::ConvertLuks { path: path.clone(), version: val(form, "Version").into() }),
    }
}

fn share_build(form: &Form, uid: u32, gid: u32, fstab_text: &str) -> Build {
    let protocol = if val(form, "Protocol") == "custom" {
        val(form, "Type").to_string()
    } else {
        val(form, "Protocol").to_string()
    };
    let draft = ShareDraft {
        id: val(form, "Id").into(),
        protocol,
        source: val(form, "Source").into(),
        mount_point: val(form, "Mount point").into(),
        guest: yes(form, "Guest"),
        username: val(form, "Username").into(),
        password: val(form, "Password").into(),
        credentials_file: val(form, "Credentials file").into(),
        identity_file: val(form, "SSH identity").into(),
        extra: val(form, "Extra options").into(),
        netdev: yes(form, "_netdev"),
        nofail: yes(form, "nofail"),
        automount: yes(form, "Automount"),
        idle_timeout: val(form, "Idle timeout").into(),
        vers: val(form, "SMB version").into(),
        uid,
        gid,
    };
    let plan = match fstab::plan_share(&draft) {
        Ok(plan) => plan,
        Err(err) => return Build::Err(err),
    };
    if plan.entry_line.contains("DISKTUI_") || plan.credentials.as_ref().is_some_and(|c| c.contents.contains("DISKTUI_")) {
        return Build::Err("A field contains a reserved marker.".into());
    }
    let contents = fstab::transform(fstab_text, &Change::Upsert {
        id: plan.id,
        source: plan.source,
        target: plan.target.clone(),
        line: plan.entry_line,
    });
    let mount = yes(form, "Mount now").then(|| plan.target);
    Build::Sudo(SudoJob::ApplyFstab { contents, creds: plan.credentials, mount })
}

fn set_i32(pairs: &mut Vec<(String, PropVal)>, key: &str, text: &str) {
    if text.trim().is_empty() {
        pairs.retain(|(k, _)| k != key);
        return;
    }
    let Ok(value) = text.trim().parse::<i32>() else { return };
    if let Some((_, slot)) = pairs.iter_mut().find(|(k, _)| k == key) {
        *slot = PropVal::I32(value);
    } else {
        pairs.push((key.into(), PropVal::I32(value)));
    }
}

fn set_bool(pairs: &mut Vec<(String, PropVal)>, key: &str, value: bool) {
    if let Some((_, slot)) = pairs.iter_mut().find(|(k, _)| k == key) {
        *slot = PropVal::Bool(value);
    } else {
        pairs.push((key.into(), PropVal::Bool(value)));
    }
}

fn drive_int(pairs: &[(String, PropVal)], key: &str) -> String {
    pairs.iter().find(|(k, _)| k == key).and_then(|(_, v)| match v {
        PropVal::I32(n) => Some(n.to_string()),
        PropVal::U32(n) => Some(n.to_string()),
        PropVal::I64(n) => Some(n.to_string()),
        _ => None,
    }).unwrap_or_default()
}

fn drive_bool(pairs: &[(String, PropVal)], key: &str) -> Option<bool> {
    pairs.iter().find(|(k, _)| k == key).and_then(|(_, v)| match v {
        PropVal::Bool(b) => Some(*b),
        _ => None,
    })
}

fn opt_of(options: &str, key: &str) -> Option<String> {
    options.split(',').find_map(|opt| {
        let (k, v) = opt.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
}

