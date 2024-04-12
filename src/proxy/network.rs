use std::io::{self, ErrorKind};
use std::{fs, net};

use mio::net::{TcpListener, TcpStream};
#[cfg(unix)]
use mio::net::{UnixListener, UnixStream};

use crate::addr::Addr;

#[cfg(not(unix))]
fn unix_not_supported() -> io::Error {
    io::Error::new(
        ErrorKind::Unsupported,
        "Unix Domain sockets are not supported on this system",
    )
}

#[derive(Debug)]
pub enum MioListener {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix(UnixListener),
}

#[derive(Debug)]
pub enum MioStream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
}

impl mio::event::Source for MioListener {
    fn register(
        &mut self,
        registry: &mio::Registry,
        token: mio::Token,
        interests: mio::Interest,
    ) -> io::Result<()> {
        match self {
            Self::Tcp(lis) => lis.register(registry, token, interests),
            #[cfg(unix)]
            Self::Unix(lis) => lis.register(registry, token, interests),
        }
    }

    fn reregister(
        &mut self,
        registry: &mio::Registry,
        token: mio::Token,
        interests: mio::Interest,
    ) -> io::Result<()> {
        match self {
            Self::Tcp(lis) => lis.reregister(registry, token, interests),
            #[cfg(unix)]
            Self::Unix(lis) => lis.reregister(registry, token, interests),
        }
    }

    fn deregister(&mut self, registry: &mio::Registry) -> io::Result<()> {
        match self {
            Self::Tcp(lis) => lis.deregister(registry),
            #[cfg(unix)]
            Self::Unix(lis) => lis.deregister(registry),
        }
    }
}

impl MioListener {
    pub fn new(addr: &Addr) -> io::Result<Self> {
        let listener = match addr {
            Addr::Tcp(a) => MioListener::Tcp(TcpListener::bind(*a)?),
            #[cfg(unix)]
            Addr::Unix(a) => {
                let listener = match UnixListener::bind(a) {
                    Ok(lis) => lis,
                    Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
                        fs::remove_file(a)?;
                        UnixListener::bind(a)?
                    }
                    Err(other) => return Err(other),
                };
                MioListener::Unix(listener)
            }
            #[cfg(not(unix))]
            Addr::Unix(_) => return Err(unix_not_supported()),
        };
        Ok(listener)
    }

    #[allow(dead_code)]
    pub fn is_tcp(&self) -> bool {
        matches!(self, Self::Tcp(_))
    }

    #[allow(dead_code)]
    pub fn is_unix(&self) -> bool {
        !self.is_tcp()
    }

    pub fn accept(&self) -> io::Result<(MioStream, Addr)> {
        match self {
            MioListener::Tcp(lis) => {
                let (conn, peer) = lis.accept()?;
                let stream = MioStream::Tcp(conn);
                let peer = Addr::Tcp(peer);
                Ok((stream, peer))
            }
            #[cfg(unix)]
            MioListener::Unix(lis) => {
                let (conn, peer) = lis.accept()?;
                let stream = MioStream::Unix(conn);
                Ok((stream, peer.into()))
            }
        }
    }
}

impl Drop for MioListener {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let MioListener::Unix(listener) = self {
            let Ok(unix_sock_addr) = listener.local_addr() else {
                return;
            };
            let Some(path) = unix_sock_addr.as_pathname() else {
                return;
            };
            let _ = fs::remove_file(path);
        }
    }
}

impl mio::event::Source for MioStream {
    fn register(
        &mut self,
        registry: &mio::Registry,
        token: mio::Token,
        interests: mio::Interest,
    ) -> io::Result<()> {
        match self {
            Self::Tcp(lis) => lis.register(registry, token, interests),
            #[cfg(unix)]
            Self::Unix(lis) => lis.register(registry, token, interests),
        }
    }

    fn reregister(
        &mut self,
        registry: &mio::Registry,
        token: mio::Token,
        interests: mio::Interest,
    ) -> io::Result<()> {
        match self {
            Self::Tcp(lis) => lis.reregister(registry, token, interests),
            #[cfg(unix)]
            Self::Unix(lis) => lis.reregister(registry, token, interests),
        }
    }

