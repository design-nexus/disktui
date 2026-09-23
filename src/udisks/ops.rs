use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::OwnedFd;
use std::os::unix::fs::FileExt;
use std::sync::atomic::Ordering;
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use tokio::sync::mpsc::UnboundedSender;
use zbus::zvariant::{Fd, OwnedObjectPath, OwnedValue, Value};
use zbus::Connection;

use super::{owned, proxy};
use crate::model::{ConfigItem, PropVal};
use crate::msg::{Msg, Progress};
use crate::op::{CreateReq, FormatReq, MountOptReq, Op};
use crate::util::{brief, human_size, with_nul};

const BLOCK: &str = "org.freedesktop.UDisks2.Block";
const FS: &str = "org.freedesktop.UDisks2.Filesystem";
const PART: &str = "org.freedesktop.UDisks2.Partition";
const TABLE: &str = "org.freedesktop.UDisks2.PartitionTable";
const ENC: &str = "org.freedesktop.UDisks2.Encrypted";
const SWAP: &str = "org.freedesktop.UDisks2.Swapspace";
const LOOP: &str = "org.freedesktop.UDisks2.Loop";
const DRIVE: &str = "org.freedesktop.UDisks2.Drive";
const ATA: &str = "org.freedesktop.UDisks2.Drive.Ata";
const NVME: &str = "org.freedesktop.UDisks2.NVMe.Controller";
const RAID: &str = "org.freedesktop.UDisks2.MDRaid";
const MGR: &str = "org.freedesktop.UDisks2.Manager";
const JOB: &str = "org.freedesktop.UDisks2.Job";

pub async fn execute(conn: &Connection, op: Op, tx: UnboundedSender<Msg>) {
    let result = dispatch(conn, op, &tx).await;
    match result {
        Ok(text) => {
            let _ = tx.send(Msg::Info(text));
        }
        Err(err) => {
            let _ = tx.send(Msg::Error(brief(&format!("{err:#}"))));
        }
    }
    let _ = tx.send(Msg::Busy(false));
    let _ = tx.send(Msg::Progress(None));
}

