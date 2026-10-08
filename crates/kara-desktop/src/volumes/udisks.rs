//! UDisks2 over the system bus: reading the devices, acting on them, and
//! hearing when they change.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use super::{Block, Drive, VolumeError};

const SERVICE: &str = "org.freedesktop.UDisks2";
const ROOT: &str = "/org/freedesktop/UDisks2";
const BLOCK: &str = "org.freedesktop.UDisks2.Block";
const DRIVE: &str = "org.freedesktop.UDisks2.Drive";
const FILESYSTEM: &str = "org.freedesktop.UDisks2.Filesystem";
const ENCRYPTED: &str = "org.freedesktop.UDisks2.Encrypted";
const LOOP: &str = "org.freedesktop.UDisks2.Loop";
const MANAGER: &str = "org.freedesktop.UDisks2.Manager";

type Properties = HashMap<String, OwnedValue>;
type Objects = HashMap<OwnedObjectPath, HashMap<zbus::names::OwnedInterfaceName, Properties>>;

/// A connection to the system bus, where UDisks2 lives.
pub fn connect() -> Result<Connection, VolumeError> {
    Connection::system().map_err(|_| VolumeError::Unavailable)
}

/// Every block device and drive UDisks2 knows of.
pub fn snapshot(connection: &Connection) -> Result<(Vec<Block>, Vec<Drive>), VolumeError> {
    let reply = connection
        .call_method(Some(SERVICE), ROOT, Some("org.freedesktop.DBus.ObjectManager"), "GetManagedObjects", &())
        .map_err(|_| VolumeError::Unavailable)?;
    let objects: Objects = reply.body().deserialize().map_err(|_| VolumeError::Unavailable)?;

    let mut blocks = Vec::new();
    let mut drives = Vec::new();
    for (path, interfaces) in &objects {
        let get = |name: &str| interfaces.iter().find(|(k, _)| k.as_str() == name).map(|(_, v)| v);
        if let Some(block) = get(BLOCK) {
            blocks.push(read_block(path.as_str(), block, get(FILESYSTEM), get(ENCRYPTED), get(LOOP)));
        }
        if let Some(drive) = get(DRIVE) {
            drives.push(read_drive(path.as_str(), drive));
        }
    }
    Ok((blocks, drives))
}

fn read_block(
    path: &str,
    block: &Properties,
    filesystem: Option<&Properties>,
    encrypted: Option<&Properties>,
    loop_device: Option<&Properties>,
) -> Block {
    Block {
        path: path.to_string(),
        device: bytes(block, "Device").map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default(),
        size: number(block, "Size"),
        id_usage: text(block, "IdUsage"),
        id_label: text(block, "IdLabel"),
        hint_ignore: flag(block, "HintIgnore"),
        hint_name: text(block, "HintName"),
        hint_icon_name: text(block, "HintIconName"),
        drive: object(block, "Drive"),
        crypto_backing: object(block, "CryptoBackingDevice"),
        configuration: configuration(block),
        mount_points: filesystem.map(|fs| {
            byte_strings(fs, "MountPoints")
                .into_iter()
                .map(|b| PathBuf::from(<std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(b)))
                .collect()
        }),
        cleartext: encrypted.map(|e| object(e, "CleartextDevice")),
        loop_uid: loop_device.map(|l| u32::try_from(number(l, "SetupByUID")).unwrap_or(u32::MAX)),
    }
}

