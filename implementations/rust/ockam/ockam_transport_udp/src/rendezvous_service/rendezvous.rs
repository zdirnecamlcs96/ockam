use crate::{
    rendezvous_service::{RendezvousRequest, RendezvousResponse},
    UDP,
};
use ockam_core::{async_trait, Address, Result, Route, Routed, Worker};
use ockam_node::Context;
use std::collections::BTreeMap;
use tracing::{debug, trace, warn};

/// High level management interface for UDP Rendezvous Service
///
/// The Rendezvous service is a part of UDP NAT Hole Punching (see [Wikipedia](https://en.wikipedia.org/wiki/UDP_hole_punching)).
///
/// A node could start multiple Rendezvous services, each with its own address.
///
/// To work, this service requires the UDP Transport to be working.
///
/// # Example
///
/// ```rust
/// use ockam_transport_udp::{UdpTransport, UdpBindOptions, UdpRendezvousService, UdpBindArguments};
/// # use ockam_node::Context;
/// # use ockam_core::Result;
/// # async fn test(ctx: Context) -> Result<()> {
///
/// // Start a Rendezvous service with address 'my_rendezvous' and listen on UDP port 4000
/// UdpRendezvousService::start(&ctx, "my_rendezvous").await?;
/// let udp = UdpTransport::create(&ctx).await?;
/// udp.bind(UdpBindArguments::new().with_bind_address("0.0.0.0:4000")?, UdpBindOptions::new()).await?;
/// # Ok(()) }
/// ```
pub struct UdpRendezvousService;

impl UdpRendezvousService {
    /// Start a new Rendezvous service with the given local address
    pub async fn start(ctx: &Context, address: impl Into<Address>) -> Result<()> {
        ctx.start_worker(address.into(), RendezvousWorker::new())
            .await
    }
}

// TODO: Implement mechanism for removing entries from internal map, possibly by
// deleting 'old' entries on heartbeat events.

/// Worker for the UDP NAT Hole Punching Rendezvous service
///
/// Maintains an internal map for remote nodes and the public IP address
/// from which they send UDP datagrams.
///
/// Remote nodes can send requests to update and query the map.
struct RendezvousWorker {
    map: BTreeMap<String, Route>,
}

impl Default for RendezvousWorker {
    fn default() -> Self {
        Self::new()
    }
}

impl RendezvousWorker {
    fn new() -> Self {
        Self {
            map: BTreeMap::new(),
        }
    }

    /// Extract from `return route` everything just before we received the
    /// message via UDP
    fn parse_route(return_route: &Route) -> Route {
        let addrs = return_route
            .iter()
            .skip_while(|x| x.transport_type() != UDP)
            .cloned();

        let mut res = Route::new();
        for a in addrs {
            res = res.append(a);
        }
        res.into()
    }

    /// Extract first UDP Address from `return route`
    fn get_udp_address(return_route: &Route) -> Option<Address> {
        return_route
            .iter()
            .find(|&x| x.transport_type() == UDP)
            .cloned()
    }

    // Handle Update request
    fn handle_update(&mut self, puncher_name: &str, return_route: &Route) {
        let r = Self::parse_route(return_route);
        if !r.is_empty() {
            debug!("Update {} route to {}", puncher_name, r);
            self.map.insert(puncher_name.to_owned(), r);
        } else {
            // This could happen if a client erroneously contacts this service over TCP not UDP, for example
            warn!(
                "Return route has no UDP part, will not update map: {:?}",
                return_route
            );
            // Ignore issue. There's no (current) way to inform sender.
        }
    }

    // Handle Query request
    fn handle_query(&self, puncher_name: &String) -> Option<Route> {
        match self.map.get(puncher_name) {
            Some(route) => {
                debug!("Return route for {}. Which is {}", puncher_name, route);
                Some(route.clone())
            }
            None => None,
        }
    }

    // Handle Update request
    fn handle_get_my_address(&mut self, return_route: &Route) -> Option<String> {
        Self::get_udp_address(return_route).map(|a| a.address().to_string())
    }
}

#[async_trait]
impl Worker for RendezvousWorker {
    type Message = RendezvousRequest;
    type Context = Context;