async fn dispatch(conn: &Connection, op: Op, tx: &UnboundedSender<Msg>) -> Result<String> {
    match op {
        Op::Mount(path) => {
            let where_ = call_string(conn, &path, FS, "Mount", &empty()).await?;
            Ok(format!("Mounted at {where_}"))
        }
        Op::Unmount { path, force } => {
            let mut options = empty();
            if force {
                options.insert("force".into(), owned(true)?);
            }
            call_unit(conn, &path, FS, "Unmount", &options).await?;
            Ok("Unmounted.".into())
        }
        Op::SetLabel { path, label, swap } => {
            let iface = if swap { SWAP } else { FS };
            let options = empty();
            // SetLabel's label is a positional argument, not an option.
            let proxy = proxy(conn, &path, iface).await?;
            proxy
                .call::<_, _, ()>("SetLabel", &(label.as_str(), options_as_value(&options)?))
                .await
                .map_err(|e| anyhow!(e))?;
            let _ = options;
            Ok(format!("Label set to {label}."))
        }
        Op::Format(req) => format_device(conn, &req).await,
        Op::CreatePartition(req) => create_partition(conn, &req).await,
        Op::DeletePartition { path } => {
            let mut options = empty();
            options.insert("tear-down".into(), owned(true)?);
            call_unit(conn, &path, PART, "Delete", &options).await?;
            Ok("Partition deleted.".into())
        }
        Op::Resize { path, current, new_size, filesystem } => {
            resize(conn, &path, current, new_size, filesystem).await
        }
        Op::EditPartition { path, type_code, name, flags } => {
            call_unit_body(conn, &path, PART, "SetType", &(type_code.as_str(), &empty_map())).await?;
            call_unit_body(conn, &path, PART, "SetName", &(name.as_str(), &empty_map())).await?;
            call_unit_body(conn, &path, PART, "SetFlags", &(flags, &empty_map())).await?;
            Ok("Partition updated.".into())
        }
        Op::MountOptions(req) => write_mount_options(conn, &req).await,
        Op::ClearFstab { path, items } => {
            remove_fstab(conn, &path, &items).await?;
            Ok("Removed the fstab entry.".into())
        }
        Op::Check(path) => {
            let ok: bool = call_ret(conn, &path, FS, "Check", &empty()).await?;
            Ok(if ok {
                "Filesystem is consistent.".into()
            } else {
                "Filesystem reported problems.".into()
            })
        }
        Op::Repair(path) => {
            let ok: bool = call_ret(conn, &path, FS, "Repair", &empty()).await?;
            Ok(if ok {
                "Filesystem repaired.".into()
            } else {
                "Repair finished, but the filesystem still reports problems.".into()
            })
        }
        Op::TakeOwnership(path) => {
            let mut options = empty();
            options.insert("recursive".into(), owned(true)?);
            call_unit(conn, &path, FS, "TakeOwnership", &options).await?;
            Ok("Ownership updated.".into())
        }
        Op::Unlock { path, passphrase } => {
            let proxy = proxy(conn, &path, ENC).await?;
            let clear: OwnedObjectPath = proxy
                .call("Unlock", &(passphrase.as_str(), &empty_map()))
                .await
                .map_err(|e| anyhow!(e))?;
            Ok(format!("Unlocked {clear}."))
        }
        Op::Lock(path) => {
            call_unit(conn, &path, ENC, "Lock", &empty()).await?;
            Ok("Locked.".into())
        }
        Op::ChangePassphrase { path, old, new } => {
            call_unit_body(conn, &path, ENC, "ChangePassphrase", &(old.as_str(), new.as_str(), &empty_map())).await?;
            Ok("Passphrase changed.".into())
        }
        Op::HeaderBackup { path, file } => {
            call_unit_body(conn, &path, ENC, "HeaderBackup", &(file.as_str(), &empty_map())).await?;
            Ok(format!("Header saved to {file}."))
        }
        Op::RestoreHeader { path, file } => {
            call_unit_body(conn, &path, BLOCK, "RestoreEncryptedHeader", &(file.as_str(), &empty_map())).await?;
            Ok("LUKS header restored.".into())
        }
        Op::ConvertLuks { path, version } => {
            call_unit_body(conn, &path, ENC, "Convert", &(version.as_str(), &empty_map())).await?;
            Ok(format!("Converted to {version}."))
        }
        Op::ResizeLuks { path, size, passphrase } => {
            let mut options = HashMap::new();
            options.insert("passphrase", Value::from(passphrase));
            call_unit_body(conn, &path, ENC, "Resize", &(size, options)).await?;
            Ok("LUKS container resized.".into())
        }
        Op::SwapStart(path) => {
            call_unit(conn, &path, SWAP, "Start", &empty()).await?;
            Ok("Swap enabled.".into())
        }
        Op::SwapStop(path) => {
            call_unit(conn, &path, SWAP, "Stop", &empty()).await?;
            Ok("Swap disabled.".into())
        }
        Op::LoopSetup { file, read_only } => loop_setup(conn, &file, read_only, tx).await,
        Op::LoopDelete(path) => {
            call_unit(conn, &path, LOOP, "Delete", &empty()).await?;
            Ok("Loop device detached.".into())
        }
        Op::PowerOff(path) => {
            call_unit(conn, &path, DRIVE, "PowerOff", &empty()).await?;
            Ok("Drive powered off.".into())
        }
        Op::Eject(path) => {
            call_unit(conn, &path, DRIVE, "Eject", &empty()).await?;
            Ok("Ejected.".into())
        }
        Op::Standby(path) => {
            call_unit(conn, &path, ATA, "PmStandby", &empty()).await?;
            Ok("Drive entering standby.".into())
        }
        Op::Wakeup(path) => {
            call_unit(conn, &path, ATA, "PmWakeup", &empty()).await?;
            Ok("Drive waking up.".into())
        }
        Op::DriveConfig { path, pairs } => {
            let dict = props_to_map(&pairs)?;
            call_unit_body(conn, &path, DRIVE, "SetConfiguration", &(dict, empty_map())).await?;
            Ok("Drive settings saved.".into())
        }
        Op::Smart { path, nvme } => smart_text(conn, &path, nvme, tx).await,
        Op::SmartTest { path, kind, nvme } => {
            let iface = if nvme { NVME } else { ATA };
            call_unit_body(conn, &path, iface, "SmartSelftestStart", &(kind.as_str(), &empty_map())).await?;
            Ok(format!("{kind} self-test started."))
        }
        Op::SmartAbort { path, nvme } => {
            let iface = if nvme { NVME } else { ATA };
            call_unit(conn, &path, iface, "SmartSelftestAbort", &empty()).await?;
            Ok("Self-test aborted.".into())
        }
        Op::SecureErase { path, enhanced } => {
            let mut options = empty();
            options.insert("enhanced".into(), owned(enhanced)?);
            call_unit(conn, &path, ATA, "SecurityEraseUnit", &options).await?;
            Ok("Secure erase finished.".into())
        }
        Op::Sanitize { path, action } => {
            call_unit_body(conn, &path, NVME, "SanitizeStart", &(action.as_str(), &empty_map())).await?;
            Ok(format!("NVMe sanitize ({action}) started. It cannot be cancelled."))
        }
        Op::RaidCreate { devices, level, name, chunk } => {
            let paths: Vec<OwnedObjectPath> = devices
                .iter()
                .map(|p| OwnedObjectPath::try_from(p.as_str()))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| anyhow!(e))?;
            let proxy = proxy(conn, "/org/freedesktop/UDisks2", MGR).await?;
            let created: OwnedObjectPath = proxy
                .call("MDRaidCreate", &(paths, level.as_str(), name.as_str(), chunk, &empty_map()))
                .await
                .map_err(|e| anyhow!(e))?;
            Ok(format!("RAID array created at {created}."))
        }
        Op::RaidStart { path, degraded } => {
            let mut options = HashMap::new();
            if degraded {
                options.insert("start-degraded", Value::from(true));
            }
            call_unit_body(conn, &path, RAID, "Start", &options).await?;
            Ok("RAID array started.".into())
        }
        Op::RaidStop(path) => {
            call_unit(conn, &path, RAID, "Stop", &empty()).await?;
            Ok("RAID array stopped.".into())
        }
        Op::RaidDelete(path) => {
            let mut options = empty();
            options.insert("tear-down".into(), owned(true)?);
            call_unit(conn, &path, RAID, "Delete", &options).await?;
            Ok("RAID array deleted.".into())
        }
        Op::Benchmark { path, write, size, cancel } => {
            benchmark(conn, &path, write, size, &cancel, tx).await
        }
        Op::Backup { path, file, size, cancel } => {
            image_copy(conn, &path, &file, size, false, &cancel, tx).await
        }
        Op::Restore { path, file, size, cancel } => {
            image_copy(conn, &path, &file, size, true, &cancel, tx).await
        }
        Op::Rescan(path) => {
            call_unit(conn, &path, BLOCK, "Rescan", &empty()).await?;
            Ok("Rescanned.".into())
        }
        Op::CancelJob(path) => {
            call_unit(conn, &path, JOB, "Cancel", &empty()).await?;
            Ok("Cancel requested.".into())
        }
    }
}

