use std::{
    ffi::{OsStr, OsString},
    fmt::Display,
    io::{self, ErrorKind},
    net::{IpAddr, SocketAddr as TcpSocketAddr, ToSocketAddrs},
    path::PathBuf,
};

// These are only used by Unix Domain socket code
#[cfg(unix)]
use std::path::Path;

#[cfg(unix)]
use mio::net::SocketAddr as UnixSocketAddr;

use lazy_regex::{regex_captures, regex_is_match};

/// A user can pass a listen- or connect address as a host/port combination, a
/// path or a lone port number.
#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub enum MonetAddr {
    /// A host name that must still be resolved to IP addresses, plus a port
    Dns { host: String, port: u16 },
    /// A specific IP address, plus a port
    Ip { ip: IpAddr, port: u16 },
    /// The path of a Unix Domain socket in the file system
    Unix(PathBuf),
    /// Only a port, may resolve to a combination of ip/port pairs and a Unix
    /// Domain socket.
    PortOnly(u16),
}

impl Display for MonetAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // MonetAddr::Tcp { host, port } => write!(f, "{host}:{port}"),
            MonetAddr::Dns { host, port } => write!(f, "{host}:{port}"),
            MonetAddr::Ip {
                ip: IpAddr::V4(ip4),
                port,
            } => write!(f, "{ip4}:{port}"),
            MonetAddr::Ip {
                ip: IpAddr::V6(ip6),
                port,
            } => write!(f, "[{ip6}]:{port}"),
            MonetAddr::Unix(path) => path.display().fmt(f),
            MonetAddr::PortOnly(n) => n.fmt(f),
        }
    }
}

impl TryFrom<&OsStr> for MonetAddr {
    type Error = io::Error;

    fn try_from(os_value: &OsStr) -> Result<Self, io::Error> {
        // this nested function does all the work but it returns Option rather
        // than Result.
        fn parse(os_value: &OsStr) -> Option<MonetAddr> {
            // If it contains slashes or backslashes, it must be a path
            let bytes = os_value.as_encoded_bytes();
            if bytes.contains(&b'/') || bytes.contains(&b'\\') {
                return Some(MonetAddr::Unix(os_value.into()));
            }

            // The other possibilities are all proper str's
            let str_value = os_value.to_str()?;

            // If it's a number, it must be the port number.
            if let Ok(port) = str_value.parse() {
                return Some(MonetAddr::PortOnly(port));
            }

            // it must end in :PORTNUMBER
            let (_, host_part, port_part) = regex_captures!(r"^(.+):(\d+)$", str_value)?;
            let port: u16 = port_part.parse().ok()?;

            // is the host IPv4, IPv6 or DNS?
            if regex_is_match!(r"^\d+.\d+.\d+.\d+$", host_part) {
                // IPv4
                Some(MonetAddr::Ip {
                    ip: IpAddr::V4(host_part.parse().ok()?),
                    port,
                })
            } else if let Some((_, ip)) = regex_captures!(r"^\[([0-9a-f:]+)\]$"i, host_part) {
                // IPv6
                Some(MonetAddr::Ip {
                    ip: IpAddr::V6(ip.parse().ok()?),
                    port,
                })
            } else if regex_is_match!(r"^[a-z0-9][-a-z0-9.]*$"i, host_part) {
                // names consisting of letters, digits and hyphens, separated or terminated by periods
                Some(MonetAddr::Dns {
                    host: host_part.to_string(),
                    port,
                })
            } else {
                None
            }
        }

        // now apply the nested function and handle errors
        if let Some(monetaddr) = parse(os_value) {
            Ok(monetaddr)
        } else {
            Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("invalid address: {}", os_value.to_string_lossy()),
            ))
        }
    }
}

impl TryFrom<OsString> for MonetAddr {
    type Error = io::Error;

    fn try_from(value: OsString) -> Result<Self, Self::Error> {
        Self::try_from(value.as_os_str())
    }
}

impl MonetAddr {
    /// Resolve the user-specified address into a Vec of concrete addresses.
    ///
    /// For example, a lone port number might resolve to a Unix Domain socket
    /// and a number of ip/port pairs.
    pub fn resolve(&self) -> io::Result<Vec<Addr>> {
        let mut addrs = self.resolve_tcp()?;
        if let Some(unix_addr) = self.resolve_unix()? {
            // Prepend
            addrs.insert(0, unix_addr);
        }
        Ok(addrs)
    }

    /// Resolve the user-specified address into TCP-based [`Addr`]'s
    pub fn resolve_tcp(&self) -> io::Result<Vec<Addr>> {
        fn gather<T: ToSocketAddrs>(a: T) -> io::Result<Vec<Addr>> {
            Ok(a.to_socket_addrs()?.map(Addr::Tcp).collect())
        }

        match self {
            MonetAddr::Unix(_) => Ok(vec![]),
            MonetAddr::Dns { host, port } => gather((host.as_str(), *port)),
            MonetAddr::Ip { ip, port } => gather((*ip, *port)),
            MonetAddr::PortOnly(port) => gather(("localhost", *port)),
        }
    }

    /// Resolve the user-specified address into a Unix Domain TCP-based [`Addr`]
    pub fn resolve_unix(&self) -> io::Result<Option<Addr>> {
        if cfg!(unix) {
            let path = match self {
                MonetAddr::Dns { .. } | MonetAddr::Ip { .. } => return Ok(None),
                MonetAddr::Unix(p) => p.clone(),
                MonetAddr::PortOnly(port) => PathBuf::from(format!("/tmp/.s.monetdb.{port}")),
            };
            Ok(Some(Addr::Unix(path)))
        } else {
            Ok(None)
        }
    }
}

/// A generalization of [std::net::SocketAddr] that also allows Unix Domain
/// sockets.
#[derive(Debug, Clone)]
pub enum Addr {
    Tcp(TcpSocketAddr),
    Unix(PathBuf),
}

impl Display for Addr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Addr::Tcp(a) => a.fmt(f),
            Addr::Unix(path) => path.display().fmt(f),
        }
    }
}

impl Addr {
    pub fn is_tcp(&self) -> bool {
        matches!(self, Self::Tcp(_))
    }

    pub fn is_unix(&self) -> bool {
        !self.is_tcp()
    }
}

impl From<TcpSocketAddr> for Addr {
    fn from(value: TcpSocketAddr) -> Self {
        Addr::Tcp(value)
    }
}

impl From<PathBuf> for Addr {
    fn from(value: PathBuf) -> Self {
        Addr::Unix(value)
    }
}

#[cfg(unix)]
impl From<UnixSocketAddr> for Addr {
    fn from(value: UnixSocketAddr) -> Self {
        value
            .as_pathname()
            .unwrap_or(Path::new("<UNNAMED>"))
            .to_path_buf()
            .into()
    }
}
