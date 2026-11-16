//! Where the window manager is reached.
//!
//! By default over a **local endpoint** that only the user's own programs
//! can open: a Unix socket in the runtime folder on Linux
//! (`$XDG_RUNTIME_DIR/gevrek/<instance>.sock`, mode 0600), a named pipe on
//! Windows (`\\.\pipe\gevrek-<user>-<instance>`, not reachable from another
//! machine, writable by its creator, administrators and the system only).
//! The window manager can start programs and replace the configuration, so
//! this must not be a port any local process of any user can connect to.
//!
//! Over **TCP** only when an address is given (`127.0.0.1:57651`,
//! `http://host:port`): for tests across machines. Nothing checks who
//! connects there.
//!
//! An address is written `local` (this instance, see
//! `gevrek_model::paths::instance`), `local:<instance>`, or as a TCP
//! address.

use std::path::PathBuf;

/// The address that means "the local endpoint of this instance".
pub const LOCAL: &str = "local";

/// One way to reach a window manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// The local endpoint of the named instance.
    Local(String),
    /// A TCP address, as a URL (`http://127.0.0.1:57651`).
    Tcp(String),
}

impl Endpoint {
    /// Read an address as programs take it on `--address`.
    pub fn parse(text: &str) -> Self {
        let text = text.trim();
        if text.is_empty() || text == LOCAL {
            return Self::Local(gevrek_model::paths::instance());
        }
        if let Some(instance) = text.strip_prefix("local:") {
            return Self::Local(instance.to_string());
        }
        if text.starts_with("http://") || text.starts_with("https://") {
            Self::Tcp(text.to_string())
        } else {
            Self::Tcp(format!("http://{text}"))
        }
    }

    /// The address as text that `parse` reads back.
    pub fn address(&self) -> String {
        match self {
            Self::Local(instance) => format!("local:{instance}"),
            Self::Tcp(url) => url.clone(),
        }
    }

    /// For a message: what is listened on or connected to.
    pub fn describe(&self) -> String {
        match self {
            #[cfg(unix)]
            Self::Local(instance) => socket_path(instance).display().to_string(),
            #[cfg(windows)]
            Self::Local(instance) => pipe_name(instance),
            Self::Tcp(url) => url.clone(),
        }
    }
}

/// The Unix socket of an instance.
pub fn socket_path(instance: &str) -> PathBuf {
    gevrek_model::paths::runtime_dir().join(format!("{instance}.sock"))
}

/// The named pipe of an instance. The user's name is in it only so that
/// two users' pipes do not collide; who may open it is decided by the
/// pipe's access list, not by its name.
pub fn pipe_name(instance: &str) -> String {
    let user: String = std::env::var("USERNAME")
        .unwrap_or_default()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .collect();
    format!(r"\\.\pipe\gevrek-{user}-{instance}")
}

// --- client side -----------------------------------------------------------

/// Open a channel to the local endpoint of `instance`.
pub(crate) async fn connect_local(instance: &str) -> anyhow::Result<tonic::transport::Channel> {
    // The URI is never resolved: the connector below ignores it.
    let endpoint = tonic::transport::Endpoint::from_static("http://gevrek.local")
        .connect_timeout(std::time::Duration::from_secs(2));
    #[cfg(unix)]
    {
        let path = socket_path(instance);
        let channel = endpoint
            .connect_with_connector(tower::service_fn(move |_: tonic::transport::Uri| {
                let path = path.clone();
                async move {
                    let stream = tokio::net::UnixStream::connect(path).await?;
                    Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
                }
            }))
            .await?;
        Ok(channel)
    }
    #[cfg(windows)]
    {
        let name = pipe_name(instance);
        let channel = endpoint
            .connect_with_connector(tower::service_fn(move |_: tonic::transport::Uri| {
                let name = name.clone();
                async move {
                    // All instances of the pipe may be taken for a moment
                    // (the server makes the next one after each connection).
                    const PIPE_BUSY: i32 = 231;
                    let mut tries = 0;
                    let pipe = loop {
                        match tokio::net::windows::named_pipe::ClientOptions::new().open(&name) {
                            Ok(pipe) => break pipe,
                            Err(error) if error.raw_os_error() == Some(PIPE_BUSY) && tries < 40 => {
                                tries += 1;
                                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                            }
                            Err(error) => return Err(error),
                        }
                    };
                    Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(pipe))
                }
            }))
            .await?;
        Ok(channel)
    }
}

// --- server side -----------------------------------------------------------

/// The connections arriving at a local endpoint, for
/// `tonic::transport::Server::serve_with_incoming_shutdown`.
#[cfg(unix)]
pub type LocalIncoming = tokio_stream::wrappers::UnixListenerStream;
#[cfg(windows)]
pub type LocalIncoming = tokio_stream::wrappers::ReceiverStream<std::io::Result<PipeConnection>>;

/// Start listening on the local endpoint of `instance`. Fails when another
/// window manager already answers there.
#[cfg(unix)]
pub fn bind_local(instance: &str) -> std::io::Result<LocalIncoming> {
    use std::os::unix::fs::PermissionsExt;
    let path = socket_path(instance);
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
        // Only the user enters the folder (inside XDG_RUNTIME_DIR it is so
        // already; elsewhere this makes it so).
        let _ = std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700));
    }
    if path.exists() {
        if std::os::unix::net::UnixStream::connect(&path).is_ok() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                format!("another gevrek already answers on {}", path.display()),
            ));
        }
        // Left by a window manager that did not end cleanly.
        std::fs::remove_file(&path)?;
    }
    let listener = tokio::net::UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok(tokio_stream::wrappers::UnixListenerStream::new(listener))
}

