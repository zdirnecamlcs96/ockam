use crate::hole_punching::puncher::worker::UdpHolePunchWorker;
use crate::hole_punching::puncher::{Addresses, UdpHolePuncherOptions};
use crate::transport::common::UdpBind;
use ockam_core::errcode::{Kind, Origin};
use ockam_core::flow_control::FlowControlId;
use ockam_core::Error;
use ockam_core::{Address, Result, Route};
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
pub struct UdpHolePuncher {
    notify_hole_open_receiver: broadcast::Receiver<Route>,
    notify_hole_open_sender: broadcast::Sender<Route>,
    addresses: Addresses,
    flow_control_id: FlowControlId,
}

// TODO: Allow stopping

// TODO: Allow app to specify how often keepalives are used - they may have
// limited bandwidth. Also, allow app to specify other configurations?

impl UdpHolePuncher {
    /// Create a new UDP NAT Hole Puncher
    pub async fn create(
        ctx: &Context,
        bind: &UdpBind,
        peer_udp_address: String,
        my_remote_address: Address,
        their_remote_address: Address,
        options: UdpHolePuncherOptions,
        redirect_first_message_to_transport: bool,
    ) -> Result<UdpHolePuncher> {
        let flow_control_id = options.producer_flow_control_id();

        // Create worker
        let addresses = Addresses::generate(my_remote_address);
        let (notify_hole_open_sender, notify_hole_open_receiver) = broadcast::channel(1);
        UdpHolePunchWorker::create(
            ctx,
            bind,
            peer_udp_address,
            their_remote_address,
            addresses.clone(),
            notify_hole_open_sender.clone(),
            options,
            redirect_first_message_to_transport,
        )
        .await?;

        Ok(Self {
            notify_hole_open_receiver,
            notify_hole_open_sender,
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

    /// Receiver that will receive a message with a Route when puncture is open, of empty route
    /// when closed
    // TODO: The route value doesn't make much sense, we not the sender address from the very beginning
    //   Also, the empty route is a bad API
    pub fn notify_hole_open_receiver(&self) -> broadcast::Receiver<Route> {
        self.notify_hole_open_sender.subscribe()
    }
}
