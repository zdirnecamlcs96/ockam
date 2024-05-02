use crate::hole_punching::rendezvous_service::RendezvousClient;
use crate::hole_punching::side_channel::message::UdpHolePuncherNegotiationMessage;
use crate::hole_punching::side_channel::worker::UdpHolePuncherNegotiationWorker;
use crate::transport::common::UdpBind;
use ockam_core::{
    async_trait, Address, Any, AsyncTryClone, Decodable, Result, Route, Routed, Worker,
};
use ockam_node::Context;

/// FIXME
pub struct UdpHolePuncherNegotiationListener {
    udp_bind: UdpBind,
    client: RendezvousClient,
}

impl UdpHolePuncherNegotiationListener {
    /// FIXME
    pub async fn create(
        ctx: &Context,
        address: impl Into<Address>,
        udp_bind: &UdpBind,
        rendezvous_route: Route,
    ) -> Result<()> {
        let client = RendezvousClient::new(ctx, udp_bind, rendezvous_route).await?;
        let s = Self {
            udp_bind: udp_bind.clone(),
            client,
        };

        ctx.start_worker(address, s).await?;

        Ok(())
    }
}

#[async_trait]
impl Worker for UdpHolePuncherNegotiationListener {
    type Message = Any;
    type Context = Context;

    async fn handle_message(
        &mut self,
        ctx: &mut Self::Context,
        msg: Routed<Self::Message>,
    ) -> Result<()> {
        let src_addr = msg.src_addr();
        let msg_payload = UdpHolePuncherNegotiationMessage::decode(msg.payload())?;

        if let UdpHolePuncherNegotiationMessage::Initiate { .. } = msg_payload {
            let address = Address::random_tagged("UdpHolePunctureNegotiator.responder");

            if let Some(producer_flow_control_id) = ctx
                .flow_controls()
                .get_flow_control_with_producer(&src_addr)
                .map(|x| x.flow_control_id().clone())
            {
                // Allow a sender with corresponding flow_control_id send messages to this address
                ctx.flow_controls()
                    .add_consumer(address.clone(), &producer_flow_control_id);
            }

            let worker = UdpHolePuncherNegotiationWorker::new_responder(
                &self.udp_bind,
                self.client.async_try_clone().await?,
            );

            let msg = msg
                .into_local_message()
                .pop_front_onward_route()?
                .push_front_onward_route(&address);

            ctx.start_worker(address, worker).await?; // FIXME: Access Control

            ctx.forward(msg).await?;
        }

        Ok(())
    }
}