async fn format_device(conn: &Connection, req: &FormatReq) -> Result<String> {
    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("tear-down", Value::from(true));
    options.insert("update-partition-type", Value::from(true));
    if !req.label.is_empty() && req.kind != "empty" {
        options.insert("label", Value::from(req.label.as_str()));
    }
    if !req.erase.is_empty() && req.erase != "none" {
        options.insert("erase", Value::from(req.erase.as_str()));
    }
    if req.take_ownership {
        options.insert("take-ownership", Value::from(true));
    }
    if req.encrypt {
        options.insert("encrypt.passphrase", Value::from(req.passphrase.as_str()));
        options.insert("encrypt.type", Value::from("luks2"));
    }
    call_unit_body(conn, &req.path, BLOCK, "Format", &(req.kind.as_str(), options)).await?;
    Ok(format!("Formatted as {}.", req.kind))
}

async fn create_partition(conn: &Connection, req: &CreateReq) -> Result<String> {
    let mut options: HashMap<&str, Value> = HashMap::new();
    if req.type_code.starts_with("0x") {
        options.insert("partition-type", Value::from("primary"));
    }
    if req.fstype.is_empty() || req.fstype == "empty" {
        let proxy = proxy(conn, &req.disk, TABLE).await?;
        let created: OwnedObjectPath = proxy
            .call(
                "CreatePartition",
                &(req.offset, req.size, req.type_code.as_str(), req.name.as_str(), options),
            )
            .await
            .map_err(|e| anyhow!(e))?;
        return Ok(format!("Partition created at {created}."));
    }
    let mut format_options: HashMap<&str, Value> = HashMap::new();
    format_options.insert("update-partition-type", Value::from(true));
    format_options.insert("take-ownership", Value::from(true));
    if !req.label.is_empty() {
        format_options.insert("label", Value::from(req.label.as_str()));
    }
    if req.encrypt {
        format_options.insert("encrypt.passphrase", Value::from(req.passphrase.as_str()));
        format_options.insert("encrypt.type", Value::from("luks2"));
    }
    let proxy = proxy(conn, &req.disk, TABLE).await?;
    let created: OwnedObjectPath = proxy
        .call(
            "CreatePartitionAndFormat",
            &(
                req.offset,
                req.size,
                req.type_code.as_str(),
                req.name.as_str(),
                options,
                req.fstype.as_str(),
                format_options,
            ),
        )
        .await
        .map_err(|e| anyhow!(e))?;
    Ok(format!("Partition created and formatted at {created}."))
}

