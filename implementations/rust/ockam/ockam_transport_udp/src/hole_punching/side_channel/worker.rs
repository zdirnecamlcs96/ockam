use crate::hole_punching::rendezvous_service::RendezvousClient;
use crate::hole_punching::side_channel::message::UdpHolePuncherNegotiationMessage;
use crate::transport::common::UdpBind;
use crate::{UdpHolePuncher, UdpHolePuncherOptions};
use ockam_core::{async_trait, route, Address, Any, Decodable, Route, Routed, Worker};
use ockam_node::Context;
use tokio::sync::broadcast::Sender;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

pub struct UdpHolePuncherNegotiationWorker {
    onward_route: Option<Route>,
    is_initiator: bool,
    udp_bind: UdpBind,

    client: RendezvousClient,
    my_udp_public_address: Option<String>,
    their_udp_public_address: Option<String>,
    their_current_remote_address: Option<Address>,

    current_remote_address: Address,
    current_puncher: Option<UdpHolePuncher>,
    current_handle: Option<JoinHandle<()>>,

    notify_reachable: Option<Sender<Route>>,
}

impl UdpHolePuncherNegotiationWorker {
    pub fn new_initiator(
        onward_route: Route,
        udp_bind: &UdpBind,
        client: RendezvousClient,
        notify_reachable: Sender<Route>,
    ) -> Self {
        let current_remote_address = Address::random_tagged("UdpHolePuncher.remote");
        Self {
            onward_route: Some(onward_route),
            is_initiator: true,
            udp_bind: udp_bind.clone(),
            client,
            my_udp_public_address: None,
            their_udp_public_address: None,
            their_current_remote_address: None,
            current_remote_address,
            current_puncher: None,
            current_handle: None,
            notify_reachable: Some(notify_reachable),
        }
    }

    pub fn new_responder(udp_bind: &UdpBind, client: RendezvousClient) -> Self {
        let current_remote_address = Address::random_tagged("UdpHolePuncher.remote");
        Self {
            onward_route: None,
            is_initiator: false,
            udp_bind: udp_bind.clone(),
            client,
            my_udp_public_address: None,
            their_udp_public_address: None,
            their_current_remote_address: None,
            current_remote_address,
            current_puncher: None,
            current_handle: None,
            notify_reachable: None,
        }
    }

    fn role(&self) -> &str {
        if self.is_initiator {
            "initiator"
        } else {
            "responder"
        }
    }
}

#[async_trait]
impl Worker for UdpHolePuncherNegotiationWorker {
    type Message = Any;
    type Context = Context;

    async fn initialize(&mut self, ctx: &mut Self::Context) -> ockam_core::Result<()> {
        debug!(
            "Initializing UdpHolePuncherNegotiationWorker {} as {}",
            self.role(),
            ctx.address()
        );
        let my_udp_public_address = self.client.get_my_address().await?;

        info!(
            "UdpHolePuncherNegotiationWorker {} at {} got its public address: {}",
            self.role(),
            ctx.address(),
            my_udp_public_address
        );

        self.my_udp_public_address = Some(my_udp_public_address.clone());

        if self.is_initiator {
            ctx.send(
                self.onward_route.clone().unwrap(), // FIXME
                UdpHolePuncherNegotiationMessage::Initiate {
                    initiator_udp_public_address: my_udp_public_address,
                    current_remote_address: self.current_remote_address.to_string(),
                },
            )
            .await?;
        }

        Ok(())
    }

    async fn handle_message(
        &mut self,
        ctx: &mut Self::Context,
        msg: Routed<Self::Message>,
    ) -> ockam_core::Result<()> {
        let return_route = msg.return_route();
        let msg = UdpHolePuncherNegotiationMessage::decode(msg.payload())?;

        info!(
            "UdpHolePuncherNegotiationWorker {} at {} received msg: {:?}",
            self.role(),
            ctx.address(),
            msg
        );

        // TODO: Stop existing puncher

        let puncher = match msg {
            UdpHolePuncherNegotiationMessage::Initiate {
                initiator_udp_public_address,
                current_remote_address,
            } => {
                if self.is_initiator {
                    panic!() // FIXME
                }
                self.their_udp_public_address = Some(initiator_udp_public_address.clone());
                self.their_current_remote_address = Some(Address::from(current_remote_address));

                let options = UdpHolePuncherOptions::new(); // FIXME
                UdpHolePuncher::create(
                    ctx,
                    &self.udp_bind,
                    self.their_udp_public_address.clone().unwrap(), // FIXME
                    self.current_remote_address.clone(),
                    self.their_current_remote_address.take().unwrap(), // FIXME
                    options,
                    true,
                )
                .await?;

                ctx.send(
                    return_route,
                    UdpHolePuncherNegotiationMessage::Acknowledge {
                        responder_udp_public_address: self.my_udp_public_address.clone().unwrap(), // FIXME
                        current_remote_address: self.current_remote_address.to_string(),
                    },
                )
                .await?;

                return Ok(());
            }
            UdpHolePuncherNegotiationMessage::Acknowledge {
                responder_udp_public_address,
                current_remote_address,
            } => {
                if !self.is_initiator {
                    panic!() // FIXME
                }
                let their_current_remote_address = Address::from(current_remote_address);
                self.their_udp_public_address = Some(responder_udp_public_address.clone());

                // TODO: Could have been started at the very beginning inside initialize
                let options = UdpHolePuncherOptions::new(); // FIXME
                UdpHolePuncher::create(
                    ctx,
                    &self.udp_bind,
                    responder_udp_public_address,
                    self.current_remote_address.clone(),
                    their_current_remote_address,
                    options,
                    false,
                )
                .await?
            }
        };

        if let Some(current_handle) = self.current_handle.take() {
            current_handle.abort();
        }

        if let Some(notify_reachable) = self.notify_reachable.clone() {
            let mut notify_hole_open_receiver = puncher.notify_hole_open_receiver();
            let route = route![puncher.sender_address()];
            self.current_handle = Some(tokio::spawn(async move {
                loop {
                    match notify_hole_open_receiver.recv().await {
                        Ok(r) => {
                            if !r.is_empty() {
                                info!("Notifying UDP hole is reachable");
                                _ = notify_reachable.send(route.clone())
                            } else {
                                info!("Notifying UDP hole is NOT reachable");
                                _ = notify_reachable.send(route![])
                            }
                        }
                        Err(err) => {
                            error!("Error receiving UDP hole notification: {}", err);
                        }
                    }
                }
            }));
        }

        self.current_puncher = Some(puncher);

        Ok(())
    }
}