/// Remove what `bind_local` made, when the server ends.
pub fn release_local(instance: &str) {
    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(socket_path(instance));
    }
    #[cfg(windows)]
    {
        let _ = instance;
    }
}

#[cfg(windows)]
pub fn bind_local(instance: &str) -> std::io::Result<LocalIncoming> {
    let name = pipe_name(instance);
    // The first instance: fails when another process owns the name. The
    // default access list of a pipe lets only its creator, administrators
    // and the system write to it; remote clients are refused.
    let rights = pipe_rights();
    let mut server = make_pipe(&name, true, rights).map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("another gevrek already answers on {name} ({error})"),
            )
        })?;
    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    tokio::spawn(async move {
        loop {
            if let Err(error) = server.connect().await {
                if sender.send(Err(error)).await.is_err() {
                    return;
                }
                continue;
            }
            // The next instance exists before this one is handed over, so
            // a client never finds the name gone.
            let next = match make_pipe(&name, false, rights) {
                Ok(next) => next,
                Err(error) => {
                    let _ = sender.send(Err(error)).await;
                    return;
                }
            };
            let connected = std::mem::replace(&mut server, next);
            if sender.send(Ok(PipeConnection(connected))).await.is_err() {
                return;
            }
        }
    });
    Ok(tokio_stream::wrappers::ReceiverStream::new(receiver))
}

/// Who may open the pipe, as Windows keeps it: the person the window
/// manager runs for, and the system; nobody else. (Left to the default,
/// it depended on how the window manager was started: one that runs as
/// an administrator had a pipe its own clients, which do not, could not
/// write to.) The label says that a program without an administrator's
/// rights may write to it, which is what the pipe is for. Made once and
/// kept for as long as the program lives; 0 where it could not be made
/// (the default then).
#[cfg(windows)]
fn pipe_rights() -> usize {
    use windows::core::{HSTRING, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows::Win32::Security::{
        GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
        TOKEN_USER,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    static RIGHTS: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *RIGHTS.get_or_init(|| {
        let made = (|| -> windows::core::Result<usize> {
            // SAFETY: the token is closed below; the buffer is as large
            // as Windows says the answer is; the text Windows makes is
            // freed once copied; the descriptor and its frame are kept
            // for good (they are used for every pipe that is made).
            unsafe {
                let mut token = HANDLE::default();
                OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
                let mut needed = 0u32;
                let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
                let mut buffer = vec![0u8; needed as usize];
                let asked = GetTokenInformation(
                    token,
                    TokenUser,
                    Some(buffer.as_mut_ptr().cast()),
                    needed,
                    &mut needed,
                );
                let _ = CloseHandle(token);
                asked?;
                let user = &*(buffer.as_ptr() as *const TOKEN_USER);
                let mut text = PWSTR::null();
                ConvertSidToStringSidW(user.User.Sid, &mut text)?;
                let person = text.to_string().unwrap_or_default();
                let _ = LocalFree(HLOCAL(text.0.cast()));
                let words = HSTRING::from(format!(
                    "D:P(A;;GA;;;SY)(A;;GA;;;{person})S:(ML;;NW;;;ME)"
                ));
                let mut descriptor = PSECURITY_DESCRIPTOR::default();
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    &words,
                    SDDL_REVISION_1,
                    &mut descriptor,
                    None,
                )?;
                let frame = Box::leak(Box::new(SECURITY_ATTRIBUTES {
                    nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: descriptor.0,
                    bInheritHandle: false.into(),
                }));
                Ok(frame as *mut SECURITY_ATTRIBUTES as usize)
            }
        })();
        made.unwrap_or_else(|error| {
            log::warn!("The pipe has Windows' usual rights: its own could not be made ({error})");
            0
        })
    })
}

/// Make one instance of the pipe, with its rights (`pipe_rights`).
#[cfg(windows)]
fn make_pipe(
    name: &str,
    first: bool,
    rights: usize,
) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    use tokio::net::windows::named_pipe::ServerOptions;
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(first)
        .reject_remote_clients(true);
    if rights == 0 {
        return options.create(name);
    }
    // SAFETY: `rights` is the address of a frame that is never freed.
    unsafe { options.create_with_security_attributes_raw(name, rights as *mut std::ffi::c_void) }
}

/// One client on the named pipe.
#[cfg(windows)]
pub struct PipeConnection(tokio::net::windows::named_pipe::NamedPipeServer);

#[cfg(windows)]
impl tonic::transport::server::Connected for PipeConnection {
    type ConnectInfo = ();
    fn connect_info(&self) -> Self::ConnectInfo {}
}

#[cfg(windows)]
impl tokio::io::AsyncRead for PipeConnection {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_read(context, buffer)
    }
}

#[cfg(windows)]
impl tokio::io::AsyncWrite for PipeConnection {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.0).poll_write(context, buffer)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_flush(context)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_shutdown(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_read_as_local_or_tcp() {
        assert!(matches!(Endpoint::parse("local"), Endpoint::Local(_)));
        assert!(matches!(Endpoint::parse(""), Endpoint::Local(_)));
        assert_eq!(
            Endpoint::parse("local:dev"),
            Endpoint::Local("dev".to_string())
        );
        assert_eq!(
            Endpoint::parse("127.0.0.1:57651"),
            Endpoint::Tcp("http://127.0.0.1:57651".to_string())
        );
        assert_eq!(
            Endpoint::parse("http://host:1"),
            Endpoint::Tcp("http://host:1".to_string())
        );
        let local = Endpoint::Local("dev".to_string());
        assert_eq!(Endpoint::parse(&local.address()), local);
    }
}
