//! Helper functions abstracting away different transport protocols for Transit

use super::{ConnectionType, TransitConnection, TransitHandshakeError, TransitInfo};
#[cfg(not(target_family = "wasm"))]
use super::{DirectHint, StunError};

#[cfg(not(target_family = "wasm"))]
use async_std::net::TcpStream;
use async_trait::async_trait;
use futures::{
    future::TryFutureExt,
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
};
#[cfg(not(target_family = "wasm"))]
use std::{
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    sync::Arc,
};

#[async_trait]
pub(super) trait TransitTransportRx: AsyncRead + std::any::Any + Unpin + Send {
    /// Helper method for handshake: read a fixed number of bytes and make sure they are as expected
    async fn read_expect(&mut self, expected: &[u8]) -> Result<(), TransitHandshakeError> {
        let mut buffer = vec![0u8; expected.len()];
        self.read_exact(&mut buffer).await?;
        ensure!(buffer == expected, TransitHandshakeError::HandshakeFailed);
        Ok(())
    }

    /// Helper method: read a four bytes length prefix then the appropriate number of bytes
    async fn read_transit_message(&mut self) -> Result<Vec<u8>, std::io::Error> {
        // 1. read 4 bytes from the stream. This represents the length of the encrypted packet.
        let length = {
            let mut length_arr: [u8; 4] = [0; 4];
            self.read_exact(&mut length_arr[..]).await?;
            u32::from_be_bytes(length_arr) as usize
        };

        // 2. read that many bytes into an array (or a vector?)
        let mut buffer = Vec::with_capacity(length);
        let len = self.take(length as u64).read_to_end(&mut buffer).await?;
        use std::io::{Error, ErrorKind};
        ensure!(
            len == length,
            Error::new(ErrorKind::UnexpectedEof, "failed to read whole message")
        );
        Ok(buffer)
    }
}

#[async_trait]
pub(super) trait TransitTransportTx: AsyncWrite + std::any::Any + Unpin + Send {
    /// Helper method: write the message length then the message
    async fn write_transit_message(&mut self, message: &[u8]) -> Result<(), std::io::Error> {
        // send the encrypted record
        self.write_all(&(message.len() as u32).to_be_bytes())
            .await?;
        self.write_all(message).await
    }
}

/// Trait abstracting our socket used for communicating over the wire.
///
/// Will be primarily instantiated by either a TCP or web socket. Custom methods
/// will be added in the future.
pub(super) trait TransitTransport: TransitTransportRx + TransitTransportTx {}

impl<T> TransitTransportRx for T where T: AsyncRead + std::any::Any + Unpin + Send {}
impl<T> TransitTransportTx for T where T: AsyncWrite + std::any::Any + Unpin + Send {}
impl<T> TransitTransport for T where T: AsyncRead + AsyncWrite + std::any::Any + Unpin + Send {}

#[cfg(not(target_family = "wasm"))]
pub(super) fn set_socket_opts(socket: &socket2::Socket) -> std::io::Result<()> {
    socket.set_nonblocking(true)?;

    /* See https://stackoverflow.com/a/14388707/6094756.
     * On most BSD and Linux systems, we need both REUSEADDR and REUSEPORT;
     * and if they don't support the latter we won't compile.
     * On Windows, there is only REUSEADDR but it does what we want.
     */
    socket.set_reuse_address(true)?;
    #[cfg(all(unix, not(any(target_os = "solaris", target_os = "illumos"))))]
    {
        socket.set_reuse_port(true)?;
    }
    #[cfg(not(any(
        all(unix, not(any(target_os = "solaris", target_os = "illumos"))),
        target_os = "windows"
    )))]
    {
        compile_error!("Your system is not supported yet, please raise an error");
    }

    Ok(())
}

