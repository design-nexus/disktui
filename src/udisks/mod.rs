mod ops;

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use tokio::sync::mpsc::UnboundedSender;
use zbus::names::OwnedInterfaceName;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, Proxy};

use crate::model::{
    Ata, Block, ConfigItem, Drive, Job, Model, PartTable, Partition, PropVal, Raid,
};
use crate::msg::Msg;
use crate::util::{brief, c_string};

pub use ops::execute;

const DEST: &str = "org.freedesktop.UDisks2";

type RawObjects = HashMap<OwnedObjectPath, HashMap<OwnedInterfaceName, HashMap<String, OwnedValue>>>;
type Props = HashMap<String, OwnedValue>;

pub async fn connect() -> Result<Connection> {
    Connection::system().await.context("system bus")
}

pub async fn watch(conn: Connection, tx: UnboundedSender<Msg>) {
    // Poll the object tree. A message stream on this connection fills and then
    // the socket reader stops, which makes every call including SMART wait.
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        match fetch(&conn).await {
            Ok(model) => { let _ = tx.send(Msg::Model(model)); }
            Err(err) => { let _ = tx.send(Msg::Error(brief(&format!("{err:#}")))); }
        }
    }
}

pub async fn load_caps(conn: &Connection) -> crate::model::Caps {
    let mut caps = crate::model::Caps::default();
    let types = ["ext4", "btrfs", "xfs", "f2fs", "vfat", "exfat", "ntfs", "udf", "swap", "empty"];
    for kind in types {
        if let Ok((available, missing)) = can_bs(conn, "CanFormat", kind).await {
            caps.format.insert(kind.into(), crate::model::Tool { available, missing });
        }
        if let Ok((available, missing)) = can_bs(conn, "CanCheck", kind).await {
            caps.check.insert(kind.into(), crate::model::Tool { available, missing });
        }
        if let Ok((available, missing)) = can_bs(conn, "CanRepair", kind).await {
            caps.repair.insert(kind.into(), crate::model::Tool { available, missing });
        }
        if let Ok((available, mode, missing)) = can_resize(conn, kind).await {
            caps.resize.insert(
                kind.into(),
                crate::model::ResizeCap {
                    available,
                    mode: mode as u32,
                    missing,
                },
            );
        }
    }
    caps
}

async fn can_bs(conn: &Connection, method: &str, kind: &str) -> Result<(bool, String)> {
    let proxy = manager(conn).await?;
    proxy.call(method, &kind).await.map_err(|e| anyhow!(e))
}

async fn can_resize(conn: &Connection, kind: &str) -> Result<(bool, u64, String)> {
    let proxy = manager(conn).await?;
    proxy.call("CanResize", &kind).await.map_err(|e| anyhow!(e))
}

async fn manager(conn: &Connection) -> Result<Proxy<'_>> {
    proxy(conn, "/org/freedesktop/UDisks2", "org.freedesktop.UDisks2.Manager").await
}

pub(crate) async fn proxy<'a>(
    conn: &'a Connection,
    path: &'a str,
    iface: &'static str,
) -> Result<Proxy<'a>> {
    Proxy::new(conn, DEST, path, iface)
        .await
        .with_context(|| format!("proxy {iface} {path}"))
}

async fn fetch(conn: &Connection) -> Result<Model> {
    let proxy = proxy(
        conn,
        "/org/freedesktop/UDisks2",
        "org.freedesktop.DBus.ObjectManager",
    )
    .await?;
    let reply = proxy
        .call_method("GetManagedObjects", &())
        .await
        .context("GetManagedObjects")?;
    let objects: RawObjects = reply.body().deserialize().context("managed objects")?;
    Ok(parse_model(&objects))
}

fn parse_model(objects: &RawObjects) -> Model {
    let mut model = Model::default();
    for (path, ifaces) in objects {
        let path = path.as_str();
        if let Some(props) = iface(ifaces, "org.freedesktop.UDisks2.Drive") {
            model.drives.push(parse_drive(
                path,
                props,
                iface(ifaces, "org.freedesktop.UDisks2.Drive.Ata"),
                iface(ifaces, "org.freedesktop.UDisks2.NVMe.Controller"),
            ));
        }
        if let Some(props) = iface(ifaces, "org.freedesktop.UDisks2.Block") {
            model.blocks.push(parse_block(path, props, ifaces));
        }
        if let Some(props) = iface(ifaces, "org.freedesktop.UDisks2.MDRaid") {
            model.raids.push(parse_raid(path, props));
        }
        if let Some(props) = iface(ifaces, "org.freedesktop.UDisks2.Job") {
            model.jobs.push(parse_job(path, props));
        }
    }
    model
}

