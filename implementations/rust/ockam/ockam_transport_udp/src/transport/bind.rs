use crate::transport::common::{parse_socket_addr, resolve_peer, UdpBind};
use crate::workers::{Addresses, TransportMessageCodec, UdpReceiverProcessor, UdpSenderWorker};
use crate::{UdpBindOptions, UdpTransport};
use futures_util::StreamExt;
use ockam_core::errcode::{Kind, Origin};
use ockam_core::{Address, AllowAll, DenyAll, Error, Result};
use ockam_node::{ProcessorBuilder, WorkerBuilder};
use ockam_transport_core::TransportError;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tokio::net::UdpSocket;
use tokio_util::udp::UdpFramed;
use tracing::{debug, error};

/// FIXME
pub struct UdpBindArguments {
    peer_address: Option<SocketAddr>,
    bind_address: SocketAddr,
}

impl Default for UdpBindArguments {
    fn default() -> Self {
        Self {
            peer_address: None,
            bind_address: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
        }
    }
}

impl UdpBindArguments {
    /// FIXME
    pub fn new() -> Self {
        Self::default()
    }

    /// FIXME
    pub fn with_bind_address(mut self, bind_address: impl AsRef<str>) -> Result<Self> {
        let bind_address = parse_socket_addr(bind_address.as_ref())?;
        self.bind_address = bind_address;

        Ok(self)
    }

    /// FIXME
    pub fn with_peer_address(mut self, peer_address: impl AsRef<str>) -> Result<Self> {
        let peer_address = resolve_peer(peer_address.as_ref().to_string())?;
        self.peer_address = Some(peer_address);

        Ok(self)
    }
}

impl UdpTransport {
    /// FIXME
    pub async fn bind(
        &self,
        arguments: UdpBindArguments,
        options: UdpBindOptions,
    ) -> Result<UdpBind> {
        // This transport only supports IPv4
        if !arguments.bind_address.is_ipv4() {
            error!(local_addr = %arguments.bind_address, "This transport only supports IPv4");
            return Err(TransportError::InvalidAddress)?;
        }

        // Bind new socket
        let socket = UdpSocket::bind(arguments.bind_address)
            .await
            .map_err(|_| TransportError::BindFailed)?;

        if let Some(_peer) = &arguments.peer_address {
            // FIXME: Doesn't work
            // socket.connect(peer).await.unwrap();
        }

        let local_addr = socket
            .local_addr()
            .map_err(|_| Error::new(Origin::Transport, Kind::Io, "invalid local address"))?;

        // Split socket into sink and stream
        let (sink, stream) = UdpFramed::new(socket, TransportMessageCodec).split();

        let addresses = Addresses::generate();

        debug!("Creating UDP sender and receiver. Peer: {:?}, Local address: {}, Sender: {}, Receiver: {}",
            arguments.peer_address,
            local_addr,
            addresses.sender_address(),
            addresses.receiver_address());

        options.setup_flow_control(self.ctx.flow_controls(), &addresses);
        let flow_control_id = options.flow_control_id.clone();
        let receiver_outgoing_access_control =
            options.create_receiver_outgoing_access_control(self.ctx.flow_controls());

        let sender = UdpSenderWorker::new(addresses.clone(), sink, arguments.peer_address);
        WorkerBuilder::new(sender)
            .with_address(addresses.sender_address().clone())
            .with_incoming_access_control(AllowAll)
            .with_outgoing_access_control(DenyAll)
            .start(&self.ctx)
            .await?;

        let receiver = UdpReceiverProcessor::new(addresses.clone(), stream, arguments.peer_address);
        ProcessorBuilder::new(receiver)
            .with_address(addresses.receiver_address().clone())
            .with_incoming_access_control(DenyAll)
            .with_outgoing_access_control_arc(receiver_outgoing_access_control)
            .start(&self.ctx)
            .await?;

        let bind = UdpBind::new(
            addresses,
            arguments.peer_address,
            local_addr,
            flow_control_id,
        );

        Ok(bind)
    }

    /// Interrupt an active TCP connection given its Sender `Address`
    pub async fn unbind(&self, address: impl Into<Address>) -> Result<()> {
        self.ctx.stop_worker(address.into()).await
    }
}