async fn resize(conn: &Connection, path: &str, current: u64, new_size: u64, filesystem: bool) -> Result<String> {
    let grow = new_size == 0 || new_size > current;
    if filesystem && !grow {
        call_unit_body(conn, path, FS, "Resize", &(new_size, &empty_map())).await?;
        call_unit_body(conn, path, PART, "Resize", &(new_size, &empty_map())).await?;
    } else {
        call_unit_body(conn, path, PART, "Resize", &(new_size, &empty_map())).await?;
        if filesystem {
            call_unit_body(conn, path, FS, "Resize", &(0u64, &empty_map())).await?;
        }
    }
    let label = if new_size == 0 { "maximum".into() } else { human_size(new_size) };
    Ok(format!("Resized to {label}."))
}

async fn write_mount_options(conn: &Connection, req: &MountOptReq) -> Result<String> {
    let mut details: HashMap<&str, Value> = HashMap::new();
    details.insert("fsname", Value::from(with_nul(&req.fsname)));
    details.insert("dir", Value::from(with_nul(&req.dir)));
    details.insert("type", Value::from(with_nul(&req.fstype)));
    details.insert("opts", Value::from(with_nul(&req.opts)));
    details.insert("freq", Value::from(0i32));
    details.insert("passno", Value::from(req.passno));
    details.insert("track-parents", Value::from(true));
    let item = ("fstab", details);
    let proxy = proxy(conn, &req.path, BLOCK).await?;
    if let Some(old) = req.existing.iter().find(|item| item.kind == "fstab") {
        let old_item = config_value(old)?;
        proxy
            .call::<_, _, ()>("UpdateConfigurationItem", &(old_item, item, &empty_map()))
            .await
            .map_err(|e| anyhow!(e))?;
    } else {
        proxy
            .call::<_, _, ()>("AddConfigurationItem", &(item, &empty_map()))
            .await
            .map_err(|e| anyhow!(e))?;
    }
    for extra in req.existing.iter().filter(|item| item.kind == "fstab").skip(1) {
        let _ = remove_one(conn, &req.path, extra).await;
    }
    Ok(format!("fstab now mounts this volume at {}.", req.dir))
}

async fn remove_fstab(conn: &Connection, path: &str, items: &[ConfigItem]) -> Result<()> {
    let mut removed = false;
    for item in items.iter().filter(|item| item.kind == "fstab") {
        remove_one(conn, path, item).await?;
        removed = true;
    }
    if !removed {
        return Err(anyhow!("There is no fstab entry for this device."));
    }
    Ok(())
}

async fn remove_one(conn: &Connection, path: &str, item: &ConfigItem) -> Result<()> {
    let value = config_value(item)?;
    call_unit_body(conn, path, BLOCK, "RemoveConfigurationItem", &(value, &empty_map())).await
}