fn parse_drive(path: &str, props: &Props, ata: Option<&Props>, nvme: Option<&Props>) -> Drive {
    Drive {
        path: path.to_string(),
        vendor: prop_str(props, "Vendor"),
        model: prop_str(props, "Model"),
        revision: prop_str(props, "Revision"),
        serial: prop_str(props, "Serial"),
        size: prop_u64(props, "Size"),
        connection: prop_str(props, "ConnectionBus"),
        media: prop_str(props, "Media"),
        removable: prop_bool(props, "Removable"),
        ejectable: prop_bool(props, "Ejectable"),
        optical: prop_bool(props, "Optical"),
        can_power_off: prop_bool(props, "CanPowerOff"),
        sort_key: prop_str(props, "SortKey"),
        rotation: prop_i32(props, "RotationRate"),
        config: prop_dict(props, "Configuration"),
        ata: ata.map(parse_ata),
        nvme: nvme.is_some(),
        nvme_path: path.to_string(),
    }
}

fn parse_ata(props: &Props) -> Ata {
    Ata {
        smart_supported: prop_bool(props, "SmartSupported"),
        smart_enabled: prop_bool(props, "SmartEnabled"),
        smart_failing: prop_bool(props, "SmartFailing"),
        pm_supported: prop_bool(props, "PmSupported"),
        apm_supported: prop_bool(props, "ApmSupported"),
        write_cache_supported: prop_bool(props, "WriteCacheSupported"),
        write_cache_enabled: prop_bool(props, "WriteCacheEnabled"),
        lookahead_supported: prop_bool(props, "ReadLookaheadSupported"),
        lookahead_enabled: prop_bool(props, "ReadLookaheadEnabled"),
        secure_erase_minutes: prop_i32(props, "SecurityEraseUnitMinutes"),
        secure_erase_enhanced_minutes: prop_i32(props, "SecurityEnhancedEraseUnitMinutes"),
        security_frozen: prop_bool(props, "SecurityFrozen"),
    }
}

fn iface<'a>(
    ifaces: &'a HashMap<OwnedInterfaceName, Props>,
    name: &str,
) -> Option<&'a Props> {
    ifaces.iter().find(|(key, _)| key.as_str() == name).map(|(_, props)| props)
}

fn parse_block(path: &str, props: &Props, ifaces: &HashMap<OwnedInterfaceName, Props>) -> Block {
    let partition = iface(ifaces, "org.freedesktop.UDisks2.Partition").map(parse_partition);
    let table = iface(ifaces, "org.freedesktop.UDisks2.PartitionTable").map(|props| PartTable {
        kind: prop_str(props, "Type"),
    });
    let fs = iface(ifaces, "org.freedesktop.UDisks2.Filesystem");
    let swap = iface(ifaces, "org.freedesktop.UDisks2.Swapspace");
    let enc = iface(ifaces, "org.freedesktop.UDisks2.Encrypted");
    let loop_dev = iface(ifaces, "org.freedesktop.UDisks2.Loop");
    let preferred = prop_bytes(props, "PreferredDevice");
    let device = prop_bytes(props, "Device");
    Block {
        path: path.to_string(),
        device: device.clone(),
        preferred: if preferred.is_empty() { device } else { preferred },
        id_usage: prop_str(props, "IdUsage"),
        id_type: prop_str(props, "IdType"),
        id_label: prop_str(props, "IdLabel"),
        id_uuid: prop_str(props, "IdUUID"),
        size: prop_u64(props, "Size"),
        read_only: prop_bool(props, "ReadOnly"),
        hint_system: prop_bool(props, "HintSystem"),
        hint_ignore: prop_bool(props, "HintIgnore"),
        drive: prop_path(props, "Drive"),
        crypto_backing: prop_path(props, "CryptoBackingDevice"),
        mdraid: prop_path(props, "MDRaid"),
        mdraid_member: prop_path(props, "MDRaidMember"),
        symlinks: prop_byte_list(props, "Symlinks"),
        partition,
        table,
        filesystem: fs.is_some(),
        mount_points: fs.map(|props| prop_byte_list(props, "MountPoints")).unwrap_or_default(),
        swap: swap.is_some(),
        swap_active: swap.is_some_and(|props| prop_bool(props, "Active")),
        encrypted: enc.is_some(),
        cleartext: enc.map(|props| prop_path(props, "CleartextDevice")).unwrap_or_else(|| "/".into()),
        hint_encryption: enc.map(|props| prop_str(props, "HintEncryptionType")).unwrap_or_default(),
        loopback: loop_dev.is_some(),
        backing_file: loop_dev.map(|props| prop_bytes(props, "BackingFile")).unwrap_or_default(),
        loop_autoclear: loop_dev.is_some_and(|props| prop_bool(props, "Autoclear")),
        config: props.get("Configuration").map(|v| parse_config(v)).unwrap_or_default(),
    }
}

