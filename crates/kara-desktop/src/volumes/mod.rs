//! Drives, partitions and network locations for the navigation pane.
//!
//! Reference convenience: `ground/spec/06-contexto-power.md`, «Montar y
//! expulsar unidades»: the drives show in the side bar with an eject button;
//! ejecting unmounts and says when the device can be pulled out; mounting a
//! partition or an image makes it browsable; an encrypted volume asks for its
//! passphrase; network locations disconnect the same way.
//!
//! Local storage comes from UDisks2 over D-Bus — the service both GNOME
//! (through gvfs) and Plasma (through Solid) sit on — so mount, unmount,
//! unlock and eject go through polkit exactly as they would from Nautilus or
//! Dolphin. Which devices to show follows gvfs's rules, which is what users of
//! either desktop see in their file manager. Network locations are the gvfs
//! FUSE mounts under `$XDG_RUNTIME_DIR/gvfs`, where GNOME puts every SMB,
//! SFTP, WebDAV or MTP location it mounts.

pub mod gvfs;
pub mod udisks;

use std::path::{Path, PathBuf};

/// What sort of thing a volume is, for its icon and for what «eject» means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeKind {
    /// A USB stick, a card, an external disk.
    Removable,
    /// A CD, DVD or Blu-ray.
    Optical,
    /// A partition of an internal disk.
    Fixed,
    /// A disk image the user mounted (a loop device).
    Image,
    /// A gvfs network location.
    Network,
    /// A phone or a media player (MTP, AFC).
    Phone,
    /// A camera (PTP).
    Camera,
}

/// Where a network location points, for the view to name it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkLocation {
    /// gvfs backend: `smb-share`, `sftp`, `dav`, `ftp`, `mtp`…
    pub scheme: String,
    pub host: Option<String>,
    pub share: Option<String>,
    pub user: Option<String>,
    /// The name gvfs itself gives it, when gvfs said.
    pub display_name: Option<String>,
}

/// Something the navigation pane lists under «Este equipo» or «Red».
#[derive(Debug, Clone, PartialEq)]
pub struct Volume {
    /// Stable for the session: the UDisks2 object path of the block device
    /// (of the encrypted container, for an encrypted volume), or `gvfs:`
    /// and the FUSE directory name.
    pub id: String,
    /// What the system calls it: the file system label, a hint from udev, or
    /// gvfs's name. `None` when it has none; the view then names it by size.
    pub label: Option<String>,
    /// Bytes.
    pub size: u64,
    pub kind: VolumeKind,
    /// Icon theme names, most specific first.
    pub icons: Vec<String>,
    /// Where it is mounted, if it is.
    pub mount_point: Option<PathBuf>,
    /// The block whose file system to mount or unmount: the volume itself,
    /// or the cleartext side of an unlocked encrypted one. `None` while
    /// locked, and for what UDisks2 does not manage.
    pub filesystem: Option<String>,
    /// An encrypted volume that still needs its passphrase.
    pub locked: bool,
    /// The drive it belongs to: «expulsar» acts on all of a drive's volumes.
    pub drive: Option<String>,
    /// «SanDisk Ultra», for the message that the drive can be removed.
    pub drive_name: Option<String>,
    pub network: Option<NetworkLocation>,
}

impl Volume {
    /// Whether the pane offers the eject button: a removable medium, an image,
    /// a network location, or any mounted partition (which it unmounts).
    #[must_use]
    pub fn can_eject(&self) -> bool {
        match self.kind {
            VolumeKind::Removable | VolumeKind::Optical | VolumeKind::Image => true,
            VolumeKind::Network | VolumeKind::Phone | VolumeKind::Camera => self.mount_point.is_some(),
            VolumeKind::Fixed => self.mount_point.is_some(),
        }
    }

    /// Whether ejecting means taking a device away (and the user is told
    /// when it is safe to pull it out) rather than just unmounting.
    #[must_use]
    pub fn ejects_hardware(&self) -> bool {
        matches!(self.kind, VolumeKind::Removable | VolumeKind::Optical)
    }
}

/// What the user asked a volume to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VolumeAction {
    /// Mount it (unlocking first is a separate step).
    Mount,
    /// Unlock an encrypted volume with this passphrase, then mount it.
    Unlock(String),
    /// Unmount it; for a removable drive, also power it off or eject the
    /// medium so it can be pulled out; for an image, detach it.
    Eject,
}