/** Perform a STUN query to get the external IP address */
#[cfg(not(target_family = "wasm"))]
pub(super) async fn tcp_get_external_ip() -> Result<(SocketAddr, TcpStream), StunError> {
    let mut socket = tcp_connect_custom(
        &"[::]:0".parse::<SocketAddr>().unwrap().into(),
        &super::PUBLIC_STUN_SERVER
            .to_socket_addrs()?
            /* If you find yourself behind a NAT66, open an issue */
            .find(|x| x.is_ipv4())
            /* TODO add a helper method to stdlib for this */
            .map(|addr| match addr {
                SocketAddr::V4(v4) => {
                    SocketAddr::new(IpAddr::V6(v4.ip().to_ipv6_mapped()), v4.port())
                },
                SocketAddr::V6(_) => unreachable!(),
            })
            .ok_or(StunError::ServerIsV6Only)?
            .into(),
    )
    .await?;

    use bytecodec::{DecodeExt, EncodeExt};
    use stun_codec::{
        rfc5389::{
            self,
            attributes::{MappedAddress, Software, XorMappedAddress},
            Attribute,
        },
        Message, MessageClass, MessageDecoder, MessageEncoder, TransactionId,
    };

    fn get_binding_request() -> Result<Vec<u8>, bytecodec::Error> {
        use rand::Rng;
        let random_bytes = rand::thread_rng().gen::<[u8; 12]>();

        let mut message: Message<Attribute> = Message::new(
            MessageClass::Request,
            rfc5389::methods::BINDING,
            TransactionId::new(random_bytes),
        );

        message.add_attribute(Attribute::Software(Software::new(
            "magic-wormhole-rust".to_owned(),
        )?));

        // Encodes the message
        let mut encoder = MessageEncoder::new();
        let bytes = encoder.encode_into_bytes(message.clone())?;
        Ok(bytes)
    }

    fn decode_address(buf: &[u8]) -> Result<Option<SocketAddr>, bytecodec::Error> {
        let mut decoder = MessageDecoder::<Attribute>::new();
        let decoded = decoder.decode_from_bytes(buf)??;

        let external_addr1 = decoded
            .get_attribute::<XorMappedAddress>()
            .map(|x| x.address());
        //let external_addr2 = decoded.get_attribute::<XorMappedAddress2>().map(|x|x.address());
        let external_addr3 = decoded
            .get_attribute::<MappedAddress>()
            .map(|x| x.address());
        let external_addr = external_addr1
            // .or(external_addr2)
            .or(external_addr3);

        Ok(external_addr)
    }

    /* Connect the plugs */

    socket.write_all(get_binding_request()?.as_ref()).await?;

    let mut buf = [0u8; 256];
    /* Read header first */
    socket.read_exact(&mut buf[..20]).await?;
    let len: u16 = u16::from_be_bytes([buf[2], buf[3]]);
    /* Read the rest of the message */
    socket.read_exact(&mut buf[20..][..len as usize]).await?;
    let external_addr =
        decode_address(&buf[..20 + len as usize])?.ok_or(StunError::ServerNoResponse)?;

    Ok((external_addr, socket))
}

/**
 * Bind to a port with SO_REUSEADDR, connect to the destination and then hide the blood behind a pretty [`async_std::net::TcpStream`]
 *
 * We want an `async_std::net::TcpStream`, but with SO_REUSEADDR set.
 * The former is just a wrapper around `async_io::Async<std::net::TcpStream>`, of which we
 * copy the `connect` method to add a statement that will set the socket flag.
 * See https://github.com/smol-rs/async-net/issues/20.
 */
