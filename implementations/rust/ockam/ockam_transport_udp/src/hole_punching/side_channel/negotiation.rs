use crate::hole_punching::rendezvous_service::RendezvousClient;
use crate::hole_punching::side_channel::worker::UdpHolePuncherNegotiationWorker;
use crate::transport::common::UdpBind;
use ockam_core::{Address, Result, Route};
use ockam_node::Context;
use tokio::sync::broadcast;
use tokio::sync::broadcast::Receiver;

/// FIXME
pub struct UdpHolePuncherNegotiation {}

impl UdpHolePuncherNegotiation {
    /// FIXME
    pub async fn start_negotiation(
        ctx: &Context,
        onward_route: Route,
        udp_bind: &UdpBind,
        rendezvous_route: Route,
    ) -> Result<Receiver<Route>> {
        let next = onward_route.next()?.clone();

        let client = RendezvousClient::new(ctx, udp_bind, rendezvous_route).await?;
        let (notify_reachable_sender, notify_reachable_receiver) = broadcast::channel(1);
        let worker = UdpHolePuncherNegotiationWorker::new_initiator(
            onward_route,
            udp_bind,
            client,
            notify_reachable_sender,
        );

        let address = Address::random_tagged("UdpHolePunctureNegotiator.initiator");

        if let Some(flow_control_id) = ctx
            .flow_controls()
            .find_flow_control_with_producer_address(&next)
            .map(|x| x.flow_control_id().clone())
        {
            // To be able to receive the response
            ctx.flow_controls()
                .add_consumer(address.clone(), &flow_control_id);
        }

        ctx.start_worker(address, worker).await?; // FIXME: Access Control

        Ok(notify_reachable_receiver)
    }
}
