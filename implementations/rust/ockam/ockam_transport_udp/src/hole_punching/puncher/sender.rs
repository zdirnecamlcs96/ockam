use crate::hole_punching::puncher::message::PunchMessage;
use crate::PunchError;
use ockam_core::{Any, Encodable, LocalMessage, Result, Route, Routed, Worker};
use ockam_node::Context;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Receiver;
use tracing::{debug, error};

pub(crate) struct UdpHolePunchSenderWorker {
    notify_hole_open_receiver: Receiver<Route>,
    peer_route: Option<Route>,
}

impl UdpHolePunchSenderWorker {
    pub fn new(notify_hole_open_receiver: Receiver<Route>) -> Self {
        Self {
            notify_hole_open_receiver,
            peer_route: None,
        }
    }

    /// Handle messages from local entities
    async fn handle_local(&mut self, ctx: &mut Context, msg: Routed<Any>) -> Result<()> {
        debug!("Local => Puncher: {:?}", msg);

        let peer_route = self.peer_route.clone().ok_or(PunchError::HoleNotOpen)?;

        debug!("App => Puncher: {:?}", msg);

        let onward_route = msg.onward_route().modify().pop_front().into();
        let return_route = msg.return_route();

        // Wrap payload
        let wrapped_payload = PunchMessage::Payload {
            onward_route,
            return_route,
            payload: msg.into_payload(),
        };

        let msg = LocalMessage::new()
            .with_onward_route(peer_route)
            .with_payload(wrapped_payload.encode()?);

        // Forward
        debug!("Puncher => Peer: {:?}", msg);
        ctx.forward(msg).await
    }

    async fn wait_hole_open(&mut self, ctx: &Context) -> Result<()> {
        loop {
            match self.notify_hole_open_receiver.recv().await {
                Ok(peer_route) => {
                    self.peer_route = Some(peer_route);
                    return Ok(());
                }
                Err(err) => match err {
                    RecvError::Closed => {
                        error!("Error waiting for a UDP hole: {}", err);
                        ctx.stop_processor(ctx.address()).await?
                    }
                    RecvError::Lagged(_) => continue,
                },
            }
        }
    }
}

#[ockam_core::worker]
impl Worker for UdpHolePunchSenderWorker {
    type Message = Any;
    type Context = Context;

    async fn initialize(&mut self, ctx: &mut Self::Context) -> Result<()> {
        self.wait_hole_open(ctx).await?;

        Ok(())
    }

    async fn handle_message(
        &mut self,
        ctx: &mut Context,
        msg: Routed<Self::Message>,
    ) -> Result<()> {
        self.handle_local(ctx, msg).await?;

        Ok(())
    }
}
