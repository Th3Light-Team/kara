//! Shared by the SFTP test files: a hermetic in-process SFTP server and the
//! helpers to reach it.
//!
//! The server is `russh`'s server with a `russh-sftp` handler serving a
//! tempdir on `127.0.0.1:<ephemeral>`. It answers like OpenSSH's
//! `sftp-server`: errno folded into the five v3 codes with OpenSSH's table
//! (`ENOTDIR`/`ELOOP` → `NO_SUCH_FILE`, most others → a bare «Failure»),
//! `readdir` with `.`/`..` and `lstat` attributes in pages of 100, plain
//! `rename` that never replaces (link + unlink for files, a stat check for the
//! rest), and the `posix-rename@openssh.com` / `fsync@openssh.com` /
//! `hardlink@openssh.com` extensions, each optional.
//!
//! Fault injection ([`Faults`]): a delay per request, a full stall, `FAILURE`
//! «No space left on device» past an offset, `PERMISSION_DENIED` under path
//! prefixes, dropping every connection after N bytes written or read, and a
//! frozen transport (nothing read or written: keepalives go unanswered).

#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::net::Shutdown;
use std::os::unix::fs::{DirBuilderExt, FileExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use kara_remote::sftp::SftpFactory;
use kara_remote::{DriveConfig, Prompt, PromptAnswer, PromptHandler};
use kara_vfs::Cancel;
use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::ssh_key::{LineEnding, PrivateKey, PublicKey};
use russh::server::{Auth, Msg, Session};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{
    Attrs, Data, File as SftpFile, FileAttributes, Handle, Name, OpenFlags, Packet, Status,
    StatusCode, Version,
};
use russh_sftp::server::StatusReply;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::runtime::Runtime;

pub use kara_remote::sftp::SftpBackend;

// ---------------------------------------------------------------------------
// Keys.

/// A deterministic Ed25519 key: the same seed gives the same key.
pub fn key_from_seed(seed: u8) -> PrivateKey {
    PrivateKey::from(Ed25519Keypair::from_seed(&[seed; 32]))
}

/// Writes `key` as an OpenSSH private key file (mode 0600).
pub fn write_key_file(dir: &Path, name: &str, key: &PrivateKey) -> io::Result<PathBuf> {
    let text = key
        .to_openssh(LineEnding::LF)
        .map_err(|e| io::Error::other(e.to_string()))?;
    let path = dir.join(name);
    fs::write(&path, text.as_bytes())?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    Ok(path)
}

/// An Ed25519 key encrypted with aes256-ctr + bcrypt; the passphrase is
/// [`ENCRYPTED_KEY_PASSPHRASE`]. (The fixture russh's own tests use.)
pub const ENCRYPTED_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jdHIAAAAGYmNyeXB0AAAAGAAAABD1phlku5
A2G7Q9iP+DcOc9AAAAEAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHeLC1lWiCYrXsf/
85O/pkbUFZ6OGIt49PX3nw8iRoXEAAAAkKRF0st5ZI7xxo9g6A4m4l6NarkQre3mycqNXQ
dP3jryYgvsCIBAA5jMWSjrmnOTXhidqcOy4xYCrAttzSnZ/cUadfBenL+DQq6neffw7j8r
0tbCxVGp6yCQlKrgSZf6c0Hy7dNEIU2bJFGxLe6/kWChcUAt/5Ll5rI7DVQPJdLgehLzvv
sJWR7W+cGvJ/vLsw==
-----END OPENSSH PRIVATE KEY-----
";
pub const ENCRYPTED_KEY_PASSPHRASE: &str = "test";

/// The public half of [`ENCRYPTED_KEY`], readable without the passphrase.
pub fn encrypted_key_public() -> io::Result<PublicKey> {
    let key = PrivateKey::from_openssh(ENCRYPTED_KEY).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(key.public_key().clone())
}

/// `<entry> <type> <base64>`, a plain known_hosts line.
pub fn known_hosts_line(entry: &str, key: &PublicKey) -> io::Result<String> {
    let text = key.to_openssh().map_err(|e| io::Error::other(e.to_string()))?;
    let mut fields = text.split_whitespace();
    let (Some(kind), Some(base64)) = (fields.next(), fields.next()) else {
        return Err(io::Error::other("bad key"));
    };
    Ok(format!("{entry} {kind} {base64}\n"))
}

/// `|1|salt|hmac` for `entry`, as `HashKnownHosts yes` writes it.
pub fn hashed_host(entry: &str, salt: &[u8]) -> io::Result<String> {
    use hmac::{Hmac, KeyInit, Mac};
    let mac = Hmac::<sha1::Sha1>::new_from_slice(salt).map_err(|e| io::Error::other(e.to_string()))?;
    let hash = mac.chain_update(entry.as_bytes()).finalize().into_bytes();
    Ok(format!(
        "|1|{}|{}",
        data_encoding::BASE64.encode(salt),
        data_encoding::BASE64.encode(&hash)
    ))
}

// ---------------------------------------------------------------------------
// Server options and faults.

pub const USER: &str = "kara";
pub const PASSWORD: &str = "correct horse battery staple";

#[derive(Clone)]
pub struct ServerOptions {
    pub user: String,
    pub password: Option<String>,
    pub keys: Vec<PublicKey>,
    pub host_key: PrivateKey,
    pub posix_rename: bool,
    pub fsync: bool,
    /// `FAILURE` messages carry `strerror` text instead of OpenSSH's «Failure».
    pub strerror_messages: bool,
    /// Plain `SSH_FXP_RENAME` is rename(2) and replaces an existing file, as
    /// some non-OpenSSH servers do.
    pub rename_overwrites: bool,
}

impl Default for ServerOptions {
    fn default() -> Self {
        ServerOptions {
            user: USER.to_owned(),
            password: Some(PASSWORD.to_owned()),
            keys: Vec::new(),
            host_key: key_from_seed(1),
            posix_rename: true,
            fsync: true,
            strerror_messages: false,
            rename_overwrites: false,
        }
    }
}

const OFF: u64 = u64::MAX;

/// Switches flipped by tests while the server runs.
pub struct Faults {
    pub delay_ms: AtomicU64,
    pub stall: AtomicBool,
    /// A write reaching past this offset fails with «No space left on device».
    pub no_space_after: AtomicU64,
    /// Requests on paths under these prefixes get `PERMISSION_DENIED`.
    pub deny: Mutex<Vec<String>>,
    /// The most a single read answers with (a short read below what was asked).
    pub read_cap: AtomicU64,
    /// Every connection is dropped once this many bytes were written (total).
    pub kill_after_written: AtomicU64,
    /// Every connection is dropped once this many bytes were read (total).
    pub kill_after_read: AtomicU64,
    pub written: AtomicU64,
    pub read: AtomicU64,
    pub requests: AtomicU64,
    /// Paths of every rename / posix-rename request, in order.
    pub renames: Mutex<Vec<(String, String)>>,
}

impl Default for Faults {
    fn default() -> Self {
        Faults {
            delay_ms: AtomicU64::new(0),
            stall: AtomicBool::new(false),
            no_space_after: AtomicU64::new(OFF),
            deny: Mutex::new(Vec::new()),
            read_cap: AtomicU64::new(OFF),
            kill_after_written: AtomicU64::new(OFF),
            kill_after_read: AtomicU64::new(OFF),
            written: AtomicU64::new(0),
            read: AtomicU64::new(0),
            requests: AtomicU64::new(0),
            renames: Mutex::new(Vec::new()),
        }
    }
}

/// The open TCP connections, to cut them.
#[derive(Default)]
struct Connections {
    sockets: Mutex<Vec<std::net::TcpStream>>,
    frozen: AtomicBool,
}

impl Connections {
    fn kill_all(&self) {
        if let Ok(mut sockets) = self.sockets.lock() {
            for socket in sockets.drain(..) {
                let _ = socket.shutdown(Shutdown::Both);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The server.

pub struct TestServer {
    runtime: Option<Runtime>,
    pub port: u16,
    dir: tempfile::TempDir,
    root: PathBuf,
    pub faults: Arc<Faults>,
    connections: Arc<Connections>,
    host_public: PublicKey,
}

impl TestServer {
    pub fn start(options: ServerOptions) -> io::Result<TestServer> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().join("srv").canonicalize().or_else(|_| {
            fs::create_dir(dir.path().join("srv"))?;
            dir.path().join("srv").canonicalize()
        })?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let listener = runtime.block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))?;
        let port = listener.local_addr()?.port();
        let faults = Arc::new(Faults::default());
        let connections = Arc::new(Connections::default());
        let host_public = options.host_key.public_key().clone();
        let config = Arc::new(russh::server::Config {
            auth_rejection_time: Duration::from_millis(1),
            auth_rejection_time_initial: Some(Duration::from_millis(0)),
            keys: vec![options.host_key.clone()],
            ..russh::server::Config::default()
        });
        let options = Arc::new(options);
        {
            let faults = Arc::clone(&faults);
            let connections = Arc::clone(&connections);
            let root = root.clone();
            runtime.spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        continue;
                    };
                    let Ok(std_stream) = stream.into_std() else {
                        continue;
                    };
                    if let Ok(clone) = std_stream.try_clone()
                        && let Ok(mut sockets) = connections.sockets.lock()
                    {
                        sockets.push(clone);
                    }
                    let Ok(stream) = tokio::net::TcpStream::from_std(std_stream) else {
                        continue;
                    };
                    let gate = Gate {
                        inner: stream,
                        connections: Arc::clone(&connections),
                    };
                    let handler = SshHandler {
                        options: Arc::clone(&options),
                        faults: Arc::clone(&faults),
                        connections: Arc::clone(&connections),
                        root: root.clone(),
                        channels: HashMap::new(),
                    };
                    let config = Arc::clone(&config);
                    tokio::spawn(async move {
                        if let Ok(session) = russh::server::run_stream(config, gate, handler).await {
                            let _ = session.await;
                        }
                    });
                }
            });
        }
        Ok(TestServer {
            runtime: Some(runtime),
            port,
            dir,
            root,
            faults,
            connections,
            host_public,
        })
    }

    /// The local folder the server serves as `/`.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// A scratch folder next to the served one (for client-side files).
    pub fn scratch(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }

    pub fn host_key(&self) -> &PublicKey {
        &self.host_public
    }

    /// `[127.0.0.1]:<port>`.
    pub fn entry(&self) -> String {
        format!("[127.0.0.1]:{}", self.port)
    }

    /// Cuts every open connection (both directions).
    pub fn kill_connections(&self) {
        self.connections.kill_all();
    }

    /// Nothing is read or written any more on any connection, new or old.
    pub fn freeze(&self) {
        self.connections.frozen.store(true, Ordering::SeqCst);
    }

    pub fn deny(&self, prefix: &str) {
        if let Ok(mut deny) = self.faults.deny.lock() {
            deny.push(prefix.to_owned());
        }
    }

    /// Every `.kara-part` under the served folder.
    pub fn temporaries(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut stack = vec![self.root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.to_string_lossy().ends_with(".kara-part") {
                    found.push(path.clone());
                }
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    stack.push(path);
                }
            }
        }
        found
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.connections.kill_all();
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