/// Why a volume action did not happen. The messages are the ones the window
/// shows, except [`VolumeError::Dismissed`], which shows nothing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VolumeError {
    #[error("«{0}» está en uso: cierra los archivos abiertos en él y vuelve a intentarlo")]
    Busy(String),
    #[error("no tienes permiso para hacerlo")]
    NotAuthorized,
    /// The user closed the password prompt. Not an error to report.
    #[error("cancelado")]
    Dismissed,
    #[error("no se pudo desbloquear «{0}»: comprueba la contraseña")]
    WrongPassphrase(String),
    #[error("ese volumen ya no está")]
    Gone,
    #[error("no se puede gestionar unidades: el servicio UDisks2 no responde")]
    Unavailable,
    #[error("{0}")]
    Failed(String),
}

/// A UDisks2 block device, as read from the bus.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Block {
    pub path: String,
    pub device: String,
    pub size: u64,
    pub id_usage: String,
    pub id_label: String,
    pub hint_ignore: bool,
    pub hint_name: String,
    pub hint_icon_name: String,
    pub drive: Option<String>,
    /// For the cleartext side of an unlocked encrypted volume: the
    /// container it comes from.
    pub crypto_backing: Option<String>,
    /// fstab entries: mount directory and options.
    pub configuration: Vec<(String, String)>,
    /// `Some` when the block has a file system (the `Filesystem` interface),
    /// with where it is mounted.
    pub mount_points: Option<Vec<PathBuf>>,
    /// `Some` for an encrypted container: its cleartext block once unlocked.
    pub cleartext: Option<Option<String>>,
    /// `Some` for a loop device: the uid that set it up.
    pub loop_uid: Option<u32>,
}

/// A UDisks2 drive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Drive {
    pub path: String,
    pub vendor: String,
    pub model: String,
    pub removable: bool,
    pub media_removable: bool,
    pub ejectable: bool,
    pub can_power_off: bool,
    pub optical: bool,
    pub connection_bus: String,
}

impl Drive {
    fn name(&self) -> Option<String> {
        let name = format!("{} {}", self.vendor.trim(), self.model.trim()).trim().to_string();
        (!name.is_empty()).then_some(name)
    }
}

/// What the machine looks like to the classifier.
#[derive(Debug, Clone, Default)]
pub struct Context {
    pub uid: u32,
    pub home: Option<PathBuf>,
    /// Mount options by mount point, from the mount table.
    pub mount_options: Vec<(PathBuf, String)>,
}

/// Turns UDisks2's devices into the volumes the pane shows, by gvfs's rules
/// (`gvfsudisks2volumemonitor.c`, `should_include_volume`):
///
/// - `HintIgnore` hides it, whatever else is true.
/// - A loop device is shown only if root or this user set it up, and is not
///   empty.
/// - An encrypted container is shown locked, and once unlocked as its
///   cleartext file system — never both.
/// - Everything else needs a file system, and every place it is mounted must
///   be one users browse; unmounted, its fstab entry, if any, must be too.
#[must_use]
pub fn classify(blocks: &[Block], drives: &[Drive], context: &Context) -> Vec<Volume> {
    let mut volumes = Vec::new();
    for block in blocks {
        if block.hint_ignore || block.crypto_backing.is_some() {
            continue;
        }
        if let Some(uid) = block.loop_uid
            && ((uid != 0 && uid != context.uid) || block.size == 0)
        {
            continue;
        }

        // For an encrypted container, the file system is its cleartext side.
        let (filesystem, locked) = match &block.cleartext {
            Some(Some(cleartext)) => match blocks.iter().find(|b| &b.path == cleartext) {
                Some(inner) => (inner, false),
                None => continue,
            },
            Some(None) => (block, true),
            None => (block, false),
        };
        if !locked && filesystem.mount_points.is_none() {
            continue;
        }

        let mounts = filesystem.mount_points.clone().unwrap_or_default();
        if !mounts.iter().all(|m| context.shows(m, context.options_of(m))) {
            continue;
        }
        if mounts.is_empty()
            && !filesystem
                .configuration
                .iter()
                .chain(&block.configuration)
                .all(|(dir, opts)| context.shows(Path::new(dir), opts))
        {
            continue;
        }

        let drive = block.drive.as_ref().and_then(|d| drives.iter().find(|x| &x.path == d));
        let kind = if block.loop_uid.is_some() {
            VolumeKind::Image
        } else {
            match drive {
                Some(d) if d.optical => VolumeKind::Optical,
                Some(d) if d.removable || d.media_removable || d.connection_bus == "usb" => VolumeKind::Removable,
                _ => VolumeKind::Fixed,
            }
        };

        let label = [&filesystem.id_label, &block.id_label, &block.hint_name]
            .into_iter()
            .map(|l| l.trim())
            .find(|l| !l.is_empty())
            .map(String::from);

        let mut icons = Vec::new();
        if !block.hint_icon_name.is_empty() {
            icons.push(block.hint_icon_name.clone());
        }
        icons.extend(default_icons(kind).iter().map(|s| (*s).to_string()));

        volumes.push(Volume {
            id: block.path.clone(),
            label,
            size: block.size,
            kind,
            icons,
            mount_point: mounts.first().cloned(),
            filesystem: (!locked).then(|| filesystem.path.clone()),
            locked,
            drive: block.drive.clone(),
            drive_name: drive.and_then(Drive::name),
            network: None,
        });
    }
    volumes.sort_by(|a, b| (a.kind == VolumeKind::Fixed).cmp(&(b.kind == VolumeKind::Fixed)).then_with(|| a.id.cmp(&b.id)));
    volumes
}

