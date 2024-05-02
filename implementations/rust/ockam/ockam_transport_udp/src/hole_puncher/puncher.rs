use crate::hole_puncher::{Addresses, UdpHolePuncherOptions};
use crate::transport::common::UdpBind;
use crate::{hole_puncher::worker::UdpHolePunchWorker, PunchError};
use ockam_core::errcode::{Kind, Origin};
use ockam_core::flow_control::FlowControlId;
use ockam_core::Error;
use ockam_core::{route, Address, Result, Route};
use ockam_node::Context;
use tokio::sync::broadcast;

/// FIXME
/// High level management interface for UDP NAT Hole Punchers
///
/// See [Wikipedia](https://en.wikipedia.org/wiki/UDP_hole_punching) and
/// ['Peer-to-Peer Communication Across Network Address Translators'](https://bford.info/pub/net/p2pnat/).
///
/// A node can have multiple Hole Punchers (punchers) at a time, each one with
/// a unique name and working with a different peer puncher in a remote node.
///
/// For a puncher to work a (e.g. from 'alice' to 'bob', using rendezvous service
/// 'zurg') the remote node will also need to create its own puncher
/// (e.g. from 'bob' to 'alice', using 'zurg').
///
/// # Warnings
///
/// This UDP NAT Hole Puncher implementation is __currently a prototype__.
/// No guarantees are provided.
///
/// UDP and NAT Hole Punching are unreliable protocols. Expect send and receive
/// failures.
///
/// # Example
///
/// ```rust
/// # use {ockam_node::Context, ockam_core::{Result, route}};
/// # async fn test(ctx: &mut Context) -> Result<()> {
/// use ockam_transport_udp::{UdpBindArguments, UdpBindOptions, UdpHolePuncher, UdpTransport, UDP, UdpHolePuncherOptions};
///
/// // Create transport
/// let udp = UdpTransport::create(ctx).await?;
/// let bind = udp.bind(UdpBindArguments::new(), UdpBindOptions::new()).await?;
///
/// // Create a NAT hole from us 'alice' to them 'bob' using
/// // the Rendezvous service 'zurg' at public IP address `192.168.1.10:4000`
/// let rendezvous_route = route![bind.sender_address().clone(), (UDP, "192.168.1.10:4000"), "zurg"];
/// let mut puncher = UdpHolePuncher::create(ctx, &bind, "alice", "bob", rendezvous_route, UdpHolePuncherOptions::new()).await?;
///
/// // Note: For this to work, 'bob' will likewise need to create a hole thru to us
///
/// // Wait for hole to open.
/// // Note that the hole could close at anytime. If the hole closes, the
/// // puncher will automatically try to re-open it.
/// puncher.wait_for_hole_open().await?;
///
/// // Try to send a message to a remote 'echoer' via our puncher
/// ctx.send(route![puncher.sender_address(), "echoer"], "Góðan daginn".to_string()).await?;
/// # Ok(())
/// # }
/// ```
///
pub struct UdpHolePuncher {
    notify_hole_open_receiver: broadcast::Receiver<Route>,
    addresses: Addresses,
    flow_control_id: FlowControlId,
}

// TODO: Allow app to specify how often keepalives are used - they may have
// limited bandwidth. Also, allow app to specify other configurations?

impl UdpHolePuncher {
    /// Create a new UDP NAT Hole Puncher
    pub async fn create(
        ctx: &Context,
        bind: &UdpBind,
        puncher_name: impl AsRef<str>,
        peer_puncher_name: impl AsRef<str>,
        rendezvous_route: impl Into<Route>,
        options: UdpHolePuncherOptions,
    ) -> Result<UdpHolePuncher> {
        let rendezvous_route = route![bind.sender_address().clone(), rendezvous_route.into()];

        // Check if we can reach the rendezvous service
        if !UdpHolePunchWorker::rendezvous_reachable(ctx, &rendezvous_route).await? {
            return Err(PunchError::RendezvousServiceNotFound)?;
        }

        let flow_control_id = options.producer_flow_control_id();

        // Create worker
        let (addresses, notify_hole_open_receiver) = UdpHolePunchWorker::create(
            ctx,
            bind,
            rendezvous_route,
            puncher_name.as_ref(),
            peer_puncher_name.as_ref(),
            options,
        )
        .await?;

        Ok(Self {
            notify_hole_open_receiver,
            addresses,
            flow_control_id,
        })
    }

    /// Wait until Hole Puncher successfully opens a hole to the peer or a
    /// timeout
    ///
    /// Note that the hole could close at anytime. If the hole closes, the
    /// puncher will automatically try to re-open it.
    ///
    /// Timeout is the same as that of [`Context::receive()`].
    pub async fn wait_for_hole_open(&mut self) -> Result<()> {
        self.notify_hole_open_receiver.recv().await.map_err(|_| {
            Error::new(
                Origin::Transport,
                Kind::Cancelled,
                "UDP hole won't be opened",
            )
        })?;

        Ok(())
    }

    /// Address of this UDP NAT Hole Puncher's worker.
    pub fn sender_address(&self) -> Address {
        self.addresses.sender_address().clone()
    }

    /// Flow Control Id
    pub fn flow_control_id(&self) -> &FlowControlId {
        &self.flow_control_id
    }
}