/// The server side of a TCP connection, which a test can freeze.
struct Gate {
    inner: tokio::net::TcpStream,
    connections: Arc<Connections>,
}

impl AsyncRead for Gate {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.connections.frozen.load(Ordering::SeqCst) {
            return Poll::Pending;
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for Gate {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.connections.frozen.load(Ordering::SeqCst) {
            return Poll::Pending;
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.connections.frozen.load(Ordering::SeqCst) {
            return Poll::Pending;
        }
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

struct SshHandler {
    options: Arc<ServerOptions>,
    faults: Arc<Faults>,
    connections: Arc<Connections>,
    root: PathBuf,
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl russh::server::Handler for SshHandler {
    type Error = russh::Error;

    async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::reject())
    }

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        let ok = user == self.options.user && self.options.password.as_deref() == Some(password);
        Ok(if ok { Auth::Accept } else { Auth::reject() })
    }

    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        self.auth_publickey(user, key).await
    }

    async fn auth_publickey(&mut self, user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        let ok = user == self.options.user
            && self.options.keys.iter().any(|k| k.key_data() == key.key_data());
        Ok(if ok { Auth::Accept } else { Auth::reject() })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn channel_eof(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        session.close(channel)?;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        match (name, self.channels.remove(&channel_id)) {
            ("sftp", Some(channel)) => {
                session.channel_success(channel_id)?;
                let handler = SftpHandler {
                    options: Arc::clone(&self.options),
                    faults: Arc::clone(&self.faults),
                    connections: Arc::clone(&self.connections),
                    root: self.root.clone(),
                    handles: HashMap::new(),
                    next_handle: 0,
                };
                russh_sftp::server::run(channel.into_stream(), handler).await;
            }
            _ => session.channel_failure(channel_id)?,
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// The SFTP handler: OpenSSH's sftp-server, over a chroot-like folder.

enum Item {
    File { file: File, path: String },
    Dir { entries: VecDeque<SftpFile>, path: String },
}

struct SftpHandler {
    options: Arc<ServerOptions>,
    faults: Arc<Faults>,
    connections: Arc<Connections>,
    root: PathBuf,
    handles: HashMap<String, Item>,
    next_handle: u64,
}

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: String::from("Ok"),
        language_tag: String::from("en-US"),
    }
}

/// Reads one SSH string from extended-request data.
fn take_string(data: &mut &[u8]) -> Option<String> {
    let (len, rest) = data.split_first_chunk::<4>()?;
    let len = u32::from_be_bytes(*len) as usize;
    let text = rest.get(..len)?;
    *data = rest.get(len..)?;
    String::from_utf8(text.to_vec()).ok()
}

fn attrs_of(m: &fs::Metadata) -> FileAttributes {
    FileAttributes {
        size: Some(m.len()),
        uid: Some(m.uid()),
        user: None,
        gid: Some(m.gid()),
        group: None,
        permissions: Some(m.mode()),
        atime: Some(u32::try_from(m.atime().max(0)).unwrap_or(u32::MAX)),
        mtime: Some(u32::try_from(m.mtime().max(0)).unwrap_or(u32::MAX)),
    }
}

impl SftpHandler {
    /// OpenSSH's errno_to_portable.
    fn status(&self, error: &io::Error) -> StatusReply {
        let code = match error.raw_os_error() {
            Some(2 | 20 | 9 | 40) => StatusCode::NoSuchFile,
            Some(1 | 13 | 14) => StatusCode::PermissionDenied,
            Some(36 | 22) => StatusCode::BadMessage,
            Some(38) => StatusCode::OpUnsupported,
            _ => match error.kind() {
                io::ErrorKind::NotFound => StatusCode::NoSuchFile,
                io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
                _ => StatusCode::Failure,
            },
        };
        if self.options.strerror_messages {
            code.with_message(error.to_string())
        } else {
            StatusReply::new(code)
        }
    }

    fn local(&self, path: &str) -> PathBuf {
        let mut out = self.root.clone();
        for segment in path.split('/') {
            match segment {
                "" | "." => {}
                ".." => {
                    if out != self.root {
                        out.pop();
                    }
                }
                name => out.push(name),
            }
        }
        out
    }

    /// Delay, stall and deny, applied to every request.
    async fn gate(&self, path: Option<&str>) -> Result<(), StatusReply> {
        self.faults.requests.fetch_add(1, Ordering::SeqCst);
        let delay = self.faults.delay_ms.load(Ordering::SeqCst);
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        while self.faults.stall.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if let Some(path) = path {
            let normal = self.normal(path);
            let denied = self
                .faults
                .deny
                .lock()
                .map(|deny| deny.iter().any(|prefix| normal.starts_with(prefix.as_str())))
                .unwrap_or(false);
            if denied {
                return Err(StatusReply::new(StatusCode::PermissionDenied));
            }
        }
        Ok(())
    }

    fn normal(&self, path: &str) -> String {
        let local = self.local(path);
        let rest = local.strip_prefix(&self.root).unwrap_or(Path::new(""));
        format!("/{}", rest.to_string_lossy())
    }

    fn handle_path(&self, handle: &str) -> Option<String> {
        match self.handles.get(handle)? {
            Item::File { path, .. } | Item::Dir { path, .. } => Some(path.clone()),
        }
    }

    fn new_handle(&mut self, item: Item) -> String {
        self.next_handle += 1;
        let handle = self.next_handle.to_string();
        self.handles.insert(handle.clone(), item);
        handle
    }

    fn file(&self, handle: &str) -> Result<&File, StatusReply> {
        match self.handles.get(handle) {
            Some(Item::File { file, .. }) => Ok(file),
            Some(Item::Dir { .. }) => Err(StatusReply::new(StatusCode::Failure)),
            None => Err(StatusReply::new(StatusCode::Failure)),
        }
    }

    fn rename_like_openssh(&self, old: &Path, new: &Path) -> io::Result<()> {
        let meta = fs::symlink_metadata(old)?;
        if meta.is_file() {
            match fs::hard_link(old, new) {
                Ok(()) => {
                    if let Err(error) = fs::remove_file(old) {
                        let _ = fs::remove_file(new);
                        return Err(error);
                    }
                    Ok(())
                }
                Err(error) if matches!(error.raw_os_error(), Some(95 | 38 | 18)) => {
                    if fs::metadata(new).is_ok() {
                        return Err(io::Error::other("Failure"));
                    }
                    fs::rename(old, new)
                }
                Err(error) => Err(error),
            }
        } else if fs::metadata(new).is_err() {
            fs::rename(old, new)
        } else {
            Err(io::Error::other("Failure"))
        }
    }
}

impl russh_sftp::server::Handler for SftpHandler {
    type Error = StatusReply;

    fn unimplemented(&self) -> Self::Error {
        StatusReply::new(StatusCode::OpUnsupported)
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        let mut version = Version::new();
        version.version = 3;
        if self.options.posix_rename {
            version
                .extensions
                .insert(String::from("posix-rename@openssh.com"), String::from("1"));
        }
        if self.options.fsync {
            version
                .extensions
                .insert(String::from("fsync@openssh.com"), String::from("1"));
        }
        version
            .extensions
            .insert(String::from("hardlink@openssh.com"), String::from("1"));
        Ok(version)
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        self.gate(Some(&filename)).await?;
        let mut options = OpenOptions::new();
        options
            .read(pflags.contains(OpenFlags::READ))
            .write(pflags.contains(OpenFlags::WRITE))
            .append(pflags.contains(OpenFlags::APPEND))
            .truncate(pflags.contains(OpenFlags::TRUNCATE))
            .mode(attrs.permissions.unwrap_or(0o666) & 0o7777);
        if pflags.contains(OpenFlags::CREATE) {
            if pflags.contains(OpenFlags::EXCLUDE) {
                options.create_new(true);
            } else {
                options.create(true);
            }
        }
        let file = options
            .open(self.local(&filename))
            .map_err(|e| self.status(&e))?;
        let path = self.normal(&filename);
        let handle = self.new_handle(Item::File { file, path });
        Ok(Handle { id, handle })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.gate(None).await?;
        match self.handles.remove(&handle) {
            Some(_) => Ok(ok(id)),
            None => Err(StatusReply::new(StatusCode::Failure)),
        }
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        let path = self.handle_path(&handle);
        self.gate(path.as_deref()).await?;
        let file = self.file(&handle)?;
        let cap = self.faults.read_cap.load(Ordering::SeqCst).min(256 * 1024);
        let mut buf = vec![0u8; u64::from(len).min(cap) as usize];
        let n = file.read_at(&mut buf, offset).map_err(|e| self.status(&e))?;
        if n == 0 {
            return Err(StatusReply::new(StatusCode::Eof));
        }
        buf.truncate(n);
        let total = self.faults.read.fetch_add(n as u64, Ordering::SeqCst) + n as u64;
        if total >= self.faults.kill_after_read.load(Ordering::SeqCst) {
            self.connections.kill_all();
        }
        Ok(Data { id, data: buf })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        let path = self.handle_path(&handle);
        self.gate(path.as_deref()).await?;
        let end = offset + data.len() as u64;
        if end > self.faults.no_space_after.load(Ordering::SeqCst) {
            return Err(StatusCode::Failure.with_message("No space left on device"));
        }
        let file = self.file(&handle)?;
        file.write_all_at(&data, offset).map_err(|e| self.status(&e))?;
        let total = self.faults.written.fetch_add(data.len() as u64, Ordering::SeqCst)
            + data.len() as u64;
        if total >= self.faults.kill_after_written.load(Ordering::SeqCst) {
            self.connections.kill_all();
        }
        Ok(ok(id))
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        self.gate(Some(&path)).await?;
        let meta = fs::symlink_metadata(self.local(&path)).map_err(|e| self.status(&e))?;
        Ok(Attrs { id, attrs: attrs_of(&meta) })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        self.gate(Some(&path)).await?;
        let meta = fs::metadata(self.local(&path)).map_err(|e| self.status(&e))?;
        Ok(Attrs { id, attrs: attrs_of(&meta) })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        self.gate(None).await?;
        let meta = self.file(&handle)?.metadata().map_err(|e| self.status(&e))?;
        Ok(Attrs { id, attrs: attrs_of(&meta) })
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        self.gate(Some(&path)).await?;
        let local = self.local(&path);
        let reader = fs::read_dir(&local).map_err(|e| self.status(&e))?;
        let mut entries = VecDeque::new();
        for (name, target) in [(".", local.clone()), ("..", local.parent().map(Path::to_path_buf).unwrap_or(local.clone()))] {
            if let Ok(meta) = fs::symlink_metadata(&target) {
                entries.push_back(SftpFile::new(name, attrs_of(&meta)));
            }
        }
        for entry in reader.flatten() {
            if let Ok(meta) = fs::symlink_metadata(entry.path()) {
                entries.push_back(SftpFile::new(
                    entry.file_name().to_string_lossy().into_owned(),
                    attrs_of(&meta),
                ));
            }
        }
        let path = self.normal(&path);
        let handle = self.new_handle(Item::Dir { entries, path });
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        let path = self.handle_path(&handle);
        self.gate(path.as_deref()).await?;
        match self.handles.get_mut(&handle) {
            Some(Item::Dir { entries, .. }) => {
                if entries.is_empty() {
                    return Err(StatusReply::new(StatusCode::Eof));
                }
                let take = entries.len().min(100);
                let files = entries.drain(..take).collect();
                Ok(Name { id, files })
            }
            _ => Err(StatusReply::new(StatusCode::Failure)),
        }
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        self.gate(Some(&filename)).await?;
        fs::remove_file(self.local(&filename)).map_err(|e| self.status(&e))?;
        Ok(ok(id))
    }

    async fn mkdir(&mut self, id: u32, path: String, attrs: FileAttributes) -> Result<Status, Self::Error> {
        self.gate(Some(&path)).await?;
        fs::DirBuilder::new()
            .mode(attrs.permissions.unwrap_or(0o777) & 0o7777)
            .create(self.local(&path))
            .map_err(|e| self.status(&e))?;
        Ok(ok(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        self.gate(Some(&path)).await?;
        fs::remove_dir(self.local(&path)).map_err(|e| self.status(&e))?;
        Ok(ok(id))
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        self.gate(Some(&path)).await?;
        let real = fs::canonicalize(self.local(&path)).map_err(|e| self.status(&e))?;
        let rest = real
            .strip_prefix(&self.root)
            .map_err(|_| StatusReply::new(StatusCode::NoSuchFile))?;
        let name = format!("/{}", rest.to_string_lossy());
        Ok(Name {
            id,
            files: vec![SftpFile::dummy(name)],
        })
    }

    async fn rename(&mut self, id: u32, oldpath: String, newpath: String) -> Result<Status, Self::Error> {
        self.gate(Some(&oldpath)).await?;
        self.gate(Some(&newpath)).await?;
        if let Ok(mut renames) = self.faults.renames.lock() {
            renames.push((self.normal(&oldpath), self.normal(&newpath)));
        }
        let (old, new) = (self.local(&oldpath), self.local(&newpath));
        if self.options.rename_overwrites {
            fs::rename(&old, &new).map_err(|e| self.status(&e))?;
        } else {
            self.rename_like_openssh(&old, &new).map_err(|e| self.status(&e))?;
        }
        Ok(ok(id))
    }

    async fn extended(&mut self, id: u32, request: String, data: Vec<u8>) -> Result<Packet, Self::Error> {
        self.gate(None).await?;
        let mut rest = data.as_slice();
        match request.as_str() {
            "posix-rename@openssh.com" if self.options.posix_rename => {
                let (Some(old), Some(new)) = (take_string(&mut rest), take_string(&mut rest)) else {
                    return Err(StatusReply::new(StatusCode::BadMessage));
                };
                self.gate(Some(&old)).await?;
                self.gate(Some(&new)).await?;
                if let Ok(mut renames) = self.faults.renames.lock() {
                    renames.push((self.normal(&old), self.normal(&new)));
                }
                fs::rename(self.local(&old), self.local(&new)).map_err(|e| self.status(&e))?;
                Ok(Packet::Status(ok(id)))
            }
            "fsync@openssh.com" if self.options.fsync => {
                let Some(handle) = take_string(&mut rest) else {
                    return Err(StatusReply::new(StatusCode::BadMessage));
                };
                self.file(&handle)?.sync_all().map_err(|e| self.status(&e))?;
                Ok(Packet::Status(ok(id)))
            }
            "hardlink@openssh.com" => {
                let (Some(old), Some(new)) = (take_string(&mut rest), take_string(&mut rest)) else {
                    return Err(StatusReply::new(StatusCode::BadMessage));
                };
                fs::hard_link(self.local(&old), self.local(&new)).map_err(|e| self.status(&e))?;
                Ok(Packet::Status(ok(id)))
            }
            _ => Err(StatusReply::new(StatusCode::OpUnsupported)),
        }
    }
}

// ---------------------------------------------------------------------------
// Client side helpers.

/// Answers prompts from a script and records what was asked.
#[derive(Default)]
pub struct Scripted {
    pub answers: Mutex<VecDeque<PromptAnswer>>,
    pub asked: Mutex<Vec<Prompt>>,
}

impl Scripted {
    pub fn new(answers: impl IntoIterator<Item = PromptAnswer>) -> Scripted {
        Scripted {
            answers: Mutex::new(answers.into_iter().collect()),
            asked: Mutex::new(Vec::new()),
        }
    }

    pub fn asked(&self) -> Vec<Prompt> {
        self.asked.lock().map(|a| a.clone()).unwrap_or_default()
    }
}

impl PromptHandler for Scripted {
    fn ask(&self, prompt: &Prompt) -> PromptAnswer {
        if let Ok(mut asked) = self.asked.lock() {
            asked.push(prompt.clone());
        }
        self.answers
            .lock()
            .ok()
            .and_then(|mut answers| answers.pop_front())
            .unwrap_or(PromptAnswer::Refuse)
    }
}

/// A drive config for `server`, with `known_hosts` in `client_dir` and no agent.
pub fn drive_config(
    server: &TestServer,
    name: &str,
    known_hosts: &Path,
    extra: &[(&str, &str)],
) -> io::Result<DriveConfig> {
    let mut params: Vec<(String, String)> = vec![
        (String::from("host"), String::from("127.0.0.1")),
        (String::from("port"), server.port.to_string()),
        (String::from("user"), String::from(USER)),
        (String::from("known_hosts"), known_hosts.to_string_lossy().into_owned()),
        (String::from("agent"), String::from("none")),
    ];
    for (key, value) in extra {
        params.retain(|(k, _)| k != key);
        params.push(((*key).to_owned(), (*value).to_owned()));
    }
    DriveConfig::new("sftp", name, name, params).map_err(|e| io::Error::other(e.to_string()))
}

/// A known_hosts file in `dir` that already trusts the server.
pub fn trusting_known_hosts(server: &TestServer, dir: &Path) -> io::Result<PathBuf> {
    let path = dir.join("known_hosts");
    fs::write(&path, known_hosts_line(&server.entry(), server.host_key())?)?;
    Ok(path)
}

/// A server, a client folder that trusts it, and a connected backend rooted at `/`.
pub struct Fixture {
    pub server: TestServer,
    pub client: tempfile::TempDir,
    pub known_hosts: PathBuf,
    pub backend: Arc<SftpBackend>,
}

impl Fixture {
    pub fn new(options: ServerOptions, extra: &[(&str, &str)]) -> io::Result<Fixture> {
        let server = TestServer::start(options)?;
        let client = tempfile::tempdir()?;
        let known_hosts = trusting_known_hosts(&server, client.path())?;
        let backend = connect(&server, &known_hosts, extra)?;
        Ok(Fixture {
            server,
            client,
            known_hosts,
            backend,
        })
    }

    /// Default server, password auth.
    pub fn standard() -> io::Result<Fixture> {
        Fixture::new(ServerOptions::default(), &[])
    }

    /// A second backend to the same server.
    pub fn reconnect(&self, extra: &[(&str, &str)]) -> io::Result<Arc<SftpBackend>> {
        connect(&self.server, &self.known_hosts, extra)
    }
}

/// Connects with the password and `extra` params, refusing any prompt.
pub fn connect(
    server: &TestServer,
    known_hosts: &Path,
    extra: &[(&str, &str)],
) -> io::Result<Arc<SftpBackend>> {
    let config = drive_config(server, "test", known_hosts, extra)?;
    let secret = kara_remote::Secret::new(PASSWORD);
    SftpFactory::new()
        .open(&config, Some(&secret), &Scripted::default(), &Cancel::new())
        .map_err(|e| io::Error::other(format!("connect: {e}")))
}

/// Parses a remote path or fails the test with an io error.
pub fn rp(text: &str) -> io::Result<kara_vfs::RemotePath> {
    kara_vfs::RemotePath::parse(text).map_err(|e| io::Error::other(e.to_string()))
}

/// Starts an SSH agent on a socket in `dir` holding `keys`; returns the socket.
pub fn start_agent(runtime: &Runtime, dir: &Path, keys: &[PrivateKey]) -> io::Result<PathBuf> {
    use russh::keys::agent::client::AgentClient;
    let socket = dir.join("agent.sock");
    let listener = {
        let _guard = runtime.enter();
        tokio::net::UnixListener::bind(&socket)?
    };
    let stream = Box::pin(futures::stream::unfold(listener, |listener| async move {
        let next = listener.accept().await.map(|(stream, _)| stream);
        Some((next, listener))
    }));
    runtime.spawn(async move {
        let _ = russh::keys::agent::server::serve(stream, ()).await;
    });
    runtime.block_on(async {
        let mut client = AgentClient::connect_uds(&socket)
            .await
            .map_err(|e| io::Error::other(e.to_string()))?;
        for key in keys {
            client
                .add_identity(key, &[])
                .await
                .map_err(|e| io::Error::other(e.to_string()))?;
        }
        Ok::<(), io::Error>(())
    })?;
    Ok(socket)
}