/// The local volumes: from UDisks2, or, where it does not answer, the mounted
/// file systems of the mount table in the places users browse.
#[must_use]
pub fn local_volumes() -> Vec<Volume> {
    let context = Context::current();
    match udisks::connect().and_then(|c| udisks::snapshot(&c)) {
        Ok((blocks, drives)) => classify(&blocks, &drives, &context),
        Err(_) => context
            .mount_options
            .iter()
            .filter(|(point, options)| context.shows(point, options))
            .map(|(point, _)| Volume {
                id: format!("mount:{}", point.display()),
                label: point.file_name().map(|n| n.to_string_lossy().into_owned()),
                size: 0,
                kind: VolumeKind::Fixed,
                icons: default_icons(VolumeKind::Fixed).iter().map(|s| (*s).to_string()).collect(),
                mount_point: Some(point.clone()),
                filesystem: None,
                locked: false,
                drive: None,
                drive_name: None,
                network: None,
            })
            .collect(),
    }
}

impl Context {
    /// This user, this home, this mount table.
    #[must_use]
    pub fn current() -> Self {
        Self {
            uid: rustix::process::getuid().as_raw(),
            home: std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.is_absolute()),
            mount_options: std::fs::read_to_string("/proc/self/mounts")
                .map(|t| parse_mount_table(&t))
                .unwrap_or_default(),
        }
    }
}

/// How the messages name a volume: its label, else its device.
fn display_name(volume: &Volume, blocks: &[Block]) -> String {
    volume.label.clone().unwrap_or_else(|| {
        blocks
            .iter()
            .find(|b| b.path == volume.id)
            .map(|b| b.device.clone())
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| volume.id.clone())
    })
}