    async fn handle_message(
        &mut self,
        ctx: &mut Context,
        msg: Routed<Self::Message>,
    ) -> Result<()> {
        debug!(
            "Received message: {:?} from {}",
            msg,
            Self::parse_route(&msg.return_route())
        );
        let return_route = msg.return_route();
        match msg.into_body()? {
            RendezvousRequest::Update { puncher_name } => {
                self.handle_update(&puncher_name, &return_route);
            }
            RendezvousRequest::Query { puncher_name } => {
                let res = self.handle_query(&puncher_name);
                ctx.send(return_route, RendezvousResponse::Query(res))
                    .await?;
            }
            RendezvousRequest::Ping => {
                ctx.send(return_route, RendezvousResponse::Pong).await?;
            }
            RendezvousRequest::GetMyAddress => {
                let res = self.handle_get_my_address(&return_route);
                match res {
                    Some(udp_address) => {
                        ctx.send(return_route, RendezvousResponse::GetMyAddress(udp_address))
                            .await?;
                    }
                    None => {
                        // This could happen if a client erroneously contacts this service over TCP not UDP, for example
                        warn!(
                            "Return route has no UDP part, will not return address map: {:?}",
                            return_route
                        );
                        // Ignore issue. There's no (current) way to inform sender.
                    }
                }
            }
        }
        trace!("Map: {:?}", self.map);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::RendezvousWorker;
    use crate::rendezvous_service::{RendezvousRequest, RendezvousResponse};
    use crate::transport::common::UdpBind;
    use crate::transport::UdpBindArguments;
    use crate::{UdpBindOptions, UdpRendezvousService, UdpTransport, UDP};
    use ockam_core::{route, Result, Route, TransportType};
    use ockam_node::Context;

    #[test]
    fn parse_route() {
        assert_eq!(
            route![(UDP, "c")],
            RendezvousWorker::parse_route(&route!["a", "b", (UDP, "c")])
        );
        assert_eq!(
            route![(UDP, "c"), "d"],
            RendezvousWorker::parse_route(&route!["a", "b", (UDP, "c"), "d"])
        );
        assert_eq!(
            route![(UDP, "c"), "d", "e"],
            RendezvousWorker::parse_route(&route!["a", "b", (UDP, "c"), "d", "e"])
        );
        let not_udp = TransportType::new((u8::from(UDP)) + 1);
        assert_eq!(
            route![],
            RendezvousWorker::parse_route(&route!["a", "b", (not_udp, "c"), "d"])
        );
        assert_eq!(
            route![],
            RendezvousWorker::parse_route(&route!["a", "b", "c", "d"])
        );
        assert_eq!(route![], RendezvousWorker::parse_route(&route![]));
    }

    #[ockam_macros::test]
    async fn update_and_query(ctx: &mut Context) -> Result<()> {
        let (rendezvous_route, send_addr) = test_setup(ctx).await?;

        let our_public_route = route![(UDP, send_addr.to_string()), ctx.address()];

        // Update service, should work
        //
        // Use Alice and Bob with the same address to check the service can
        // handle multiple internal mappings and that multiple map values
        // can be for the same node.
        update_operation("Alice", ctx, &rendezvous_route)
            .await
            .unwrap();
        update_operation("Bob", ctx, &rendezvous_route)
            .await
            .unwrap();

        // Query service, should work
        let res = query_operation("Alice", ctx, &rendezvous_route)
            .await
            .unwrap();
        assert_eq!(res, our_public_route);
        let res = query_operation("Bob", ctx, &rendezvous_route)
            .await
            .unwrap();
        assert_eq!(res, our_public_route);

        // Query service for non-existent node, should error
        let res = query_operation("DoesNotExist", ctx, &rendezvous_route).await;
        assert!(res.is_none(), "Query operation should have failed");
        Ok(())
    }

    #[ockam_macros::test]
    async fn ping(ctx: &mut Context) -> Result<()> {
        let (rendezvous_route, _) = test_setup(ctx).await?;

        let res: RendezvousResponse = ctx
            .send_and_receive(rendezvous_route, RendezvousRequest::Ping)
            .await?;
        assert!(matches!(res, RendezvousResponse::Pong));
        Ok(())
    }

    #[ockam_macros::test]
    async fn get_my_address(ctx: &mut Context) -> Result<()> {
        let (rendezvous_route, udp_bind) = test_setup(ctx).await?;

        let res: RendezvousResponse = ctx
            .send_and_receive(rendezvous_route, RendezvousRequest::GetMyAddress)
            .await?;

        match res {
            RendezvousResponse::GetMyAddress(address) => {
                assert_eq!(address, udp_bind.bind_address().to_string());
            }
            _ => panic!(),
        }

        Ok(())
    }

    /// Helper
    async fn test_setup(ctx: &mut Context) -> Result<(Route, UdpBind)> {
        // Create transport, start rendezvous service, start echo service and listen
        let transport = UdpTransport::create(ctx).await?;
        UdpRendezvousService::start(ctx, "rendezvous").await?;

        let udp_bind = transport
            .bind(UdpBindArguments::new(), UdpBindOptions::new())
            .await?;

        ctx.flow_controls()
            .add_consumer("rendezvous", udp_bind.flow_control_id());

        let bind_addr = udp_bind.bind_address().to_string();

        let rendezvous_route = route![
            udp_bind.sender_address().clone(),
            (UDP, bind_addr.to_string()),
            "rendezvous"
        ];

        ctx.flow_controls()
            .add_consumer("echo", udp_bind.flow_control_id());

        Ok((rendezvous_route, udp_bind))
    }

    /// Helper
    async fn update_operation(puncher_name: &str, ctx: &mut Context, route: &Route) -> Result<()> {
        let msg = RendezvousRequest::Update {
            puncher_name: String::from(puncher_name),
        };

        // Send from our context's main address
        ctx.send(route.clone(), msg).await
    }

    /// Helper
    async fn query_operation(puncher_name: &str, ctx: &Context, route: &Route) -> Option<Route> {
        let msg = RendezvousRequest::Query {
            puncher_name: String::from(puncher_name),
        };
        let res: RendezvousResponse = ctx.send_and_receive(route.clone(), msg).await.unwrap();
        match res {
            RendezvousResponse::Query(r) => r,
            r => panic!("Unexpected response: {:?}", r),
        }
    }
}