#[cfg(not(target_family = "wasm"))]
async fn tcp_connect_custom(
    local_addr: &socket2::SockAddr,
    dest_addr: &socket2::SockAddr,
) -> std::io::Result<async_std::net::TcpStream> {
    log::debug!("Binding to {}", local_addr.as_socket().unwrap());
    let socket = socket2::Socket::new(socket2::Domain::IPV6, socket2::Type::STREAM, None)?;
    /* Set our custum options */
    set_socket_opts(&socket)?;

    socket.bind(local_addr)?;

    /* Initiate connect */
    match socket.connect(dest_addr) {
        Ok(_) => {},
        #[cfg(unix)]
        Err(err) if err.raw_os_error() == Some(libc::EINPROGRESS) => {},
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {},
        Err(err) => return Err(err),
    }

    let stream = async_io::Async::new(std::net::TcpStream::from(socket))?;
    /* The stream becomes writable when connected. */
    stream.writable().await?;

    /* Check if there was an error while connecting. */
    stream
        .get_ref()
        .take_error()
        .and_then(|maybe_err| maybe_err.map_or(Ok(()), Result::Err))?;
    /* Convert our mess to `async_std::net::TcpStream */
    Ok(stream.into_inner()?.into())
}

#[cfg(not(target_family = "wasm"))]
pub(super) async fn connect_tcp_direct(
    local_addr: Option<Arc<socket2::SockAddr>>,
    hint: DirectHint,
) -> Result<TransitConnection, TransitHandshakeError> {
    let dest_addr = SocketAddr::try_from(&hint)?;
    log::debug!("Connecting directly to {}", dest_addr);
    let socket;

    if let Some(local_addr) = local_addr {
        socket = tcp_connect_custom(&local_addr, &dest_addr.into()).await?;
        log::debug!("Connected to {}!", dest_addr);
    } else {
        socket = async_std::net::TcpStream::connect(&dest_addr).await?;
        log::debug!("Connected to {}!", dest_addr);
    }

    wrap_tcp_connection(socket, ConnectionType::Direct)
}

/* Take a relay hint and try to connect to it */
#[cfg(not(target_family = "wasm"))]
pub(super) async fn connect_tcp_relay(
    host: DirectHint,
    name: Option<String>,
) -> Result<TransitConnection, TransitHandshakeError> {
    log::debug!("Connecting to relay {}", host);
    let socket = TcpStream::connect((host.hostname.as_str(), host.port))
        .err_into::<TransitHandshakeError>()
        .await?;
    log::debug!("Connected to {}!", host);

    wrap_tcp_connection(socket, ConnectionType::Relay { name })
}

#[cfg(target_family = "wasm")]
pub(super) async fn connect_ws_relay(
    url: url::Url,
    name: Option<String>,
) -> Result<TransitConnection, TransitHandshakeError> {
    log::debug!("Connecting to relay {}", url);
    let (_meta, transit) = ws_stream_wasm::WsMeta::connect(&url, None)
        .err_into::<TransitHandshakeError>()
        .await?;
    log::debug!("Connected to {}!", url);

    let transit = Box::new(transit.into_io()) as Box<dyn TransitTransport>;

    Ok((
        transit,
        TransitInfo {
            conn_type: ConnectionType::Relay { name },
            _unused: (),
        },
    ))
}

/* Take a tcp connection and transform it into a `TransitConnection` (mainly set timeouts) */
#[cfg(not(target_family = "wasm"))]
pub(super) fn wrap_tcp_connection(
    socket: TcpStream,
    conn_type: ConnectionType,
) -> Result<TransitConnection, TransitHandshakeError> {
    /* Set proper read and write timeouts. This will temporarily set the socket into blocking mode :/ */
    // https://github.com/async-rs/async-std/issues/499
    let socket = std::net::TcpStream::try_from(socket)
        .expect("Internal error: this should not fail because we never cloned the socket");
    socket.set_write_timeout(Some(std::time::Duration::from_secs(120)))?;
    socket.set_read_timeout(Some(std::time::Duration::from_secs(120)))?;
    let socket: TcpStream = socket.into();

    let info = TransitInfo {
        conn_type,
        peer_addr: socket
            .peer_addr()
            .expect("Internal error: socket must be IP"),
        _unused: (),
    };

    Ok((Box::new(socket), info))
}