/// Carries out `action` on volume `id` and says where it is mounted after.
/// Blocks on UDisks2 and, through it, on the polkit prompt: run it on a
/// thread of its own.
pub fn act(id: &str, action: &VolumeAction) -> Result<Option<PathBuf>, VolumeError> {
    if let Some(name) = id.strip_prefix("gvfs:") {
        let path = gvfs::fuse_root().ok_or(VolumeError::Gone)?.join(name);
        return match action {
            VolumeAction::Eject => gvfs::unmount(&path).map(|()| None),
            VolumeAction::Mount | VolumeAction::Unlock(_) => Ok(Some(path)),
        };
    }
    if let Some(point) = id.strip_prefix("mount:") {
        // A mount UDisks2 did not tell us about: it can only be browsed.
        return Ok(Some(PathBuf::from(point)));
    }

    let connection = udisks::connect()?;
    let (blocks, drives) = udisks::snapshot(&connection)?;
    let volumes = classify(&blocks, &drives, &Context::current());
    let volume = volumes.iter().find(|v| v.id == id).ok_or(VolumeError::Gone)?;
    let what = display_name(volume, &blocks);

    match action {
        VolumeAction::Mount => {
            if let Some(point) = &volume.mount_point {
                return Ok(Some(point.clone()));
            }
            let filesystem = volume.filesystem.as_deref().ok_or(VolumeError::WrongPassphrase(what.clone()))?;
            udisks::mount(&connection, filesystem, &what).map(Some)
        }
        VolumeAction::Unlock(passphrase) => {
            let cleartext = udisks::unlock(&connection, &volume.id, passphrase, &what)?;
            mount_when_ready(&connection, &cleartext, &what).map(Some)
        }
        VolumeAction::Eject => {
            if volume.kind == VolumeKind::Image {
                if let Some(filesystem) = &volume.filesystem
                    && volume.mount_point.is_some()
                {
                    udisks::unmount(&connection, filesystem, &what)?;
                }
                udisks::delete_loop(&connection, &volume.id, &what)?;
                return Ok(None);
            }
            // Everything on the drive goes, not just the partition that was
            // clicked: a stick with two partitions cannot be pulled out with
            // one of them still mounted.
            let siblings: Vec<&Volume> = match (&volume.drive, volume.ejects_hardware()) {
                (Some(drive), true) => volumes.iter().filter(|v| v.drive.as_ref() == Some(drive)).collect(),
                _ => vec![volume],
            };
            for sibling in &siblings {
                let name = display_name(sibling, &blocks);
                if let (Some(filesystem), Some(_)) = (&sibling.filesystem, &sibling.mount_point) {
                    udisks::unmount(&connection, filesystem, &name)?;
                }
                let encrypted = blocks.iter().any(|b| b.path == sibling.id && matches!(b.cleartext, Some(Some(_))));
                if encrypted {
                    udisks::lock(&connection, &sibling.id, &name)?;
                }
            }
            if volume.ejects_hardware()
                && let Some(drive) = volume.drive.as_ref().and_then(|d| drives.iter().find(|x| &x.path == d))
            {
                udisks::release_drive(&connection, drive, &what)?;
            }
            Ok(None)
        }
    }
}

/// A freshly unlocked or attached block gets its file system interface a
/// moment later, once udev has probed it; until then `Mount` does not exist.
fn mount_when_ready(connection: &zbus::blocking::Connection, block: &str, what: &str) -> Result<PathBuf, VolumeError> {
    let mut attempts = 0;
    loop {
        match udisks::mount(connection, block, what) {
            Err(VolumeError::Gone) if attempts < 30 => {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            other => return other,
        }
    }
}

/// Attaches a disk image (an ISO, an `.img`) read-only and mounts its file
/// system — on the whole device, or on its first partition for hybrid
/// images.
pub fn mount_image(image: &Path) -> Result<PathBuf, VolumeError> {
    let connection = udisks::connect()?;
    let what = image.file_name().map_or_else(|| image.display().to_string(), |n| n.to_string_lossy().into_owned());
    let device = udisks::attach_image(&connection, image)?;
    for _ in 0..50 {
        let (blocks, _) = udisks::snapshot(&connection)?;
        let Some(loop_block) = blocks.iter().find(|b| b.path == device) else {
            return Err(VolumeError::Gone);
        };
        let partition_prefix = format!("{}p", loop_block.device);
        let target = std::iter::once(loop_block)
            .chain(blocks.iter().filter(|b| !loop_block.device.is_empty() && b.device.starts_with(&partition_prefix)))
            .find(|b| b.mount_points.is_some());
        if let Some(target) = target {
            if let Some(point) = target.mount_points.as_ref().and_then(|m| m.first()) {
                return Ok(point.clone());
            }
            return mount_when_ready(&connection, &target.path, &what);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(VolumeError::Failed(format!("«{what}» no contiene un sistema de archivos que se pueda montar")))
}

/// Theme icon names for each kind, most specific first, ending in one every
/// theme has.
#[must_use]
pub fn default_icons(kind: VolumeKind) -> &'static [&'static str] {
    match kind {
        VolumeKind::Removable => &["drive-removable-media-usb", "drive-removable-media", "media-removable", "drive-harddisk"],
        VolumeKind::Optical => &["media-optical", "drive-optical", "drive-removable-media"],
        VolumeKind::Fixed => &["drive-harddisk", "drive-harddisk-system"],
        VolumeKind::Image => &["media-optical", "drive-removable-media", "drive-harddisk"],
        VolumeKind::Network => &["folder-remote", "network-server", "folder"],
        VolumeKind::Phone => &["phone", "multimedia-player", "folder-remote", "folder"],
        VolumeKind::Camera => &["camera-photo", "camera", "folder-remote", "folder"],
    }
}

impl Context {
    fn options_of(&self, mount: &Path) -> &str {
        self.mount_options
            .iter()
            .rev()
            .find(|(m, _)| m == mount)
            .map_or("", |(_, opts)| opts.as_str())
    }

    /// gvfs's `should_include`: `x-gvfs-show` and `x-gvfs-hide` decide on
    /// their own; otherwise only places users browse — their home, and
    /// `/media` and `/run/media`. `/mnt` is shown too, as Kara always did and
    /// Dolphin does: people mount things there by hand on purpose.
    #[must_use]
    pub fn shows(&self, path: &Path, options: &str) -> bool {
        let has = |flag: &str| options.split(',').any(|o| o.trim() == flag);
        if has("x-gvfs-show") {
            return true;
        }
        if has("x-gvfs-hide") || has("x-gdu.hide") {
            return false;
        }
        if path.to_string_lossy().contains("/.") {
            return false;
        }
        let strictly_under = |root: &Path| path.starts_with(root) && path != root;
        if let Some(home) = &self.home
            && strictly_under(home)
        {
            return true;
        }
        ["/media", "/run/media", "/mnt"]
            .iter()
            .any(|root| strictly_under(Path::new(root)))
    }
}

/// Mount points and their options, from a mount table (`/proc/self/mounts`).
#[must_use]
pub fn parse_mount_table(text: &str) -> Vec<(PathBuf, String)> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let _source = fields.next()?;
            let point = fields.next()?;
            let _type = fields.next()?;
            let options = fields.next()?;
            Some((PathBuf::from(unescape_octal(point)), options.to_string()))
        })
        .collect()
}