fn parse_partition(props: &Props) -> Partition {
    Partition {
        number: prop_u64(props, "Number") as u32,
        type_code: prop_str(props, "Type"),
        name: prop_str(props, "Name"),
        flags: prop_u64(props, "Flags"),
        offset: prop_u64(props, "Offset"),
        size: prop_u64(props, "Size"),
        uuid: prop_str(props, "UUID"),
        table: prop_path(props, "Table"),
        container: prop_bool(props, "IsContainer"),
        contained: prop_bool(props, "IsContained"),
    }
}

fn parse_raid(path: &str, props: &Props) -> Raid {
    Raid {
        path: path.to_string(),
        uuid: prop_str(props, "UUID"),
        name: prop_str(props, "Name"),
        level: prop_str(props, "Level"),
        size: prop_u64(props, "Size"),
        running: prop_bool(props, "Running"),
        degraded: prop_u64(props, "Degraded") > 0,
        sync_action: prop_str(props, "SyncAction"),
        sync_completed: prop_f64(props, "SyncCompleted"),
        members: props.get("ActiveDevices").map(active_devices).unwrap_or_default(),
    }
}

fn parse_job(path: &str, props: &Props) -> Job {
    Job {
        path: path.to_string(),
        operation: prop_str(props, "Operation"),
        progress: prop_f64(props, "Progress"),
        progress_valid: prop_bool(props, "ProgressValid"),
        cancelable: prop_bool(props, "Cancelable"),
        objects: prop_paths(props, "Objects"),
    }
}

fn active_devices(value: &OwnedValue) -> Vec<String> {
    let Value::Array(array) = unvar(value) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in array.inner() {
        let Value::Structure(fields) = unvar(item) else { continue };
        if let Some(path) = fields.fields().first().and_then(|v| as_str(unvar(v))) {
            if path != "/" {
                out.push(path.to_string());
            }
        }
    }
    out
}

fn parse_config(value: &OwnedValue) -> Vec<ConfigItem> {
    let Value::Array(array) = unvar(value) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in array.inner() {
        let Value::Structure(fields) = unvar(item) else { continue };
        let entries = fields.fields();
        let Some(kind) = entries.first().and_then(|v| as_str(unvar(v))) else { continue };
        let parsed = entries.get(1).map(|v| dict_props(unvar(v))).unwrap_or_default();
        out.push(ConfigItem { kind: kind.to_string(), fields: parsed });
    }
    out
}

fn dict_props(value: &Value) -> Vec<(String, PropVal)> {
    let Value::Dict(dict) = unvar(value) else {
        return Vec::new();
    };
    dict.iter().filter_map(|(key, raw)| {
        as_str(key).map(|key| (key.to_string(), as_prop(raw)))
    }).collect()
}

