use crate::hole_puncher::message::PunchMessage;
use crate::hole_puncher::sender::UdpHolePunchSenderWorker;
use crate::hole_puncher::{Addresses, UdpHolePuncherOptions};
use crate::rendezvous_service::{RendezvousRequest, RendezvousResponse};
use crate::transport::common::UdpBind;
use crate::PunchError;
use ockam_core::compat::sync::Arc;
use ockam_core::compat::sync::RwLock;
use ockam_core::errcode::{Kind, Origin};
use ockam_core::{
    route, Address, AllowAll, Any, Decodable, DenyAll, Error, LocalMessage, Mailbox, Mailboxes,
    Result, Route, Routed, Worker,
};
use ockam_node::{Context, DelayedEvent, MessageSendReceiveOptions, WorkerBuilder};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio::sync::broadcast::Sender;
use tracing::{debug, error, trace};

const HEARTBEAT_EVERY: Duration = Duration::from_secs(1);
const HOLE_OPEN_TIMEOUT: Duration = Duration::from_secs(20);
const PING_TRIES: usize = 5;

// UDP and NAT Hole Punching are unreliable protocols. Expect send and receive
// failures and don't wait too long for them
const QUICK_TIMEOUT: Duration = Duration::from_secs(3);

// TODO: Maybe, implement buffering of messages when hole is not open?

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
    /// Address of the corresponding sender
    udp_sender_address: Address,
    /// Route to Rendezvous service
    rendezvous_route: Route,
    /// Name of this puncher
    this_puncher_name: String,
    /// Name of peer node's puncher
    peer_puncher_name: String,
    /// Is hole open to peer?
    hole_open: Arc<RwLock<bool>>,
    /// Notify if someone is waiting for the hole
    notify_hole_open_sender: Sender<Route>,
    /// Route to peer node's puncher
    peer_route: Option<Route>,
    /// Timestamp of most recent message received from peer
    peer_received_at: Instant,
}

impl UdpHolePunchWorker {
    /// Update the Rendezvous service
    async fn rendezvous_update(&self, ctx: &mut Context) -> Result<()> {
        let msg = RendezvousRequest::Update {
            puncher_name: self.this_puncher_name.clone(),
        };
        // TODO: Add confirmation
        ctx.send_from_address(
            self.rendezvous_route.clone(),
            msg,
            self.addresses.remote_address().clone(),
        )
        .await
    }

    /// Query the Rendezvous service
    async fn rendezvous_query(&self, ctx: &mut Context) -> Result<Route> {
        let msg = RendezvousRequest::Query {
            puncher_name: self.peer_puncher_name.clone(),
        };

        // Send from a temporary context/address, so we can process the reply here
        let res = ctx
            .send_and_receive_extended::<RendezvousResponse>(
                self.rendezvous_route.clone(),
                msg,
                MessageSendReceiveOptions::new().with_timeout(QUICK_TIMEOUT),
            )
            .await?
            .into_body()?;

        let r = match res {
            RendezvousResponse::Query(r) => r,
            _ => return Err(PunchError::Internal)?,
        };

        let r = match r {
            None => return Err(PunchError::Internal)?,
            Some(r) => r,
        };

        Ok(route![self.udp_sender_address.clone(), r])
    }

    /// Query the Rendezvous service
    async fn rendezvous_get_my_address(&self, ctx: &mut Context) -> Result<Route> {
        let res = ctx
            .send_and_receive_extended::<RendezvousResponse>(
                self.rendezvous_route.clone(),
                RendezvousRequest::GetMyAddress,
                MessageSendReceiveOptions::new().with_timeout(QUICK_TIMEOUT),
            )
            .await?
            .into_body()?;

        let r = match res {
            RendezvousResponse::Query(r) => r,
            _ => return Err(PunchError::Internal)?,
        };

        let r = match r {
            None => return Err(PunchError::Internal)?,
            Some(r) => r,
        };

        Ok(route![self.udp_sender_address.clone(), r])
    }

    /// Test to see if we can reach the Rendezvous service
    pub(crate) async fn rendezvous_reachable(
        ctx: &Context,
        rendezvous_route: &Route,
    ) -> Result<bool> {
        for _ in 0..PING_TRIES {
            trace!("Start attempt to check Rendezvous service reachability");
            let res: Result<Routed<RendezvousResponse>> = ctx
                .send_and_receive_extended(
                    rendezvous_route.clone(),
                    RendezvousRequest::Ping,
                    MessageSendReceiveOptions::new().with_timeout(QUICK_TIMEOUT),
                )
                .await;

            // Check response. Ignore all but success.
            if let Ok(msg) = res {
                if let RendezvousResponse::Pong = msg.into_body()? {
                    trace!("Success reaching Rendezvous service");
                    return Ok(true);
                };
            }
        }
        trace!("Failed to reach Rendezvous service");
        Ok(false)
    }