fn read_drive(path: &str, drive: &Properties) -> Drive {
    let media: Vec<String> = match value(drive, "MediaCompatibility") {
        Some(Value::Array(array)) => array
            .inner()
            .iter()
            .filter_map(|v| match v {
                Value::Str(s) => Some(s.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Drive {
        path: path.to_string(),
        vendor: text(drive, "Vendor"),
        model: text(drive, "Model"),
        removable: flag(drive, "Removable"),
        media_removable: flag(drive, "MediaRemovable"),
        ejectable: flag(drive, "Ejectable"),
        can_power_off: flag(drive, "CanPowerOff"),
        optical: media.iter().any(|m| m.starts_with("optical")),
        connection_bus: text(drive, "ConnectionBus"),
    }
}

fn value<'a>(props: &'a Properties, key: &str) -> Option<&'a Value<'static>> {
    props.get(key).map(|v| &**v)
}

fn flag(props: &Properties, key: &str) -> bool {
    matches!(value(props, key), Some(Value::Bool(true)))
}

fn number(props: &Properties, key: &str) -> u64 {
    match value(props, key) {
        Some(Value::U64(n)) => *n,
        Some(Value::U32(n)) => u64::from(*n),
        _ => 0,
    }
}

fn text(props: &Properties, key: &str) -> String {
    match value(props, key) {
        Some(Value::Str(s)) => s.to_string(),
        _ => String::new(),
    }
}

/// An object path property; UDisks2 says "none" with `/`.
fn object(props: &Properties, key: &str) -> Option<String> {
    match value(props, key) {
        Some(Value::ObjectPath(p)) if p.as_str() != "/" => Some(p.to_string()),
        _ => None,
    }
}

fn byte_string(v: &Value<'_>) -> Option<Vec<u8>> {
    let Value::Array(array) = v else {
        return None;
    };
    let mut out: Vec<u8> = array
        .inner()
        .iter()
        .filter_map(|b| match b {
            Value::U8(b) => Some(*b),
            _ => None,
        })
        .collect();
    // UDisks2's byte strings carry their C terminator.
    while out.last() == Some(&0) {
        out.pop();
    }
    Some(out)
}

fn bytes(props: &Properties, key: &str) -> Option<Vec<u8>> {
    value(props, key).and_then(byte_string)
}

fn byte_strings(props: &Properties, key: &str) -> Vec<Vec<u8>> {
    match value(props, key) {
        Some(Value::Array(array)) => array.inner().iter().filter_map(byte_string).filter(|b| !b.is_empty()).collect(),
        _ => Vec::new(),
    }
}

/// The fstab entries of a block: `a(sa{sv})` with `dir` and `opts` as byte
/// strings.
fn configuration(block: &Properties) -> Vec<(String, String)> {
    let Some(Value::Array(entries)) = value(block, "Configuration") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.inner() {
        let Value::Structure(entry) = entry else {
            continue;
        };
        let [Value::Str(kind), Value::Dict(details)] = entry.fields() else {
            continue;
        };
        if kind.as_str() != "fstab" {
            continue;
        }
        let mut dir = String::new();
        let mut opts = String::new();
        for (key, v) in details.iter() {
            let Value::Str(key) = key else {
                continue;
            };
            let inner = match v {
                Value::Value(boxed) => boxed.as_ref(),
                other => other,
            };
            let Some(bytes) = byte_string(inner) else {
                continue;
            };
            match key.as_str() {
                "dir" => dir = String::from_utf8_lossy(&bytes).into_owned(),
                "opts" => opts = String::from_utf8_lossy(&bytes).into_owned(),
                _ => {}
            }
        }
        if !dir.is_empty() {
            out.push((dir, opts));
        }
    }
    out
}

/// Maps a UDisks2 D-Bus error to what the user is told.
fn failure(error: zbus::Error, what: &str) -> VolumeError {
    let zbus::Error::MethodError(name, message, _) = &error else {
        return VolumeError::Failed(error.to_string());
    };
    match name.as_str().trim_start_matches("org.freedesktop.UDisks2.Error.") {
        "DeviceBusy" => VolumeError::Busy(what.to_string()),
        "NotAuthorizedDismissed" | "Cancelled" => VolumeError::Dismissed,
        "NotAuthorized" | "NotAuthorizedCanObtain" => VolumeError::NotAuthorized,
        _ if name.as_str() == "org.freedesktop.DBus.Error.UnknownObject"
            || name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod" =>
        {
            VolumeError::Gone
        }
        _ => VolumeError::Failed(message.clone().unwrap_or_else(|| error.to_string())),
    }
}

fn no_options() -> HashMap<&'static str, Value<'static>> {
    HashMap::new()
}

/// Mounts the file system of `block` where UDisks2 decides (under
/// `/run/media/$USER` or `/media/$USER`), asking polkit as needed.
pub fn mount(connection: &Connection, block: &str, what: &str) -> Result<PathBuf, VolumeError> {
    let reply = connection
        .call_method(Some(SERVICE), block, Some(FILESYSTEM), "Mount", &(no_options(),))
        .map_err(|e| failure(e, what))?;
    let point: String = reply.body().deserialize().map_err(|e| VolumeError::Failed(e.to_string()))?;
    Ok(PathBuf::from(point))
}

pub fn unmount(connection: &Connection, block: &str, what: &str) -> Result<(), VolumeError> {
    connection
        .call_method(Some(SERVICE), block, Some(FILESYSTEM), "Unmount", &(no_options(),))
        .map(|_| ())
        .map_err(|e| failure(e, what))
}

/// Unlocks an encrypted container and returns its cleartext block.
pub fn unlock(connection: &Connection, container: &str, passphrase: &str, what: &str) -> Result<String, VolumeError> {
    let reply = connection
        .call_method(Some(SERVICE), container, Some(ENCRYPTED), "Unlock", &(passphrase, no_options()))
        .map_err(|e| match failure(e, what) {
            VolumeError::Failed(_) => VolumeError::WrongPassphrase(what.to_string()),
            other => other,
        })?;
    let cleartext: OwnedObjectPath = reply.body().deserialize().map_err(|e| VolumeError::Failed(e.to_string()))?;
    Ok(cleartext.to_string())
}

pub fn lock(connection: &Connection, container: &str, what: &str) -> Result<(), VolumeError> {
    connection
        .call_method(Some(SERVICE), container, Some(ENCRYPTED), "Lock", &(no_options(),))
        .map(|_| ())
        .map_err(|e| failure(e, what))
}

/// Takes a drive away once nothing on it is mounted: ejects the medium when
/// it has one to eject, powers it off when it can be, so the user may pull it
/// out.
pub fn release_drive(connection: &Connection, drive: &Drive, what: &str) -> Result<(), VolumeError> {
    let method = if drive.ejectable {
        "Eject"
    } else if drive.can_power_off {
        "PowerOff"
    } else {
        return Ok(());
    };
    connection
        .call_method(Some(SERVICE), drive.path.as_str(), Some(DRIVE), method, &(no_options(),))
        .map(|_| ())
        .map_err(|e| failure(e, what))
}

/// Detaches a loop device the user set up.
pub fn delete_loop(connection: &Connection, block: &str, what: &str) -> Result<(), VolumeError> {
    connection
        .call_method(Some(SERVICE), block, Some(LOOP), "Delete", &(no_options(),))
        .map(|_| ())
        .map_err(|e| failure(e, what))
}

/// Attaches a disk image read-only as a loop device and returns the block.
pub fn attach_image(connection: &Connection, image: &Path) -> Result<String, VolumeError> {
    let what = image.file_name().map_or_else(|| image.display().to_string(), |n| n.to_string_lossy().into_owned());
    let file = std::fs::File::open(image).map_err(|e| VolumeError::Failed(e.to_string()))?;
    let fd = zbus::zvariant::Fd::from(&file);
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("read-only", Value::Bool(true));
    let reply = connection
        .call_method(Some(SERVICE), "/org/freedesktop/UDisks2/Manager", Some(MANAGER), "LoopSetup", &(fd, options))
        .map_err(|e| failure(e, &what))?;
    let block: OwnedObjectPath = reply.body().deserialize().map_err(|e| VolumeError::Failed(e.to_string()))?;
    Ok(block.to_string())
}

/// Calls `changed` whenever UDisks2 announces anything — a device plugged,
/// a mount, a lock — at most every 200 ms: a USB stick arriving sends a
/// burst of signals, and one refresh of the pane is enough for all of them.
pub fn watch(changed: Arc<dyn Fn() + Send + Sync>) {
    let (tx, rx) = mpsc::channel::<()>();
    let listener = std::thread::Builder::new().name("kara-udisks".into()).spawn(move || {
        let Ok(connection) = Connection::system() else {
            return;
        };
        let Ok(rule) = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(SERVICE)
            .map(|b| b.build())
        else {
            return;
        };
        let Ok(messages) = zbus::blocking::MessageIterator::for_match_rule(rule, &connection, Some(256)) else {
            return;
        };
        for message in messages {
            if message.is_err() || tx.send(()).is_err() {
                break;
            }
        }
    });
    if listener.is_err() {
        return;
    }
    let _ = std::thread::Builder::new().name("kara-udisks-debounce".into()).spawn(move || {
        while rx.recv().is_ok() {
            // Quiet for 200 ms, or a second at most: a job that reports
            // progress non-stop must not hold the pane back forever.
            let started = std::time::Instant::now();
            while started.elapsed() < Duration::from_secs(1)
                && rx.recv_timeout(Duration::from_millis(200)).is_ok()
            {}
            changed();
        }
    });
}
