//! `std::net`: TCP, UDP and Unix-domain sockets, with a simple text
//! interface. Errors arrive in Xur as the `Err` of a `Result`.
//!
//! ```text
//! let listener = std::net::TcpListener::bind("127.0.0.1:8080").unwrap();
//! let (client, address) = listener.accept().unwrap();
//! client.write("hello\n").unwrap();
//! let line = client.read_line().unwrap();
//! ```

use std::io::{BufRead, BufReader, Read, Write};
use std::net;
use std::sync::{Arc, Mutex};

use facet::Facet;
use xur_runtime::registry::{ModuleBuilder, TypeBuilder};
use xur_runtime::{Registry, lock};

use crate::error_text;

/// Defines a stream type: a socket that is read through a buffer, so
/// whole lines can be read, and written directly.
macro_rules! stream {
    ($name:ident, $socket:ty, $doc:literal) => {
        #[doc = $doc]
        #[derive(Facet, Clone)]
        #[facet(opaque)]
        pub struct $name {
            reader: Arc<Mutex<BufReader<$socket>>>,
            writer: Arc<Mutex<$socket>>,
        }

        impl $name {
            fn new(socket: $socket) -> std::io::Result<Self> {
                let reader = socket.try_clone()?;
                Ok(Self {
                    reader: Arc::new(Mutex::new(BufReader::new(reader))),
                    writer: Arc::new(Mutex::new(socket)),
                })
            }

            /// Registers the methods every stream has.
            fn methods(ty: &mut TypeBuilder<'_, $name>) {
                ty.method("write", |stream: &$name, text: String| {
                    lock(&stream.writer)
                        .write_all(text.as_bytes())
                        .map_err(error_text)
                })
                .method("read_line", |stream: &$name| {
                    let mut line = String::new();
                    lock(&stream.reader)
                        .read_line(&mut line)
                        .map(|_| line)
                        .map_err(error_text)
                })
                .method("read", |stream: &$name, max: u32| {
                    let mut buffer = vec![0; max as usize];
                    let read = lock(&stream.reader).read(&mut buffer).map_err(error_text)?;
                    Ok::<_, String>(String::from_utf8_lossy(&buffer[..read]).into_owned())
                })
                .method("shutdown", |stream: &$name| {
                    lock(&stream.writer)
                        .shutdown(net::Shutdown::Both)
                        .map_err(error_text)
                });
            }
        }
    };
}

stream!(TcpStream, net::TcpStream, "A TCP connection.");

#[cfg(unix)]
stream!(
    UnixStream,
    std::os::unix::net::UnixStream,
    "A connection over a Unix-domain socket."
);

/// A TCP socket accepting connections.
#[derive(Facet, Clone)]
#[facet(opaque)]
pub struct TcpListener(Arc<net::TcpListener>);

/// A UDP socket.
#[derive(Facet, Clone)]
#[facet(opaque)]
pub struct UdpSocket(Arc<net::UdpSocket>);

/// A Unix-domain socket accepting connections.
#[cfg(unix)]
#[derive(Facet, Clone)]
#[facet(opaque)]
pub struct UnixListener(Arc<std::os::unix::net::UnixListener>);

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::net", |module| {
        module
            .ty::<TcpListener>("TcpListener")
            .function("bind", |address: String| {
                net::TcpListener::bind(address)
                    .map(|listener| TcpListener(Arc::new(listener)))
                    .map_err(error_text)
            })
            .method("accept", |listener: &TcpListener| {
                let (socket, address) = listener.0.accept().map_err(error_text)?;
                let stream = TcpStream::new(socket).map_err(error_text)?;
                Ok::<_, String>((stream, address.to_string()))
            })
            .method("local_addr", |listener: &TcpListener| {
                listener
                    .0
                    .local_addr()
                    .map(|a| a.to_string())
                    .map_err(error_text)
            });
        let mut stream = module.ty::<TcpStream>("TcpStream");
        TcpStream::methods(&mut stream);
        stream.function("connect", |address: String| {
            let socket = net::TcpStream::connect(address).map_err(error_text)?;
            TcpStream::new(socket).map_err(error_text)
        });

        module
            .ty::<UdpSocket>("UdpSocket")
            .function("bind", |address: String| {
                net::UdpSocket::bind(address)
                    .map(|socket| UdpSocket(Arc::new(socket)))
                    .map_err(error_text)
            })
            .method(
                "send_to",
                |socket: &UdpSocket, text: String, address: String| {
                    socket
                        .0
                        .send_to(text.as_bytes(), address)
                        .map_err(error_text)
                },
            )
            .method("recv_from", |socket: &UdpSocket, max: u32| {
                let mut buffer = vec![0; max as usize];
                let (read, from) = socket.0.recv_from(&mut buffer).map_err(error_text)?;
                let text = String::from_utf8_lossy(&buffer[..read]).into_owned();
                Ok::<_, String>((text, from.to_string()))
            })
            .method("local_addr", |socket: &UdpSocket| {
                socket
                    .0
                    .local_addr()
                    .map(|a| a.to_string())
                    .map_err(error_text)
            });

        #[cfg(unix)]
        register_unix(module);
    });
}

#[cfg(unix)]
fn register_unix(module: &mut ModuleBuilder<'_>) {
    use std::os::unix::net as unix;

    module
        .ty::<UnixListener>("UnixListener")
        .function("bind", |path: String| {
            unix::UnixListener::bind(path)
                .map(|listener| UnixListener(Arc::new(listener)))
                .map_err(error_text)
        })
        .method("accept", |listener: &UnixListener| {
            let (socket, _) = listener.0.accept().map_err(error_text)?;
            UnixStream::new(socket).map_err(error_text)
        });
    let mut stream = module.ty::<UnixStream>("UnixStream");
    UnixStream::methods(&mut stream);
    stream.function("connect", |path: String| {
        let socket = unix::UnixStream::connect(path).map_err(error_text)?;
        UnixStream::new(socket).map_err(error_text)
    });
}