fn as_prop(value: &Value) -> PropVal {
    match unvar(value) {
        Value::Bool(v) => PropVal::Bool(*v),
        Value::I32(v) => PropVal::I32(*v),
        Value::U32(v) => PropVal::U32(*v),
        Value::U64(v) => PropVal::U64(*v),
        Value::I64(v) => PropVal::I64(*v),
        Value::Str(v) => PropVal::Str(v.to_string()),
        Value::ObjectPath(v) => PropVal::Path(v.to_string()),
        Value::Array(array) => {
            let bytes: Vec<u8> = array.inner().iter().filter_map(|item| match unvar(item) {
                Value::U8(b) => Some(*b),
                _ => None,
            }).collect();
            PropVal::Bytes(bytes)
        }
        _ => PropVal::Str(String::new()),
    }
}

fn unvar<'a>(value: &'a Value<'a>) -> &'a Value<'a> {
    match value {
        Value::Value(inner) => inner,
        other => other,
    }
}

fn as_str<'a>(value: &'a Value<'a>) -> Option<&'a str> {
    match unvar(value) {
        Value::Str(text) => Some(text.as_str()),
        Value::ObjectPath(path) => Some(path.as_str()),
        _ => None,
    }
}

fn prop<'a>(props: &'a Props, name: &str) -> Option<&'a Value<'static>> {
    props.get(name).map(|value| &**value)
}

fn prop_str(props: &Props, name: &str) -> String {
    prop(props, name).and_then(as_str).unwrap_or("").to_string()
}

fn prop_bool(props: &Props, name: &str) -> bool {
    matches!(prop(props, name).map(unvar), Some(Value::Bool(true)))
}

fn prop_u64(props: &Props, name: &str) -> u64 {
    match prop(props, name).map(unvar) {
        Some(Value::U64(v)) => *v,
        Some(Value::U32(v)) => *v as u64,
        Some(Value::I64(v)) if *v >= 0 => *v as u64,
        Some(Value::U16(v)) => *v as u64,
        _ => 0,
    }
}

fn prop_i32(props: &Props, name: &str) -> i32 {
    match prop(props, name).map(unvar) {
        Some(Value::I32(v)) => *v,
        Some(Value::U32(v)) => *v as i32,
        Some(Value::I64(v)) => *v as i32,
        _ => 0,
    }
}

fn prop_f64(props: &Props, name: &str) -> f64 {
    match prop(props, name).map(unvar) {
        Some(Value::F64(v)) => *v,
        _ => 0.0,
    }
}

fn prop_path(props: &Props, name: &str) -> String {
    prop(props, name).and_then(as_str).unwrap_or("/").to_string()
}

fn prop_bytes(props: &Props, name: &str) -> String {
    match prop(props, name).map(unvar) {
        Some(Value::Array(array)) => {
            let bytes: Vec<u8> = array.inner().iter().filter_map(|item| match unvar(item) {
                Value::U8(b) => Some(*b),
                _ => None,
            }).collect();
            c_string(&bytes)
        }
        Some(Value::Str(text)) => text.to_string(),
        _ => String::new(),
    }
}

fn prop_byte_list(props: &Props, name: &str) -> Vec<String> {
    let Some(Value::Array(array)) = prop(props, name).map(unvar) else {
        return Vec::new();
    };
    array.inner().iter().filter_map(|item| match unvar(item) {
        Value::Array(inner) => {
            let bytes: Vec<u8> = inner.inner().iter().filter_map(|b| match unvar(b) {
                Value::U8(v) => Some(*v),
                _ => None,
            }).collect();
            let text = c_string(&bytes);
            (!text.is_empty()).then_some(text)
        }
        Value::Str(text) => Some(text.to_string()),
        _ => None,
    }).collect()
}

fn prop_paths(props: &Props, name: &str) -> Vec<String> {
    let Some(Value::Array(array)) = prop(props, name).map(unvar) else {
        return Vec::new();
    };
    array.inner().iter().filter_map(|item| as_str(item).map(|s| s.to_string())).collect()
}

fn prop_dict(props: &Props, name: &str) -> Vec<(String, PropVal)> {
    prop(props, name).map(|value| dict_props(value)).unwrap_or_default()
}

/// Keep a value alive as an owned D-Bus value.
pub fn owned(value: impl Into<Value<'static>>) -> Result<OwnedValue> {
    OwnedValue::try_from(value.into()).map_err(|e| anyhow!("dbus value: {e}"))
}
