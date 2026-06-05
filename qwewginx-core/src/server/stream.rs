use std::net::SocketAddr;

use tokio::net::TcpStream;

use crate::config::Stream;

use super::listen;
use super::ServerError;

pub async fn serve(stream: &Stream) -> Result<u32, ServerError> {
    let mut n = 0u32;
    for server in &stream.servers {
        let listener = listen::bind_reuseport(server.listen).await?;
        let upstream = server.proxy_pass;
        tracing::info!(
            "worker {} stream listening on {} -> {}",
            std::process::id(),
            server.listen,
            upstream
        );
        n += 1;
        tokio::spawn(async move {
            loop {
                let (client, remote_addr) = match listener.accept().await {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::error!("stream accept failed: {e}");
                        continue;
                    }
                };
                tokio::spawn(async move {
                    relay_tcp(client, upstream, remote_addr).await;
                });
            }
        });
    }
    Ok(n)
}

async fn relay_tcp(mut client: TcpStream, upstream: SocketAddr, remote_addr: SocketAddr) {
    let mut server = match TcpStream::connect(upstream).await {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(
                %remote_addr,
                %upstream,
                "stream upstream connect failed: {e}"
            );
            return;
        }
    };
    if let Err(e) = tokio::io::copy_bidirectional(&mut client, &mut server).await {
        tracing::debug!(
            %remote_addr,
            %upstream,
            "stream relay closed: {e}"
        );
    }
}
