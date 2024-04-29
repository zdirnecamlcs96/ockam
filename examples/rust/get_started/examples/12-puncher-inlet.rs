use ockam::identity::SecureChannelOptions;
use ockam::transport::Transport;
use ockam::Context;
use ockam::{node, Node, TcpInletOptions, TcpTransportExtension};
use ockam_core::{route, Result};
use ockam_transport_tcp::TCP;
use ockam_transport_udp::{UdpBindArguments, UdpBindOptions, UdpHolePuncherNegotiation, UdpTransportExtension, UDP};
use std::time::Duration;
use tracing::{error, info};

/// Address of remote Rendezvous service
const RENDEZVOUS: &str = "rendezvous";

#[ockam::node]
async fn main(ctx: Context) -> Result<()> {
    let mut node = node(ctx).await?;
    let res = do_main(&mut node).await;
    match res {
        Ok(()) => Ok(()),
        Err(e) => {
            error!("ERROR: {:?}", e);
            node.stop().await?;
            Err(e)
        }
    }
}

async fn do_main(node: &mut Node) -> Result<()> {
    info!("Started");

    // Handle command line arguments
    let inlet_address = std::env::args().nth(1).unwrap_or("127.0.0.1:1234".to_string());
    let outlet_node_address = std::env::args().nth(2).unwrap_or("127.0.0.1:4321".to_string());
    let rendezvous_addr = std::env::args().nth(3).unwrap_or("127.0.0.1:4000".to_string());
    info!("rendezvous = {}", rendezvous_addr);

    let identity = node.create_identity().await?;

    let tcp = node.create_tcp_transport().await?;

    let outlet_address = tcp.resolve_address((TCP, outlet_node_address).into()).await?;

    let sc_main = node
        .create_secure_channel(&identity, route![outlet_address, "api"], SecureChannelOptions::new())
        .await?;

    let inlet = tcp
        .create_inlet(inlet_address, route![sc_main.clone(), "outlet"], TcpInletOptions::new())
        .await?;

    let sc_side = node
        .create_secure_channel(&identity, route![sc_main.clone(), "api"], SecureChannelOptions::new())
        .await?;

    let udp = node.create_udp_transport().await?;
    let udp_bind = udp
        .bind(
            UdpBindArguments::new().with_bind_address("0.0.0.0:0")?,
            UdpBindOptions::new(),
        )
        .await?;

    let rendezvous_route = route![(UDP, rendezvous_addr), RENDEZVOUS];

    node.context().sleep(Duration::from_secs(10)).await;

    let mut receiver = UdpHolePuncherNegotiation::start_negotiation(
        node.context(),
        route![sc_main.clone(), "udp"],
        &udp_bind,
        rendezvous_route,
    )
    .await?;

    let mut last_route = None;

    loop {
        match receiver.recv().await {
            Ok(route) => {
                if Some(&route) == last_route.as_ref() {
                    info!("Route did not change, skipping");
                    continue;
                }

                if !route.is_empty() {
                    info!("Updating route to UDP: {}", route);

                    sc_side.update_remote_node_route(route.clone())?;

                    inlet.update_outlet_node_route(route![sc_side.clone()])?;
                } else {
                    info!("Updating route to TCP");

                    inlet.update_outlet_node_route(route![sc_main.clone()])?;
                }

                last_route = Some(route);
            }
            // TODO: Handle more errors (overflow)
            Err(_) => {
                info!("Error. Updating route to TCP");

                inlet.update_outlet_node_route(route![sc_main.clone()])?;

                break Ok(());
            }
        }
    }
}