fn config_value(item: &ConfigItem) -> Result<(&str, HashMap<String, OwnedValue>)> {
    let mut details = HashMap::new();
    for (key, value) in &item.fields {
        details.insert(key.clone(), prop_to_owned(value)?);
    }
    Ok((item.kind.as_str(), details))
}

fn prop_to_owned(value: &PropVal) -> Result<OwnedValue> {
    match value {
        PropVal::Bool(v) => owned(*v),
        PropVal::I32(v) => owned(*v),
        PropVal::U32(v) => owned(*v),
        PropVal::U64(v) => owned(*v),
        PropVal::I64(v) => owned(*v),
        PropVal::Str(v) => owned(v.clone()),
        PropVal::Bytes(v) => owned(v.clone()),
        PropVal::Path(v) => {
            let path = zbus::zvariant::ObjectPath::try_from(v.as_str()).map_err(|e| anyhow!(e))?;
            owned(path.to_owned())
        }
    }
}

fn props_to_map(pairs: &[(String, PropVal)]) -> Result<HashMap<String, OwnedValue>> {
    let mut map = HashMap::new();
    for (key, value) in pairs {
        map.insert(key.clone(), prop_to_owned(value)?);
    }
    Ok(map)
}

async fn loop_setup(conn: &Connection, file: &str, read_only: bool, tx: &UnboundedSender<Msg>) -> Result<String> {
    let handle = std::fs::File::open(file).with_context(|| format!("open {file}"))?;
    let fd = Fd::from(&handle);
    let mut options = HashMap::new();
    options.insert("read-only", Value::from(read_only));
    let proxy = proxy(conn, "/org/freedesktop/UDisks2", MGR).await?;
    let created: OwnedObjectPath = proxy
        .call("LoopSetup", &(fd, options))
        .await
        .map_err(|e| anyhow!(e))?;
    drop(handle);
    let _ = tx.send(Msg::Attached(created.to_string()));
    Ok(format!("Attached {file}."))
}

async fn smart_text(conn: &Connection, path: &str, nvme: bool, tx: &UnboundedSender<Msg>) -> Result<String> {
    let iface = if nvme { NVME } else { ATA };
    let proxy = proxy(conn, path, iface).await?;
    proxy
        .call::<_, _, ()>("SmartUpdate", &empty_map())
        .await
        .map_err(|e| anyhow!(e))?;
    let reply = proxy
        .call_method("SmartGetAttributes", &empty_map())
        .await
        .map_err(|e| anyhow!(e))?;
    let value: OwnedValue = reply.body().deserialize()?;
    let text = if nvme { format_nvme(&value) } else { format_ata(&value) };
    let _ = tx.send(Msg::Smart(text));
    Ok("SMART data updated.".into())
}

fn format_ata(value: &Value) -> String {
    let mut lines = vec!["id  attribute                         value  worst  threshold  raw".to_string()];
    let Value::Array(array) = value else {
        return format!("{value:?}");
    };
    for item in array.inner().iter() {
        let Value::Structure(fields) = item else { continue };
        let f: Vec<&Value> = fields.fields().iter().collect();
        let id = match f.first().map(|v| &**v) {
            Some(Value::U8(n)) => *n,
            _ => 0,
        };
        let name = f.get(1).and_then(|v| match &**v {
            Value::Str(s) => Some(s.as_str()),
            _ => None,
        }).unwrap_or("");
        let value_n = int_at(&f, 3);
        let worst = int_at(&f, 4);
        let threshold = int_at(&f, 5);
        let pretty = int_at(&f, 6);
        lines.push(format!("{id:<3} {name:<32} {value_n:>6} {worst:>6} {threshold:>10} {pretty:>8}"));
    }
    lines.join("\n")
}

fn int_at(fields: &[&Value], index: usize) -> i64 {
    match fields.get(index).map(|v| &**v) {
        Some(Value::I32(n)) => *n as i64,
        Some(Value::I64(n)) => *n,
        Some(Value::U64(n)) => *n as i64,
        Some(Value::U8(n)) => *n as i64,
        _ => -1,
    }
}