/// The kernel writes space, tab, newline and backslash as `\NNN` octal.
fn unescape_octal(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1..i + 4].iter().all(|b| (b'0'..=b'7').contains(b))
        {
            let digit = |k: usize| u32::from(bytes[i + k] - b'0');
            // `\777` does not fit a byte; it is not an escape the kernel writes.
            if let Ok(value) = u8::try_from(digit(1) * 64 + digit(2) * 8 + digit(3)) {
                out.push(value);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> Context {
        Context {
            uid: 1000,
            home: Some(PathBuf::from("/home/ana")),
            mount_options: Vec::new(),
        }
    }

    fn fs(path: &str, mounts: &[&str]) -> Block {
        Block {
            path: path.to_string(),
            size: 32_000_000_000,
            id_usage: "filesystem".into(),
            mount_points: Some(mounts.iter().map(PathBuf::from).collect()),
            ..Block::default()
        }
    }

    fn usb() -> Drive {
        Drive {
            path: "/drives/usb".into(),
            vendor: "SanDisk".into(),
            model: "Ultra".into(),
            removable: true,
            media_removable: true,
            ejectable: true,
            connection_bus: "usb".into(),
            ..Drive::default()
        }
    }

    #[test]
    fn a_mounted_usb_stick_is_a_removable_volume_with_its_label() {
        let mut stick = fs("/b/sdb1", &["/run/media/ana/FOTOS"]);
        stick.id_label = "FOTOS".into();
        stick.drive = Some("/drives/usb".into());
        let volumes = classify(&[stick], &[usb()], &context());
        assert_eq!(volumes.len(), 1);
        let v = &volumes[0];
        assert_eq!(v.kind, VolumeKind::Removable);
        assert_eq!(v.label.as_deref(), Some("FOTOS"));
        assert_eq!(v.mount_point.as_deref(), Some(Path::new("/run/media/ana/FOTOS")));
        assert_eq!(v.drive_name.as_deref(), Some("SanDisk Ultra"));
        assert!(v.can_eject() && v.ejects_hardware());
    }

    #[test]
    fn the_system_partitions_stay_out_of_the_pane() {
        let root = fs("/b/nvme0n1p2", &["/"]);
        let boot = fs("/b/nvme0n1p1", &["/boot/efi"]);
        let snap = fs("/b/loop3", &["/snap/core24/1643"]);
        assert!(classify(&[root, boot, snap], &[], &context()).is_empty());
    }

    #[test]
    fn hint_ignore_wins_over_everything() {
        let mut hidden = fs("/b/sdb1", &["/media/ana/x"]);
        hidden.hint_ignore = true;
        assert!(classify(&[hidden], &[], &context()).is_empty());
    }

    #[test]
    fn an_unmounted_partition_shows_unless_fstab_puts_it_somewhere_private() {
        let free = fs("/b/sda1", &[]);
        let mut fstab = fs("/b/sda2", &[]);
        fstab.configuration = vec![("/srv/data".into(), "defaults".into())];
        let mut shown = fs("/b/sda3", &[]);
        shown.configuration = vec![("/srv/media".into(), "defaults,x-gvfs-show".into())];
        let volumes = classify(&[free, fstab, shown], &[], &context());
        let ids: Vec<&str> = volumes.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["/b/sda1", "/b/sda3"]);
        assert!(volumes.iter().all(|v| v.kind == VolumeKind::Fixed && !v.can_eject()));
    }

    #[test]
    fn mount_options_can_hide_a_volume_in_a_browsable_place() {
        let mut ctx = context();
        ctx.mount_options = vec![(PathBuf::from("/media/ana/x"), "rw,x-gvfs-hide".into())];
        assert!(classify(&[fs("/b/sdb1", &["/media/ana/x"])], &[], &ctx).is_empty());
    }

    #[test]
    fn an_encrypted_volume_is_shown_once_locked_or_unlocked() {
        let mut container = Block {
            path: "/b/sdc1".into(),
            id_usage: "crypto".into(),
            size: 64,
            cleartext: Some(None),
            ..Block::default()
        };
        let locked = classify(std::slice::from_ref(&container), &[], &context());
        assert_eq!(locked.len(), 1);
        assert!(locked[0].locked);

        container.cleartext = Some(Some("/b/dm_0".into()));
        let mut inner = fs("/b/dm_0", &["/run/media/ana/Secreto"]);
        inner.crypto_backing = Some("/b/sdc1".into());
        inner.id_label = "Secreto".into();
        let unlocked = classify(&[container, inner], &[], &context());
        assert_eq!(unlocked.len(), 1);
        assert_eq!(unlocked[0].id, "/b/sdc1");
        assert!(!unlocked[0].locked);
        assert_eq!(unlocked[0].label.as_deref(), Some("Secreto"));
    }

    #[test]
    fn loop_devices_of_other_users_and_empty_ones_are_hidden() {
        let mut mine = fs("/b/loop0", &["/run/media/ana/ISO"]);
        mine.loop_uid = Some(1000);
        let mut theirs = fs("/b/loop1", &["/run/media/bob/ISO"]);
        theirs.loop_uid = Some(1001);
        let mut empty = fs("/b/loop2", &[]);
        empty.loop_uid = Some(0);
        empty.size = 0;
        let volumes = classify(&[mine, theirs, empty], &[], &context());
        assert_eq!(volumes.len(), 1);
        assert_eq!(volumes[0].kind, VolumeKind::Image);
        assert!(volumes[0].can_eject());
    }

    #[test]
    fn a_block_without_a_file_system_is_not_a_volume() {
        let mut swap = fs("/b/sda4", &[]);
        swap.mount_points = None;
        assert!(classify(&[swap], &[], &context()).is_empty());
    }

    #[test]
    fn removable_volumes_are_listed_before_internal_ones() {
        let internal = fs("/b/a_internal", &[]);
        let mut stick = fs("/b/z_stick", &[]);
        stick.drive = Some("/drives/usb".into());
        let volumes = classify(&[internal, stick], &[usb()], &context());
        assert_eq!(volumes[0].kind, VolumeKind::Removable);
    }

    #[test]
    fn the_mount_table_is_unescaped() {
        let table = "/dev/sdb1 /run/media/ana/Mis\\040cosas vfat rw,nosuid 0 0\nproc /proc proc rw 0 0\n";
        let parsed = parse_mount_table(table);
        assert_eq!(parsed[0].0, PathBuf::from("/run/media/ana/Mis cosas"));
        assert_eq!(parsed[0].1, "rw,nosuid");
    }

    #[test]
    fn hidden_directories_and_the_roots_themselves_are_not_browsable() {
        let ctx = context();
        assert!(!ctx.shows(Path::new("/media"), ""));
        assert!(!ctx.shows(Path::new("/home/ana/.cache/x"), ""));
        assert!(ctx.shows(Path::new("/home/ana/Discos/x"), ""));
        assert!(ctx.shows(Path::new("/mnt/backup"), ""));
        assert!(!ctx.shows(Path::new("/srv/x"), ""));
    }
}
