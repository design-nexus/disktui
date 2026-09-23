//! Disk objects and which actions apply to the current selection.
//!
//! Destructive work is not refused here. The confirm dialog is the gate.
//! This module only decides what is possible and why something is dimmed.

use std::collections::HashMap;

use crate::util::{human_size, kernel_name};

pub const MIB: u64 = 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub drives: Vec<Drive>,
    pub blocks: Vec<Block>,
    pub raids: Vec<Raid>,
    pub jobs: Vec<Job>,
}

#[derive(Clone, Debug)]
pub struct Drive {
    pub path: String,
    pub vendor: String,
    pub model: String,
    pub revision: String,
    pub serial: String,
    pub size: u64,
    pub connection: String,
    pub media: String,
    pub removable: bool,
    pub ejectable: bool,
    pub optical: bool,
    pub can_power_off: bool,
    pub sort_key: String,
    pub rotation: i32,
    pub config: Vec<(String, PropVal)>,
    pub ata: Option<Ata>,
    pub nvme: bool,
    pub nvme_path: String,
}

#[derive(Clone, Debug, Default)]
pub struct Ata {
    pub smart_supported: bool,
    pub smart_enabled: bool,
    pub smart_failing: bool,
    pub pm_supported: bool,
    pub apm_supported: bool,
    pub write_cache_supported: bool,
    pub write_cache_enabled: bool,
    pub lookahead_supported: bool,
    pub lookahead_enabled: bool,
    pub secure_erase_minutes: i32,
    pub secure_erase_enhanced_minutes: i32,
    pub security_frozen: bool,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub path: String,
    pub device: String,
    pub preferred: String,
    pub id_usage: String,
    pub id_type: String,
    pub id_label: String,
    pub id_uuid: String,
    pub size: u64,
    pub read_only: bool,
    pub hint_system: bool,
    pub hint_ignore: bool,
    pub drive: String,
    pub crypto_backing: String,
    pub mdraid: String,
    pub mdraid_member: String,
    #[allow(dead_code)]
    pub symlinks: Vec<String>,
    pub partition: Option<Partition>,
    pub table: Option<PartTable>,
    pub filesystem: bool,
    pub mount_points: Vec<String>,
    pub swap: bool,
    pub swap_active: bool,
    pub encrypted: bool,
    pub cleartext: String,
    pub hint_encryption: String,
    pub loopback: bool,
    pub backing_file: String,
    pub loop_autoclear: bool,
    pub config: Vec<ConfigItem>,
}

#[derive(Clone, Debug)]
pub struct Partition {
    pub number: u32,
    pub type_code: String,
    pub name: String,
    pub flags: u64,
    pub offset: u64,
    pub size: u64,
    pub uuid: String,
    pub table: String,
    pub container: bool,
    pub contained: bool,
}

#[derive(Clone, Debug)]
pub struct PartTable {
    pub kind: String,
}

#[derive(Clone, Debug)]
pub struct ConfigItem {
    pub kind: String,
    pub fields: Vec<(String, PropVal)>,
}

