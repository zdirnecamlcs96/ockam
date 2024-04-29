use log::{error, info};
use ockam::Context;
use ockam::{node, Node};
use ockam_core::{route, Result};
use ockam_transport_udp::{RendezvousClient, UdpBindArguments, UdpBindOptions, UdpTransportExtension, UDP};

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
    let rendezvous_addr = std::env::args().nth(1).unwrap();

    let udp = node.create_udp_transport().await?;
    let udp_bind = udp
        .bind(
            UdpBindArguments::new().with_bind_address("0.0.0.0:0")?,
            UdpBindOptions::new(),
        )
        .await?;
    let rendezvous_route = route![(UDP, rendezvous_addr), RENDEZVOUS];

    let client = RendezvousClient::new(node.context(), &udp_bind, rendezvous_route).await?;

    let address = client.get_my_address().await;

    info!("Address: {:?}", address);

    Ok(())
}