    pub(crate) async fn create(
        ctx: &Context,
        bind: &UdpBind,
        rendezvous_route: Route,
        this_puncher_name: &str,
        peer_puncher_name: &str,
        options: UdpHolePuncherOptions,
    ) -> Result<(Addresses, broadcast::Receiver<Route>)> {
        // Create worker's addresses, heartbeat & mailboxes
        let addresses = Addresses::generate(this_puncher_name);

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

        let (notify_hole_open_sender, notify_hole_open_receiver) = broadcast::channel(1);
        let hole_open = Arc::new(RwLock::new(false));

        let sender_worker =
            UdpHolePunchSenderWorker::new(notify_hole_open_sender.subscribe(), hole_open.clone());

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
            udp_sender_address: bind.sender_address().clone(),
            rendezvous_route,
            this_puncher_name: String::from(this_puncher_name),
            peer_puncher_name: String::from(peer_puncher_name),
            hole_open: Arc::new(RwLock::new(false)),
            notify_hole_open_sender,
            peer_route: None,
            peer_received_at: Instant::now(),
        };
        WorkerBuilder::new(worker)
            .with_mailboxes(Mailboxes::new(
                remote_mailbox,
                vec![receiver_mailbox, heartbeat_mailbox],
            ))
            .start(ctx)
            .await?;

        Ok((addresses, notify_hole_open_receiver))
    }

    /// Update state to show the hole to peer is now open
    async fn set_hole_open(&mut self) -> Result<()> {
        *self.hole_open.write().unwrap() = true;

        let peer_route = if let Some(peer_route) = self.peer_route.clone() {
            debug!("Hole open. Address={}", peer_route);
            peer_route
        } else {
            return Err(Error::new(
                Origin::Transport,
                Kind::Internal,
                "Hole is open but peer_route is empty",
            ))?;
        };

        // Inform subscribers, ignore error
        let _ = self.notify_hole_open_sender.send(peer_route);

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

        // Record contact with peer
        self.peer_received_at = Instant::now();

        // Handle message
        match msg {
            PunchMessage::Ping => {
                debug!("Received Ping from peer. Will Pong.");
                ctx.send_from_address(
                    return_route.clone(),
                    PunchMessage::Pong,
                    self.addresses.remote_address().clone(),
                )
                .await?;
            }
            PunchMessage::Pong => {
                debug!("Received Pong from peer. Setting as hole is open");
                self.set_hole_open().await?;
            }
            PunchMessage::Payload {
                onward_route,
                mut return_route,
                payload,
            } => {
                trace!("Received Payload from peer. Will forward to local entity");

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

        let mut hole_open = *self.hole_open.read().unwrap();

        // If we have not heard from peer for a while, consider hole as closed
        if hole_open && self.peer_received_at.elapsed() >= HOLE_OPEN_TIMEOUT {
            trace!("Not heard from peer for a while. Setting as hole closed.",);
            *self.hole_open.write().unwrap() = false;
            hole_open = false;
        }

        if !hole_open {
            // Attempt hole open if it is closed
            trace!("Hole closed. Will attempt to open hole to peer");

            // Update Rendezvous service
            self.rendezvous_update(ctx).await?;

            // Query Rendezvous service
            if let Ok(peer_route) = self.rendezvous_query(ctx).await {
                self.peer_route = Some(peer_route.clone());

                // Ping peer
                ctx.send_from_address(
                    peer_route.clone(),
                    PunchMessage::Ping,
                    self.addresses.remote_address().clone(),
                )
                .await?;
            }
        } else {
            // Do keepalive pings to try and keep the hole open
            let peer_route = if let Some(peer_route) = self.peer_route.as_ref() {
                peer_route.clone()
            } else {
                error!("UDP hole is open but peer_route is unknown");
                return Err(Error::new(
                    Origin::Transport,
                    Kind::Internal,
                    "UDP hole is open but peer_route is unknown",
                ));
            };

            trace!("Pinging peer for keepalive");
            ctx.send_from_address(
                peer_route.clone(),
                PunchMessage::Ping,
                self.addresses.remote_address().clone(),
            )
            .await?;
        }

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
            .stop_processor(self.addresses.sender_address().clone())
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