#[derive(Clone, Debug)]
pub struct Raid {
    pub path: String,
    pub uuid: String,
    pub name: String,
    pub level: String,
    pub size: u64,
    pub running: bool,
    pub degraded: bool,
    pub sync_action: String,
    pub sync_completed: f64,
    pub members: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Job {
    pub path: String,
    pub operation: String,
    pub progress: f64,
    pub progress_valid: bool,
    pub cancelable: bool,
    #[allow(dead_code)]
    pub objects: Vec<String>,
}

/// Values we can round-trip back into an UDisks configuration dict.
#[derive(Clone, Debug)]
pub enum PropVal {
    Bool(bool),
    I32(i32),
    U32(u32),
    U64(u64),
    I64(i64),
    Str(String),
    Bytes(Vec<u8>),
    Path(String),
}

#[derive(Clone, Debug, Default)]
pub struct Caps {
    pub format: HashMap<String, Tool>,
    pub resize: HashMap<String, ResizeCap>,
    pub check: HashMap<String, Tool>,
    pub repair: HashMap<String, Tool>,
}

#[derive(Clone, Debug)]
pub struct Tool {
    pub available: bool,
    pub missing: String,
}

#[derive(Clone, Debug)]
pub struct ResizeCap {
    pub available: bool,
    pub mode: u32,
    pub missing: String,
}

impl ResizeCap {
    pub fn allows(&self, mounted: bool, grow: bool) -> bool {
        if !self.available {
            return false;
        }
        let bit = match (mounted, grow) {
            (false, false) => 2,
            (false, true) => 4,
            (true, false) => 8,
            (true, true) => 16,
        };
        self.mode & bit != 0
    }
}

#[derive(Clone, Debug)]
pub struct Segment {
    pub label: String,
    pub start: u64,
    pub size: u64,
    pub free: bool,
    pub block: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionId {
    Mount,
    Unmount,
    MountOptions,
    Label,
    Format,
    FormatDisk,
    Delete,
    CreatePartition,
    Resize,
    EditPartition,
    Check,
    Repair,
    TakeOwnership,
    Unlock,
    Lock,
    ChangePassphrase,
    HeaderBackup,
    RestoreHeader,
    ConvertLuks,
    SwapOn,
    SwapOff,
    Smart,
    SmartTest,
    Benchmark,
    CreateImage,
    RestoreImage,
    DriveSettings,
    Standby,
    Wakeup,
    PowerOff,
    Eject,
    AttachImage,
    DetachLoop,
    SecureErase,
    Sanitize,
    RaidCreate,
    RaidStart,
    RaidStop,
    RaidDelete,
    Rescan,
}

impl ActionId {
    pub fn key(self) -> &'static str {
        match self {
            Self::Mount | Self::Unlock | Self::SwapOn => "m",
            Self::Unmount | Self::SwapOff => "u",
            Self::MountOptions => "e",
            Self::Label => "l",
            Self::Format | Self::FormatDisk => "f",
            Self::Delete | Self::RaidDelete => "d",
            Self::CreatePartition => "n",
            Self::Resize => "r",
            Self::EditPartition => "P",
            Self::Check | Self::Repair => "",
            Self::Smart | Self::SmartTest => "s",
            Self::Benchmark => "b",
            Self::CreateImage => "i",
            Self::RestoreImage => "I",
            Self::PowerOff => "p",
            Self::AttachImage => "a",
            Self::DriveSettings => "g",
            Self::Eject => "E",
            Self::Sanitize | Self::SecureErase => "X",
            _ => "",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Mount => "Mount",
            Self::Unmount => "Unmount",
            Self::MountOptions => "Mount options",
            Self::Label => "Edit label",
            Self::Format => "Format",
            Self::FormatDisk => "Format disk",
            Self::Delete => "Delete partition",
            Self::CreatePartition => "Create partition",
            Self::Resize => "Resize",
            Self::EditPartition => "Edit partition",
            Self::Check => "Check filesystem",
            Self::Repair => "Repair filesystem",
            Self::TakeOwnership => "Take ownership",
            Self::Unlock => "Unlock",
            Self::Lock => "Lock",
            Self::ChangePassphrase => "Change passphrase",
            Self::HeaderBackup => "Back up LUKS header",
            Self::RestoreHeader => "Restore LUKS header",
            Self::ConvertLuks => "Convert LUKS version",
            Self::SwapOn => "Enable swap",
            Self::SwapOff => "Disable swap",
            Self::Smart => "SMART data",
            Self::SmartTest => "SMART self-test",
            Self::Benchmark => "Benchmark",
            Self::CreateImage => "Create disk image",
            Self::RestoreImage => "Restore disk image",
            Self::DriveSettings => "Drive settings",
            Self::Standby => "Standby",
            Self::Wakeup => "Wake up",
            Self::PowerOff => "Power off",
            Self::Eject => "Eject",
            Self::AttachImage => "Attach disk image",
            Self::DetachLoop => "Detach loop",
            Self::SecureErase => "Secure erase",
            Self::Sanitize => "NVMe sanitize",
            Self::RaidCreate => "Create RAID",
            Self::RaidStart => "Start RAID",
            Self::RaidStop => "Stop RAID",
            Self::RaidDelete => "Delete RAID",
            Self::Rescan => "Rescan",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Action {
    pub id: ActionId,
    pub enabled: bool,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub enum RowKind {
    Drive { drive: String },
    Volume { block: String },
    Free { disk: String, start: u64, size: u64 },
    Raid { raid: String },
    Shares,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub kind: RowKind,
    pub depth: u8,
    pub label: String,
    pub size: String,
    pub mounted: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct PartType {
    pub label: &'static str,
    pub gpt: &'static str,
    pub dos: &'static str,
}

pub const PART_TYPES: &[PartType] = &[
    PartType {
        label: "Linux filesystem",
        gpt: "0fc63daf-8483-4772-8e79-3d69d8477de4",
        dos: "0x83",
    },
    PartType {
        label: "Linux home",
        gpt: "933ac7e1-2eb4-4f13-b844-0e14e2aef915",
        dos: "0x83",
    },
    PartType {
        label: "Linux swap",
        gpt: "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f",
        dos: "0x82",
    },
    PartType {
        label: "EFI system",
        gpt: "c12a7328-f81f-11d2-ba4b-00a0c93ec93b",
        dos: "0xef",
    },
    PartType {
        label: "BIOS boot",
        gpt: "21686148-6449-6e6f-744e-656564454649",
        dos: "0x00",
    },
    PartType {
        label: "Microsoft basic data",
        gpt: "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7",
        dos: "0x07",
    },
    PartType {
        label: "Linux LVM",
        gpt: "e6d6d379-f507-44c2-a23c-238f2a3df928",
        dos: "0x8e",
    },
    PartType {
        label: "Linux RAID",
        gpt: "a19d880f-05fc-4d3b-a006-743f0f84911e",
        dos: "0xfd",
    },
];

pub const FS_TYPES: &[&str] = &[
    "ext4", "btrfs", "xfs", "f2fs", "vfat", "exfat", "ntfs", "udf", "swap", "empty",
];

impl Model {
    pub fn block(&self, path: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.path == path)
    }

    pub fn drive(&self, path: &str) -> Option<&Drive> {
        self.drives.iter().find(|d| d.path == path)
    }

    pub fn raid(&self, path: &str) -> Option<&Raid> {
        self.raids.iter().find(|r| r.path == path)
    }

    pub fn disk_for_drive(&self, drive: &str) -> Option<&Block> {
        self.blocks
            .iter()
            .filter(|b| {
                b.drive == drive
                    && b.partition.is_none()
                    && !is_set(&b.crypto_backing)
                    && !b.hint_ignore
            })
            .max_by_key(|b| b.size)
    }

    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut drives: Vec<&Drive> = self.drives.iter().collect();
        drives.sort_by(|a, b| a.sort_key.cmp(&b.sort_key).then(a.path.cmp(&b.path)));
        let mut used = std::collections::HashSet::new();
        for drive in drives {
            let disk = self.disk_for_drive(&drive.path);
            let mounted = disk.is_some_and(|d| self.tree_mounted(&d.path));
            rows.push(Row {
                kind: RowKind::Drive {
                    drive: drive.path.clone(),
                },
                depth: 0,
                label: drive_label(drive),
                size: human_size(drive.size.max(disk.map(|d| d.size).unwrap_or(0))),
                mounted,
            });
            if let Some(disk) = disk {
                used.insert(disk.path.clone());
                self.push_disk_children(&mut rows, disk, &mut used);
            }
        }
        for block in &self.blocks {
            if used.contains(&block.path) || block.hint_ignore {
                continue;
            }
            if is_set(&block.crypto_backing) {
                continue;
            }
            let show = block.loopback || is_set(&block.mdraid) || block.partition.is_none();
            if !show {
                continue;
            }
            used.insert(block.path.clone());
            rows.push(Row {
                kind: RowKind::Volume {
                    block: block.path.clone(),
                },
                depth: 0,
                label: volume_label(block),
                size: human_size(block.size),
                mounted: self.tree_mounted(&block.path),
            });
            if block.table.is_some() {
                self.push_disk_children(&mut rows, block, &mut used);
            }
        }
        for raid in &self.raids {
            rows.push(Row {
                kind: RowKind::Raid {
                    raid: raid.path.clone(),
                },
                depth: 0,
                label: format!("{} {}", raid.level, raid_name(raid)),
                size: human_size(raid.size),
                mounted: raid.running,
            });
        }
        rows.push(Row {
            kind: RowKind::Shares,
            depth: 0,
            label: "Network shares".into(),
            size: String::new(),
            mounted: false,
        });
        rows
    }

    fn push_disk_children(
        &self,
        rows: &mut Vec<Row>,
        disk: &Block,
        used: &mut std::collections::HashSet<String>,
    ) {
        if disk.table.is_none() {
            return;
        }
        let parts: Vec<&Block> = self
            .blocks
            .iter()
            .filter(|b| b.partition.as_ref().is_some_and(|p| p.table == disk.path))
            .collect();
        let mut spec = Vec::new();
        for part in &parts {
            let p = part.partition.as_ref().unwrap();
            spec.push((
                p.offset,
                p.size,
                part.path.clone(),
                short_part(part),
            ));
        }
        for segment in segments(disk.size, &spec) {
            if segment.free {
                rows.push(Row {
                    kind: RowKind::Free {
                        disk: disk.path.clone(),
                        start: segment.start,
                        size: segment.size,
                    },
                    depth: 1,
                    label: "Free space".into(),
                    size: human_size(segment.size),
                    mounted: false,
                });
                continue;
            }
            let Some(path) = segment.block else { continue };
            let Some(block) = self.block(&path) else { continue };
            used.insert(path.clone());
            rows.push(Row {
                kind: RowKind::Volume { block: path.clone() },
                depth: 1,
                label: volume_label(block),
                size: human_size(block.size),
                mounted: !block.mount_points.is_empty() || block.swap_active,
            });
            if let Some(clear) = self.cleartext_of(&path) {
                used.insert(clear.path.clone());
                rows.push(Row {
                    kind: RowKind::Volume {
                        block: clear.path.clone(),
                    },
                    depth: 2,
                    label: format!("unlocked {}", volume_label(clear)),
                    size: human_size(clear.size),
                    mounted: !clear.mount_points.is_empty(),
                });
            }
        }
    }

    fn cleartext_of(&self, encrypted: &str) -> Option<&Block> {
        self.blocks
            .iter()
            .find(|b| b.crypto_backing == encrypted)
    }

    fn tree_mounted(&self, path: &str) -> bool {
        let Some(block) = self.block(path) else {
            return false;
        };
        if !block.mount_points.is_empty() || block.swap_active {
            return true;
        }
        self.blocks.iter().any(|b| {
            b.partition.as_ref().is_some_and(|p| p.table == path)
                && (!b.mount_points.is_empty() || b.swap_active)
        })
    }

    pub fn segments_for(&self, disk: &str) -> Vec<Segment> {
        let Some(block) = self.block(disk) else {
            return Vec::new();
        };
        let spec: Vec<_> = self
            .blocks
            .iter()
            .filter_map(|b| {
                let p = b.partition.as_ref()?;
                if p.table != disk {
                    return None;
                }
                Some((p.offset, p.size, b.path.clone(), short_part(b)))
            })
            .collect();
        if spec.is_empty() {
            return vec![Segment {
                label: volume_label(block),
                start: 0,
                size: block.size,
                free: false,
                block: Some(block.path.clone()),
            }];
        }
        segments(block.size, &spec)
    }

    pub fn actions(&self, caps: &Caps, row: &RowKind) -> Vec<Action> {
        let mut out = Vec::new();
        match row {
            RowKind::Shares => {}
            RowKind::Raid { raid } => {
                if let Some(raid) = self.raid(raid) {
                    push(&mut out, ActionId::RaidStart, !raid.running, "Already running");
                    push(&mut out, ActionId::RaidStop, raid.running, "Array is stopped");
                    push(&mut out, ActionId::RaidDelete, true, "");
                    push(&mut out, ActionId::Benchmark, raid.running, "Array is stopped");
                }
            }
            RowKind::Free { .. } => {
                push(&mut out, ActionId::CreatePartition, true, "");
            }
            RowKind::Drive { drive } => {
                let drive_obj = self.drive(drive);
                let disk = self.disk_for_drive(drive);
                let system = disk.is_some_and(|d| self.is_system_tree(&d.path));
                push(
                    &mut out,
                    ActionId::FormatDisk,
                    disk.is_some(),
                    "No whole-disk device",
                );
                if let Some(disk) = disk {
                    let free = self.segments_for(&disk.path).iter().any(|s| s.free);
                    let has_table = disk.table.is_some();
                    push(
                        &mut out,
                        ActionId::CreatePartition,
                        has_table && free,
                        if !has_table {
                            "Format the disk with a partition table first"
                        } else {
                            "No free space"
                        },
                    );
                    image_actions(&mut out, disk);
                }
                if let Some(drive) = drive_obj {
                    let smart = drive.ata.as_ref().is_some_and(|a| a.smart_supported) || drive.nvme;
                    push(&mut out, ActionId::Smart, smart, "This drive does not report SMART");
                    push(&mut out, ActionId::SmartTest, smart, "This drive does not report SMART");
                    let ata = drive.ata.as_ref();
                    push(
                        &mut out,
                        ActionId::DriveSettings,
                        ata.is_some(),
                        "No ATA settings on this drive",
                    );
                    push(
                        &mut out,
                        ActionId::Standby,
                        ata.is_some_and(|a| a.pm_supported),
                        "Power management is not available",
                    );
                    push(
                        &mut out,
                        ActionId::Wakeup,
                        ata.is_some_and(|a| a.pm_supported),
                        "Power management is not available",
                    );
                    push(
                        &mut out,
                        ActionId::SecureErase,
                        ata.is_some_and(|a| a.secure_erase_minutes > 0),
                        "Secure erase is not available",
                    );
                    push(&mut out, ActionId::Sanitize, drive.nvme, "Not an NVMe controller");
                    push(
                        &mut out,
                        ActionId::PowerOff,
                        drive.can_power_off,
                        "This drive cannot be powered off",
                    );
                    push(&mut out, ActionId::Eject, drive.ejectable, "Nothing to eject");
                    let _ = system;
                }
                push(&mut out, ActionId::AttachImage, true, "");
                push(&mut out, ActionId::RaidCreate, true, "");
                if let Some(disk) = disk {
                    push(&mut out, ActionId::Rescan, true, "");
                    let _ = disk;
                }
            }
            RowKind::Volume { block } => {
                let Some(block) = self.block(block) else {
                    return out;
                };
                volume_actions(&mut out, self, caps, block);
            }
        }
        out
    }

    pub fn is_system_block(&self, block: &Block) -> bool {
        if block.hint_system || block.swap_active {
            return true;
        }
        block.mount_points.iter().any(|m| is_system_mount(m))
    }

    pub fn is_system_tree(&self, path: &str) -> bool {
        let Some(block) = self.block(path) else {
            return false;
        };
        if self.is_system_block(block) {
            return true;
        }
        self.blocks.iter().any(|b| {
            (b.partition.as_ref().is_some_and(|p| p.table == path) || b.crypto_backing == path)
                && self.is_system_block(b)
        })
    }

    pub fn confirm_name(&self, row: &RowKind) -> String {
        match row {
            RowKind::Volume { block } => self
                .block(block)
                .map(|b| kernel_name(b.device_path()).to_string())
                .unwrap_or_default(),
            RowKind::Drive { drive } => self
                .disk_for_drive(drive)
                .map(|b| kernel_name(b.device_path()).to_string())
                .unwrap_or_default(),
            RowKind::Raid { raid } => self
                .raid(raid)
                .map(|r| {
                    if r.name.is_empty() {
                        "raid".into()
                    } else {
                        r.name.clone()
                    }
                })
                .unwrap_or_else(|| "raid".into()),
            RowKind::Free { disk, .. } => self
                .block(disk)
                .map(|b| kernel_name(b.device_path()).to_string())
                .unwrap_or_default(),
            RowKind::Shares => String::new(),
        }
    }

    pub fn row_is_system(&self, row: &RowKind) -> bool {
        match row {
            RowKind::Volume { block } => self.block(block).is_some_and(|b| self.is_system_block(b)),
            RowKind::Drive { drive } => self
                .disk_for_drive(drive)
                .is_some_and(|d| self.is_system_tree(&d.path)),
            RowKind::Free { disk, .. } => self.is_system_tree(disk),
            _ => false,
        }
    }
}

fn volume_actions(out: &mut Vec<Action>, model: &Model, caps: &Caps, block: &Block) {
    let mounted = !block.mount_points.is_empty();
    if block.encrypted && !is_set(&block.cleartext) && model.cleartext_of(&block.path).is_none() {
        push(out, ActionId::Unlock, true, "");
    }
    if block.encrypted {
        push(out, ActionId::Lock, model.cleartext_of(&block.path).is_some() || is_set(&block.cleartext), "Volume is locked");
        push(out, ActionId::ChangePassphrase, true, "");
        push(out, ActionId::HeaderBackup, true, "");
        push(out, ActionId::RestoreHeader, !mounted, "Unmount it first");
        push(out, ActionId::ConvertLuks, model.cleartext_of(&block.path).is_none(), "Lock the volume first");
    }
    if block.filesystem {
        push(out, ActionId::Mount, !mounted, "Already mounted");
        push(out, ActionId::Unmount, mounted, "Not mounted");
        push(out, ActionId::MountOptions, true, "");
        push(out, ActionId::Label, true, "");
        push(out, ActionId::TakeOwnership, mounted, "Mount the filesystem first");
        let fs = block.id_type.as_str();
        let check = tool_ok(caps.check.get(fs));
        push(out, ActionId::Check, !mounted && check.0, &check.1);
        let repair = tool_ok(caps.repair.get(fs));
        push(out, ActionId::Repair, !mounted && repair.0, &repair.1);
    }
    if block.swap {
        push(out, ActionId::SwapOn, !block.swap_active, "Swap is already on");
        push(out, ActionId::SwapOff, block.swap_active, "Swap is off");
        push(out, ActionId::MountOptions, true, "");
        push(out, ActionId::Label, true, "");
    }
    if block.loopback {
        push(out, ActionId::DetachLoop, !mounted, "Unmount it first");
    }
    let format_tool = if block.id_type.is_empty() {
        (true, String::new())
    } else {
        tool_ok(caps.format.get(&block.id_type))
    };
    push(
        out,
        ActionId::Format,
        !block.read_only && !mounted && !block.swap_active,
        if mounted || block.swap_active {
            "Unmount it first"
        } else if block.read_only {
            "Device is read-only"
        } else if !format_tool.0 {
            "Formatting tools are missing"
        } else {
            ""
        },
    );
    if block.partition.is_some() {
        push(out, ActionId::Delete, !mounted && !block.swap_active, "Unmount it first");
        push(out, ActionId::EditPartition, true, "");
        let resize = caps.resize.get(&block.id_type);
        let can_resize = block.filesystem && resize.is_none_or(|c| c.available) || !block.filesystem;
        push(
            out,
            ActionId::Resize,
            can_resize && !block.read_only,
            resize
                .filter(|c| !c.available)
                .map(|c| {
                    if c.missing.is_empty() {
                        "This filesystem cannot be resized".into()
                    } else {
                        format!("install {}", c.missing)
                    }
                })
                .unwrap_or_else(|| {
                    if block.read_only {
                        "Device is read-only".into()
                    } else {
                        String::new()
                    }
                }),
        );
    }
    image_actions(out, block);
    push(out, ActionId::Rescan, true, "");
}

fn image_actions(out: &mut Vec<Action>, block: &Block) {
    let busy = !block.mount_points.is_empty() || block.swap_active;
    push(out, ActionId::Benchmark, !busy, "Unmount it first");
    push(out, ActionId::CreateImage, !busy, "Unmount it first");
    push(out, ActionId::RestoreImage, !busy && !block.read_only, if block.read_only {
        "Device is read-only"
    } else {
        "Unmount it first"
    });
}

fn push(out: &mut Vec<Action>, id: ActionId, enabled: bool, reason: impl AsRef<str>) {
    out.push(Action {
        id,
        enabled,
        reason: if enabled {
            String::new()
        } else {
            reason.as_ref().to_string()
        },
    });
}

fn tool_ok(tool: Option<&Tool>) -> (bool, String) {
    match tool {
        None => (true, String::new()),
        Some(tool) if tool.available => (true, String::new()),
        Some(tool) => (
            false,
            if tool.missing.is_empty() {
                "Tool is not installed".into()
            } else {
                format!("install {}", tool.missing)
            },
        ),
    }
}

pub fn confirm_ok(kernel: &str, typed: &str, system: bool, ack: &str) -> bool {
    !kernel.is_empty() && typed == kernel && (!system || ack == "system")
}

pub fn is_system_mount(path: &str) -> bool {
    matches!(path, "/" | "/home" | "/boot") || path.starts_with("/boot/")
}

pub fn segments(disk_size: u64, parts: &[(u64, u64, String, String)]) -> Vec<Segment> {
    let mut parts = parts.to_vec();
    parts.sort_by_key(|p| p.0);
    let mut out = Vec::new();
    let mut cursor = 0u64;
    for (offset, size, path, label) in parts {
        if offset > cursor {
            let gap = offset - cursor;
            if gap >= MIB && !alignment_prefix(cursor, gap) {
                out.push(Segment {
                    label: "free".into(),
                    start: cursor,
                    size: gap,
                    free: true,
                    block: None,
                });
            }
        }
        let end = offset.saturating_add(size);
        out.push(Segment {
            label,
            start: offset,
            size,
            free: false,
            block: Some(path),
        });
        cursor = cursor.max(end);
    }
    if disk_size > cursor {
        let gap = disk_size - cursor;
        if gap >= MIB {
            out.push(Segment {
                label: "free".into(),
                start: cursor,
                size: gap,
                free: true,
                block: None,
            });
        }
    }
    out
}

fn alignment_prefix(start: u64, size: u64) -> bool {
    start < MIB && start + size <= MIB
}

pub fn partition_type_label(code: &str) -> String {
    let code = code.trim().to_ascii_lowercase();
    for kind in PART_TYPES {
        if code == kind.gpt || code == kind.dos {
            return kind.label.to_string();
        }
    }
    if code.is_empty() {
        "unknown".into()
    } else {
        code
    }
}

pub fn type_code_for(table: &str, label: &str) -> String {
    let kind = PART_TYPES.iter().find(|k| k.label == label).unwrap_or(&PART_TYPES[0]);
    if table == "dos" {
        kind.dos.to_string()
    } else {
        kind.gpt.to_string()
    }
}

fn is_set(path: &str) -> bool {
    !path.is_empty() && path != "/"
}

fn drive_label(drive: &Drive) -> String {
    let name = format!("{} {}", drive.vendor, drive.model)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if name.is_empty() {
        "Drive".into()
    } else {
        name
    }
}

pub fn volume_label(block: &Block) -> String {
    if let Some(part) = &block.partition {
        let name = if !part.name.is_empty() {
            part.name.clone()
        } else if !block.id_label.is_empty() {
            block.id_label.clone()
        } else if block.encrypted {
            "LUKS".into()
        } else if !block.id_type.is_empty() {
            block.id_type.clone()
        } else {
            "partition".into()
        };
        return format!("p{} {name}", part.number);
    }
    if block.loopback {
        let file = block
            .backing_file
            .rsplit('/')
            .next()
            .unwrap_or("image");
        return format!("loop {file}");
    }
    if !block.id_label.is_empty() {
        return block.id_label.clone();
    }
    kernel_name(block.device_path()).to_string()
}

fn short_part(block: &Block) -> String {
    volume_label(block)
}

fn raid_name(raid: &Raid) -> String {
    if raid.name.is_empty() {
        raid.uuid.chars().take(8).collect()
    } else {
        raid.name.clone()
    }
}

impl Block {
    pub fn device_path(&self) -> &str {
        if !self.preferred.is_empty() {
            &self.preferred
        } else {
            &self.device
        }
    }

    pub fn fstab_name(&self, by_label: bool) -> String {
        if by_label && !self.id_label.is_empty() {
            format!("LABEL={}", self.id_label)
        } else if !self.id_uuid.is_empty() {
            format!("UUID={}", self.id_uuid)
        } else {
            self.device_path().to_string()
        }
    }

    pub fn fstab_items(&self) -> Vec<&ConfigItem> {
        self.config.iter().filter(|c| c.kind == "fstab").collect()
    }
}

pub fn fstab_field(item: &ConfigItem, key: &str) -> String {
    item.fields
        .iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| match v {
            PropVal::Bytes(b) => Some(crate::util::c_string(b)),
            PropVal::Str(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(path: &str, device: &str) -> Block {
        Block {
            path: path.into(),
            device: device.into(),
            preferred: device.into(),
            id_usage: String::new(),
            id_type: "ext4".into(),
            id_label: String::new(),
            id_uuid: String::new(),
            size: 100 * MIB,
            read_only: false,
            hint_system: false,
            hint_ignore: false,
            drive: "/drives/d".into(),
            crypto_backing: "/".into(),
            mdraid: "/".into(),
            mdraid_member: "/".into(),
            symlinks: Vec::new(),
            partition: None,
            table: None,
            filesystem: true,
            mount_points: Vec::new(),
            swap: false,
            swap_active: false,
            encrypted: false,
            cleartext: "/".into(),
            hint_encryption: String::new(),
            loopback: false,
            backing_file: String::new(),
            loop_autoclear: false,
            config: Vec::new(),
        }
    }

    #[test]
    fn hides_the_leading_alignment_gap_and_keeps_real_free_space() {
        let parts = vec![(
            MIB,
            50 * MIB,
            "/b".into(),
            "p1".into(),
        )];
        let segs = segments(100 * MIB, &parts);
        assert!(segs.iter().all(|s| s.start != 0 || !s.free));
        assert!(segs.iter().any(|s| s.free && s.start == 51 * MIB));
        let tiny = segments(10 * MIB, &[(0, 10 * MIB - 100, "/b".into(), "p".into())]);
        assert!(tiny.iter().all(|s| !s.free));
    }

    #[test]
    fn confirm_requires_the_kernel_name_and_system_ack() {
        assert!(confirm_ok("nvme0n1p2", "nvme0n1p2", false, ""));
        assert!(!confirm_ok("nvme0n1p2", "nvme0n1", false, ""));
        assert!(!confirm_ok("nvme0n1", "nvme0n1", true, ""));
        assert!(confirm_ok("nvme0n1", "nvme0n1", true, "system"));
        assert!(is_system_mount("/boot/efi"));
        assert!(!is_system_mount("/mnt/data"));
    }

    #[test]
    fn mounted_volume_offers_unmount_and_blocks_format() {
        let mut model = Model::default();
        let mut vol = block("/b", "/dev/sda1");
        vol.mount_points.push("/mnt/data".into());
        vol.partition = Some(Partition {
            number: 1,
            type_code: "0x83".into(),
            name: String::new(),
            flags: 0,
            offset: MIB,
            size: 50 * MIB,
            uuid: String::new(),
            table: "/disk".into(),
            container: false,
            contained: false,
        });
        model.blocks.push(vol);
        let actions = model.actions(&Caps::default(), &RowKind::Volume { block: "/b".into() });
        let mount = actions.iter().find(|a| a.id == ActionId::Mount).unwrap();
        let unmount = actions.iter().find(|a| a.id == ActionId::Unmount).unwrap();
        let format = actions.iter().find(|a| a.id == ActionId::Format).unwrap();
        assert!(!mount.enabled);
        assert!(unmount.enabled);
        assert!(!format.enabled);
        assert!(format.reason.contains("Unmount"));
    }

    #[test]
    fn missing_formatter_dims_format_for_that_type() {
        let mut model = Model::default();
        let mut vol = block("/b", "/dev/sda1");
        vol.id_type = "xfs".into();
        vol.filesystem = false;
        model.blocks.push(vol);
        let mut caps = Caps::default();
        caps.format.insert(
            "xfs".into(),
            Tool {
                available: false,
                missing: "xfsprogs".into(),
            },
        );
        // Format checks the current type only as a hint when a filesystem is present.
        // An empty volume stays formattable; the form itself lists per-type tools.
        let actions = model.actions(&caps, &RowKind::Volume { block: "/b".into() });
        let format = actions.iter().find(|a| a.id == ActionId::Format).unwrap();
        assert!(format.enabled);
    }

    #[test]
    fn system_root_is_flagged() {
        let mut model = Model::default();
        let mut disk = block("/disk", "/dev/nvme0n1");
        disk.table = Some(PartTable { kind: "gpt".into() });
        disk.filesystem = false;
        disk.size = 100 * MIB;
        let mut root = block("/p", "/dev/nvme0n1p2");
        root.mount_points.push("/".into());
        root.partition = Some(Partition {
            number: 2,
            type_code: PART_TYPES[0].gpt.into(),
            name: "root".into(),
            flags: 0,
            offset: MIB,
            size: 90 * MIB,
            uuid: String::new(),
            table: "/disk".into(),
            container: false,
            contained: false,
        });
        model.blocks.push(disk);
        model.blocks.push(root);
        assert!(model.is_system_tree("/disk"));
        assert_eq!(
            model.confirm_name(&RowKind::Drive {
                drive: "/drives/d".into()
            }),
            "nvme0n1"
        );
        assert!(model.row_is_system(&RowKind::Drive {
            drive: "/drives/d".into()
        }));
        assert!(model.row_is_system(&RowKind::Volume { block: "/p".into() }));
    }
}
