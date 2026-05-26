use std::net::SocketAddr;

use socket2::{Domain, Socket, Type};
use tokio::net::TcpListener;

use super::ServerError;

pub async fn bind_reuseport(addr: SocketAddr) -> Result<TcpListener, ServerError> {
    let domain = match addr {
        SocketAddr::V4(_) => Domain::IPV4,
        SocketAddr::V6(_) => Domain::IPV6,
    };
    let socket = Socket::new(domain, Type::STREAM, None)?;
    socket.set_reuse_address(true)?;
    #[cfg(all(unix, not(target_os = "solaris")))]
    socket.set_reuse_port(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(1024)?;
    TcpListener::from_std(socket.into()).map_err(ServerError::Io)
}