    fn deregister(&mut self, registry: &mio::Registry) -> io::Result<()> {
        match self {
            Self::Tcp(lis) => lis.deregister(registry),
            #[cfg(unix)]
            Self::Unix(lis) => lis.deregister(registry),
        }
    }
}

impl MioStream {
    pub fn new(addr: &Addr) -> io::Result<Self> {
        let conn = match addr {
            Addr::Tcp(a) => MioStream::Tcp(TcpStream::connect(*a)?),
            #[cfg(unix)]
            Addr::Unix(a) => MioStream::Unix(UnixStream::connect(a)?),
            #[cfg(not(unix))]
            Addr::Unix(_) => return Err(unix_not_supported()),
        };
        Ok(conn)
    }

    pub fn is_tcp(&self) -> bool {
        matches!(self, Self::Tcp(_))
    }

    pub fn is_unix(&self) -> bool {
        !self.is_tcp()
    }

    pub fn established(&self) -> io::Result<Option<Addr>> {
        if let Err(e) | Ok(Some(e)) = self.take_error() {
            return Err(e);
        }

        let peer_result = match self {
            MioStream::Tcp(s) => s.peer_addr().map(Addr::from),
            #[cfg(unix)]
            MioStream::Unix(s) => s.peer_addr().map(Addr::from),
        };

        match peer_result {
            Ok(addr) => Ok(Some(addr)),
            Err(e) => match e.kind() {
                ErrorKind::WouldBlock | ErrorKind::NotConnected => Ok(None),
                _ => Err(e),
            },
        }
    }

    pub fn shutdown(&self, shutdown: net::Shutdown) -> io::Result<()> {
        match self {
            MioStream::Tcp(s) => s.shutdown(shutdown),
            #[cfg(unix)]
            MioStream::Unix(s) => s.shutdown(shutdown),
        }
    }

    pub fn take_error(&self) -> io::Result<Option<io::Error>> {
        match self {
            MioStream::Tcp(s) => s.take_error(),
            #[cfg(unix)]
            MioStream::Unix(s) => s.take_error(),
        }
    }

    #[allow(dead_code)]
    pub fn peer_addr(&self) -> io::Result<Addr> {
        let addr = match self {
            MioStream::Tcp(s) => s.peer_addr()?.into(),
            #[cfg(unix)]
            MioStream::Unix(s) => s.peer_addr()?.into(),
        };
        Ok(addr)
    }

    pub fn set_nodelay(&self, nodelay: bool) -> io::Result<()> {
        match self {
            MioStream::Tcp(s) => s.set_nodelay(nodelay),
            #[cfg(unix)]
            MioStream::Unix(_) => Ok(()),
        }
    }

    #[cfg(feature = "oob")]
    pub fn with_socket2<T, F>(&mut self, f: F) -> io::Result<T>
    where
        F: FnOnce(&mut socket2::Socket) -> io::Result<T>,
    {
        use  std::os::fd::{AsRawFd, FromRawFd};

        let fd = match self {
            MioStream::Tcp(sock) => sock.as_raw_fd(),
            MioStream::Unix(sock) => sock.as_raw_fd(),
        };
        let sock2 = unsafe {
            // SAFETY: it's clear from above that fd is always a socket.
            socket2::Socket::from_raw_fd(fd)
        };
        let mut dont_drop = std::mem::ManuallyDrop::new(sock2);
        f(&mut dont_drop)
    }
}

impl io::Write for MioStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            MioStream::Tcp(s) => s.write(buf),
            #[cfg(unix)]
            MioStream::Unix(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            MioStream::Tcp(s) => s.flush(),
            #[cfg(unix)]
            MioStream::Unix(s) => s.flush(),
        }
    }

    fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
        match self {
            MioStream::Tcp(s) => s.write_vectored(bufs),
            #[cfg(unix)]
            MioStream::Unix(s) => s.write_vectored(bufs),
        }
    }
}

impl io::Read for MioStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            MioStream::Tcp(s) => s.read(buf),
            #[cfg(unix)]
            MioStream::Unix(s) => s.read(buf),
        }
    }

    fn read_vectored(&mut self, bufs: &mut [io::IoSliceMut<'_>]) -> io::Result<usize> {
        match self {
            MioStream::Tcp(s) => s.read_vectored(bufs),
            #[cfg(unix)]
            MioStream::Unix(s) => s.read_vectored(bufs),
        }
    }
}
