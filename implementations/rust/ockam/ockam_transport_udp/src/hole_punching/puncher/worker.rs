use crate::hole_punching::puncher::message::PunchMessage;
use crate::hole_punching::puncher::sender::UdpHolePunchSenderWorker;
use crate::hole_punching::puncher::{Addresses, UdpHolePuncherOptions};
use crate::transport::common::UdpBind;
use crate::{PunchError, UDP};
use ockam_core::compat::sync::Arc;
use ockam_core::{
    route, Address, AllowAll, Any, Decodable, DenyAll, LocalMessage, Mailbox, Mailboxes, Result,
    Route, Routed, Worker,
};
use ockam_node::{Context, DelayedEvent, WorkerBuilder};
use std::time::{Duration, Instant};
use tokio::sync::broadcast::Sender;
use tracing::{debug, info, trace};

const HEARTBEAT_EVERY: Duration = Duration::from_secs(1);
const HOLE_OPEN_TIMEOUT: Duration = Duration::from_secs(10); // FIXME: 20?

// TODO: Possible future improvement, explicitly send list of possible
// reachable addresses (usually local IPs) to Rendezvous service, to allow
// opening holes to local nodes

// TODO: Should `hole_puncher` and `rendezvous_service` files be moved to their
// own crate outside  `ockam_transport_udp`?

/// [`Worker`] for UDP NAT Hole Puncher
///
/// Using a remote Rendezvous service [`UdpRendezvousService`](crate::rendezvous_service::UdpRendezvousService`) tries to create
/// one half of a bi-directional NAT hole with a remote Hole Puncher.
///
/// See documentation for [`UdpHolePuncher`](crate::hole_puncher::UdpHolePuncher).
///
/// # 'Main' Mailbox
///
/// Sends to...
/// - the remote peer's puncher [`UdpHolePunchWorker`]
/// - the remote Rendezvous service
/// - this puncher's local handle [`UdpHolePuncher`](crate::hole_puncher::UdpHolePuncher)
///
/// Receives from...
/// - the remote peer's puncher [`UdpHolePunchWorker`]
/// - this puncher's local handle [`UdpHolePuncher`](crate::hole_puncher::UdpHolePuncher)
///
/// # 'Local' Mailbox
///
/// Sends and receives to and from entities in the local node.
///
/// Messages received by the 'local' mailbox are sent on to the peer's
/// puncher [`UdpHolePunchWorker`] from the 'main' mailbox.
///
/// Messages received from the peer's puncher [`UdpHolePunchWorker`]
/// by the 'main' mailbox are forwarded to local entities from
/// the 'local' mailbox.
pub(crate) struct UdpHolePunchWorker {
    addresses: Addresses,
    /// For generating internal heartbeat messages
    heartbeat: DelayedEvent<()>,
    /// Is hole open to peer?
    hole_open: bool,
    /// Notify if someone is waiting for the hole
    notify_hole_open_sender: Sender<Route>,
    /// Route to peer node's puncher
    peer_route: Route,
    /// Timestamp of most recent message received from peer
    peer_received_at: Instant,
    first_ping_received: bool,
    recipient_address: Address,
    redirect_first_message_to_transport: bool,
    pongs_sent: u64,
}

impl UdpHolePunchWorker {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn create(
        ctx: &Context,
        bind: &UdpBind,
        peer_udp_address: String,
        recipient_address: Address,
        addresses: Addresses,
        notify_hole_open_sender: Sender<Route>,
        options: UdpHolePuncherOptions,
        redirect_first_message_to_transport: bool,
    ) -> Result<()> {
        let peer_udp_address = Address::new(UDP, peer_udp_address);

        let peer_route = route![bind.sender_address().clone(), peer_udp_address];

        let heartbeat =
            DelayedEvent::create(ctx, addresses.heartbeat_address().clone(), ()).await?;

        let remote_mailbox = Mailbox::new(
            addresses.remote_address().clone(),
            Arc::new(AllowAll),
            Arc::new(AllowAll),
        );

        options.setup_flow_control(ctx.flow_controls(), &addresses, bind.sender_address())?;

        let receiver_mailbox = Mailbox::new(
            addresses.receiver_address().clone(),
            Arc::new(DenyAll),
            options.create_receiver_outgoing_access_control(ctx.flow_controls()),
        );

        let heartbeat_mailbox = Mailbox::new(
            addresses.heartbeat_address().clone(),
            Arc::new(AllowAll),
            Arc::new(DenyAll),
        );

        let sender_worker = UdpHolePunchSenderWorker::new(notify_hole_open_sender.subscribe());

        WorkerBuilder::new(sender_worker)
            .with_address(addresses.sender_address().clone())
            .with_outgoing_access_control(AllowAll)
            .with_incoming_access_control(AllowAll)
            .start(ctx)
            .await?;

        // Create and start worker
        let worker = Self {
            addresses: addresses.clone(),
            heartbeat,
            hole_open: false,
            notify_hole_open_sender,
            peer_route,
            peer_received_at: Instant::now(),
            first_ping_received: false,
            recipient_address,
            redirect_first_message_to_transport,
            pongs_sent: 0,
        };

        WorkerBuilder::new(worker)
            .with_mailboxes(Mailboxes::new(
                remote_mailbox,
                vec![receiver_mailbox, heartbeat_mailbox],
            ))
            .start(ctx)
            .await?;

        Ok(())
    }

