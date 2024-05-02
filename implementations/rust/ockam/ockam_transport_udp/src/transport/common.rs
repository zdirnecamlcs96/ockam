use crate::workers::Addresses;
use core::fmt;
use core::fmt::Formatter;
use ockam_core::compat::net::{SocketAddr, ToSocketAddrs};
use ockam_core::flow_control::FlowControlId;
use ockam_core::{Address, Result};
use ockam_transport_core::TransportError;

/// Result of [`TcpTransport::listen`] call.
#[derive(Clone, Debug)]
pub struct UdpBind {
    addresses: Addresses,
    peer: Option<SocketAddr>,
    bind_address: SocketAddr,
    flow_control_id: FlowControlId,
}

impl fmt::Display for UdpBind {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Peer: {:?}, Bind: {}, Receiver: {}, Sender: {}, FlowId: {}",
            self.peer,
            self.bind_address,
            self.addresses.receiver_address(),
            self.addresses.sender_address(),
            self.flow_control_id
        )
    }
}

impl UdpBind {
    /// Constructor
    pub(crate) fn new(
        addresses: Addresses,
        peer: Option<SocketAddr>,
        bind_address: SocketAddr,
        flow_control_id: FlowControlId,
    ) -> Self {
        Self {
            addresses,
            peer,
            bind_address,
            flow_control_id,
        }
    }
    pub fn receiver_address(&self) -> &Address {
        self.addresses.receiver_address()
    }
    pub fn sender_address(&self) -> &Address {
        self.addresses.sender_address()
    }
    pub fn peer(&self) -> Option<SocketAddr> {
        self.peer
    }
    pub fn bind_address(&self) -> SocketAddr {
        self.bind_address
    }
    pub fn flow_control_id(&self) -> &FlowControlId {
        &self.flow_control_id
    }
}

impl From<UdpBind> for Address {
    fn from(value: UdpBind) -> Self {
        value.addresses.sender_address().clone()
    }
}

/// Resolve the given peer to a [`SocketAddr`](std::net::SocketAddr)
pub fn resolve_peer(peer: String) -> Result<SocketAddr> {
    // Try to parse as SocketAddr
    if let Ok(p) = parse_socket_addr(&peer) {
        return Ok(p);
    }

    // Try to resolve hostname
    if let Ok(mut iter) = peer.to_socket_addrs() {
        // Prefer ip4
        if let Some(p) = iter.find(|x| x.is_ipv4()) {
            return Ok(p);
        }
        if let Some(p) = iter.find(|x| x.is_ipv6()) {
            return Ok(p);
        }
    }

    // Nothing worked, return an error
    Err(TransportError::InvalidAddress)?
}

pub(super) fn parse_socket_addr(s: &str) -> Result<SocketAddr> {
    Ok(s.parse().map_err(|_| TransportError::InvalidAddress)?)
}