fn format_nvme(value: &Value) -> String {
    let Value::Dict(dict) = value else {
        return format!("{value:?}");
    };
    let mut lines = Vec::new();
    for (key, raw) in dict.iter() {
        let key = match key {
            Value::Str(s) => s.as_str().to_string(),
            other => format!("{other:?}"),
        };
        let shown = match raw {
            Value::Value(inner) => scalar(inner),
            other => scalar(other),
        };
        lines.push(format!("{key}: {shown}"));
    }
    lines.sort();
    if lines.is_empty() {
        "No NVMe health attributes.".into()
    } else {
        lines.join("\n")
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::U8(n) => n.to_string(),
        Value::U16(n) => n.to_string(),
        Value::U32(n) => n.to_string(),
        Value::U64(n) => human_size(*n).contains("B").then(|| human_size(*n)).unwrap_or_else(|| n.to_string()),
        Value::I32(n) => n.to_string(),
        Value::I64(n) => n.to_string(),
        Value::Bool(n) => n.to_string(),
        Value::Str(s) => s.to_string(),
        Value::Array(array) => {
            let nums: Vec<String> = array.inner().iter().map(|v| scalar(v)).collect();
            nums.join(", ")
        }
        other => format!("{other:?}"),
    }
}

async fn benchmark(
    conn: &Connection,
    path: &str,
    write: bool,
    size: u64,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    tx: &UnboundedSender<Msg>,
) -> Result<String> {
    let mut options = HashMap::new();
    if write {
        options.insert("writable", Value::from(true));
    }
    let proxy = proxy(conn, path, BLOCK).await?;
    let fd: zbus::zvariant::OwnedFd = proxy
        .call("OpenForBenchmark", &options)
        .await
        .map_err(|e| anyhow!(e))?;
    let file = std::fs::File::from(OwnedFd::from(fd));
    let label = if write { "Writing" } else { "Reading" };
    let copied = transfer_device(&file, None, size, write, label, cancel, tx)?;
    Ok(format!("Benchmark finished, {copied}."))
}

async fn image_copy(
    conn: &Connection,
    path: &str,
    image: &str,
    size: u64,
    restore: bool,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    tx: &UnboundedSender<Msg>,
) -> Result<String> {
    let mode = if restore { "w" } else { "r" };
    let mut options = HashMap::new();
    options.insert("flags", Value::from(0x80000i32)); // O_CLOEXEC
    let proxy = proxy(conn, path, BLOCK).await?;
    let fd: zbus::zvariant::OwnedFd = proxy
        .call("OpenDevice", &(mode, options))
        .await
        .map_err(|e| anyhow!(e))?;
    let device = std::fs::File::from(OwnedFd::from(fd));
    if restore {
        let mut image_file = std::fs::File::open(image).with_context(|| format!("open {image}"))?;
        let image_len = image_file.metadata()?.len();
        let total = size.min(image_len);
        let label = "Restoring";
        // Buffered copy. OpenDevice is not O_DIRECT, so alignment is not required.
        copy_streams(&mut image_file, &device, total, true, label, cancel, tx)?;
        Ok(format!("Restored {image}."))
    } else {
        let mut image_file = std::fs::File::create(image).with_context(|| format!("create {image}"))?;
        copy_streams(&device, &mut image_file, size, false, "Reading", cancel, tx)?;
        Ok(format!("Image written to {image}."))
    }
}

