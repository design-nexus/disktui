use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::model::{ConfigItem, PropVal};

#[derive(Clone, Debug)]
pub enum Op {
    Mount(String),
    Unmount { path: String, force: bool },
    SetLabel { path: String, label: String, swap: bool },
    Format(FormatReq),
    CreatePartition(CreateReq),
    DeletePartition { path: String },
    Resize {
        path: String,
        current: u64,
        new_size: u64,
        filesystem: bool,
    },
    EditPartition {
        path: String,
        type_code: String,
        name: String,
        flags: u64,
    },
    MountOptions(MountOptReq),
    ClearFstab { path: String, items: Vec<ConfigItem> },
    Check(String),
    Repair(String),
    TakeOwnership(String),
    Unlock { path: String, passphrase: String },
    Lock(String),
    ChangePassphrase { path: String, old: String, new: String },
    HeaderBackup { path: String, file: String },
    RestoreHeader { path: String, file: String },
    ConvertLuks { path: String, version: String },
    #[allow(dead_code)]
    ResizeLuks { path: String, size: u64, passphrase: String },
    SwapStart(String),
    SwapStop(String),
    LoopSetup { file: String, read_only: bool },
    LoopDelete(String),
    PowerOff(String),
    Eject(String),
    Standby(String),
    Wakeup(String),
    DriveConfig { path: String, pairs: Vec<(String, PropVal)> },
    Smart { path: String, nvme: bool },
    SmartTest { path: String, kind: String, nvme: bool },
    SmartAbort { path: String, nvme: bool },
    SecureErase { path: String, enhanced: bool },
    Sanitize { path: String, action: String },
    RaidCreate {
        devices: Vec<String>,
        level: String,
        name: String,
        chunk: u64,
    },
    RaidStart { path: String, degraded: bool },
    RaidStop(String),
    RaidDelete(String),
    Benchmark {
        path: String,
        write: bool,
        size: u64,
        cancel: Arc<AtomicBool>,
    },
    Backup {
        path: String,
        file: String,
        size: u64,
        cancel: Arc<AtomicBool>,
    },
    Restore {
        path: String,
        file: String,
        size: u64,
        cancel: Arc<AtomicBool>,
    },
    Rescan(String),
    CancelJob(String),
}

#[derive(Clone, Debug)]
pub struct FormatReq {
    pub path: String,
    pub kind: String,
    pub label: String,
    pub erase: String,
    pub encrypt: bool,
    pub passphrase: String,
    pub take_ownership: bool,
}

#[derive(Clone, Debug)]
pub struct CreateReq {
    pub disk: String,
    pub offset: u64,
    pub size: u64,
    pub type_code: String,
    pub name: String,
    pub fstype: String,
    pub label: String,
    pub encrypt: bool,
    pub passphrase: String,
}

#[derive(Clone, Debug)]
pub struct MountOptReq {
    pub path: String,
    pub existing: Vec<ConfigItem>,
    pub fsname: String,
    pub dir: String,
    pub fstype: String,
    pub opts: String,
    pub passno: i32,
}