    /// Update state to show the hole to peer is now open
    async fn set_hole_open(&mut self) -> Result<()> {
        self.hole_open = true;

        info!("Hole open. Address={}", self.peer_route);

        // Inform subscribers, ignore error
        // let route = route![self.addresses.] // FIXME
        let _ = self.notify_hole_open_sender.send(route![
            self.peer_route.clone(),
            self.recipient_address.clone()
        ]);

        Ok(())
    }

    /// Handle messages from peer
    async fn handle_peer(
        &mut self,
        ctx: &mut Context,
        msg: Routed<Any>,
        return_route: &Route,
    ) -> Result<()> {
        let msg = PunchMessage::decode(msg.payload())?;
        debug!("Peer => Puncher: {:?}", msg);

        // Record contact with peer, but only for pong and payload message.
        // Ping message doesn't guarantee that the other side is reachable
        let now = Instant::now();

        // Handle message
        match msg {
            PunchMessage::Ping => {
                debug!("Received Ping from peer. Will Pong.");
                self.first_ping_received = true;
                if self.pongs_sent < 20 {
                    ctx.send_from_address(
                        return_route.clone(),
                        PunchMessage::Pong,
                        self.addresses.remote_address().clone(),
                    )
                    .await?;
                    self.pongs_sent += 1;
                }
            }
            PunchMessage::Pong => {
                debug!("Received Pong from peer. Setting as hole is open");
                self.peer_received_at = now;
                self.set_hole_open().await?;
            }
            PunchMessage::Payload {
                onward_route,
                mut return_route,
                payload,
            } => {
                trace!("Received Payload from peer. Will forward to local entity");

                self.peer_received_at = now;
                let return_route = return_route
                    .modify()
                    .prepend(self.addresses.sender_address().clone())
                    .into();

                // Update routing & payload
                let local_message = LocalMessage::new()
                    .with_onward_route(onward_route)
                    .with_return_route(return_route)
                    .with_payload(payload);

                // Forward
                ctx.forward_from_address(local_message, self.addresses.receiver_address().clone())
                    .await?;
            }
        }
        Ok(())
    }

    /// Handle heartbeat messages
    async fn handle_heartbeat_impl(&mut self, ctx: &mut Context) -> Result<()> {
        debug!(
            "Heartbeat => Puncher: hole_open = {:?}, peer_route = {:?}",
            self.hole_open, self.peer_route
        );

        // If we have not heard from peer for a while, consider hole as closed
        if self.hole_open && self.peer_received_at.elapsed() >= HOLE_OPEN_TIMEOUT {
            info!("Not heard from peer for a while. Setting as hole closed.",);

            _ = self.notify_hole_open_sender.send(route![]);

            // Shut down itself
            ctx.stop_worker(self.addresses.remote_address().clone())
                .await?;

            return Ok(());
        }

        // Do keepalive pings to try and keep the hole open
        trace!("Pinging peer for keepalive");

        let route = if !self.first_ping_received && self.redirect_first_message_to_transport {
            self.peer_route.clone()
        } else {
            route![self.peer_route.clone(), self.recipient_address.clone()]
        };

        ctx.send_from_address(
            route,
            PunchMessage::Ping,
            self.addresses.remote_address().clone(),
        )
        .await?;

        Ok(())
    }

    /// Handle heartbeat messages
    async fn handle_heartbeat(&mut self, ctx: &mut Context) -> Result<()> {
        debug!(
            "Heartbeat => Puncher: hole_open = {:?}, peer_route = {:?}",
            self.hole_open, self.peer_route
        );

        let res = self.handle_heartbeat_impl(ctx).await;

        // Schedule next heartbeat here in case something errors
        self.heartbeat.schedule(HEARTBEAT_EVERY).await?;

        res
    }
}

#[ockam_core::worker]
impl Worker for UdpHolePunchWorker {
    type Message = Any;
    type Context = Context;

    async fn initialize(&mut self, _context: &mut Self::Context) -> Result<()> {
        self.heartbeat.schedule(Duration::ZERO).await?;

        Ok(())
    }

    async fn shutdown(&mut self, ctx: &mut Self::Context) -> Result<()> {
        self.heartbeat.cancel();

        _ = ctx
            .stop_worker(self.addresses.sender_address().clone())
            .await;

        Ok(())
    }

    async fn handle_message(
        &mut self,
        ctx: &mut Context,
        msg: Routed<Self::Message>,
    ) -> Result<()> {
        match msg.msg_addr() {
            // 'remote' mailbox
            addr if &addr == self.addresses.remote_address() => {
                let return_route = msg.return_route();

                self.handle_peer(ctx, msg, &return_route).await?;
            }

            // 'heartbeat' mailbox
            addr if &addr == self.addresses.heartbeat_address() => {
                self.handle_heartbeat(ctx).await?;
            }

            _ => return Err(PunchError::Internal)?,
        };

        Ok(())
    }
}