fn transfer_device(
    device: &std::fs::File,
    mut image: Option<&mut std::fs::File>,
    size: u64,
    write: bool,
    label: &str,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    tx: &UnboundedSender<Msg>,
) -> Result<String> {
    const CHUNK: usize = 8 * 1024 * 1024;
    const ALIGN: usize = 4096;
    let mut raw = vec![0u8; CHUNK + ALIGN];
    let addr = raw.as_mut_ptr() as usize;
    let off = (ALIGN - (addr % ALIGN)) % ALIGN;
    let buf = &mut raw[off..off + CHUNK];
    let mut offset = 0u64;
    let started = Instant::now();
    let mut last_bucket = usize::MAX;
    while offset + CHUNK as u64 <= size.max(CHUNK as u64) && offset < size {
        if cancel.load(Ordering::Relaxed) {
            return Err(anyhow!("cancelled"));
        }
        let n = if write {
            device.write_at(buf, offset).context("write")? 
        } else {
            device.read_at(buf, offset).context("read")?
        };
        if n == 0 {
            break;
        }
        if let Some(image) = image.as_mut() {
            image.write_all(&buf[..n])?;
        }
        offset += n as u64;
        let ratio = if size == 0 { 0.0 } else { offset as f64 / size as f64 };
        let secs = started.elapsed().as_secs_f64().max(0.001);
        let mbps = (offset as f64 / secs) / 1_000_000.0;
        let bucket = (ratio * 160.0) as usize;
        if bucket != last_bucket {
            last_bucket = bucket;
            let _ = tx.send(Msg::Sample { pos: ratio, mbps });
        }
        let _ = tx.send(Msg::Progress(Some(Progress {
            label: format!("{label}  {mbps:.0} MB/s"),
            ratio,
        })));
        if n < CHUNK {
            break;
        }
    }
    Ok(human_size(offset))
}

fn copy_streams(
    reader: &std::fs::File,
    writer: &std::fs::File,
    total: u64,
    restore: bool,
    label: &str,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    tx: &UnboundedSender<Msg>,
) -> Result<()> {
    let mut reader = reader;
    let mut writer = writer;
    let mut buf = vec![0u8; 1024 * 1024];
    let mut offset = 0u64;
    let started = Instant::now();
    reader.seek(SeekFrom::Start(0))?;
    while offset < total {
        if cancel.load(Ordering::Relaxed) {
            return Err(anyhow!("cancelled"));
        }
        let want = ((total - offset) as usize).min(buf.len());
        let n = reader.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n])?;
        offset += n as u64;
        let ratio = if total == 0 { 1.0 } else { offset as f64 / total as f64 };
        let secs = started.elapsed().as_secs_f64().max(0.001);
        let mbps = (offset as f64 / secs) / 1_000_000.0;
        let _ = tx.send(Msg::Sample { pos: ratio, mbps });
        let _ = tx.send(Msg::Progress(Some(Progress {
            label: format!("{label}  {mbps:.0} MB/s"),
            ratio,
        })));
    }
    if restore {
        writer.sync_all()?;
    } else {
        writer.sync_all()?;
    }
    Ok(())
}

fn empty() -> HashMap<String, OwnedValue> {
    HashMap::new()
}

fn empty_map() -> HashMap<&'static str, Value<'static>> {
    HashMap::new()
}

fn options_as_value(options: &HashMap<String, OwnedValue>) -> Result<HashMap<String, Value<'static>>> {
    let mut out = HashMap::new();
    for (key, value) in options {
        let cloned = zbus::zvariant::Value::try_clone(&**value).map_err(|e| anyhow!(e))?;
        out.insert(key.clone(), cloned);
    }
    Ok(out)
}

async fn call_unit(
    conn: &Connection,
    path: &str,
    iface: &'static str,
    method: &str,
    options: &HashMap<String, OwnedValue>,
) -> Result<()> {
    let proxy = proxy(conn, path, iface).await?;
    proxy
        .call::<_, _, ()>(method, options)
        .await
        .map_err(|e| anyhow!(e))
}

async fn call_unit_body(
    conn: &Connection,
    path: &str,
    iface: &'static str,
    method: &str,
    body: &(impl serde::Serialize + zbus::zvariant::DynamicType + Sync),
) -> Result<()> {
    let proxy = proxy(conn, path, iface).await?;
    proxy
        .call::<_, _, ()>(method, body)
        .await
        .map_err(|e| anyhow!(e))
}

async fn call_ret<R>(
    conn: &Connection,
    path: &str,
    iface: &'static str,
    method: &str,
    options: &HashMap<String, OwnedValue>,
) -> Result<R>
where
    R: serde::de::DeserializeOwned + zbus::zvariant::Type,
{
    let proxy = proxy(conn, path, iface).await?;
    proxy.call(method, options).await.map_err(|e| anyhow!(e))
}

async fn call_string(
    conn: &Connection,
    path: &str,
    iface: &'static str,
    method: &str,
    options: &HashMap<String, OwnedValue>,
) -> Result<String> {
    call_ret(conn, path, iface, method, options).await
}


