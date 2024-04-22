use ockam::identity::SecureChannelListenerOptions;
use ockam::{node, Node, TcpOutletOptions, TcpTransportExtension};
use ockam_core::{route, Result};
use ockam_node::Context;
use ockam_transport_udp::{
    UdpBindArguments, UdpBindOptions, UdpHolePuncher, UdpHolePuncherOptions, UdpTransportExtension,
    UDP,
};
use std::time::Duration;
use tracing::{error, info};

/// Address of remote Rendezvous service
const RENDEZVOUS: &str = "rendezvous";

#[ockam_macros::node]
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
    let this_name = std::env::args().nth(1).unwrap();
    let that_name = std::env::args().nth(2).unwrap();
    let rendezvous_addr = std::env::args()
        .nth(3)
        .unwrap_or("127.0.0.1:4000".to_string());
    info!(
        "this_name = {}, that_name = {}, rendezvous = {}",
        this_name, that_name, rendezvous_addr
    );

    let identity = node.create_identity().await?;

    let udp = node.create_udp_transport().await?;
    let tcp = node.create_tcp_transport().await?;

    let bind = udp
        .bind(
            UdpBindArguments::new().with_bind_address("0.0.0.0:0")?,
            UdpBindOptions::new(),
        )
        .await?;

    let puncher_options = UdpHolePuncherOptions::new();
    let sc_options = SecureChannelListenerOptions::new()
        .as_consumer(&puncher_options.producer_flow_control_id());
    let outlet_options = TcpOutletOptions::new().as_consumer(&sc_options.spawner_flow_control_id());

    node.create_secure_channel_listener(&identity, "api", sc_options)
        .await?;

    let rendezvous_route = route![(UDP, rendezvous_addr), RENDEZVOUS];

    let mut puncher = UdpHolePuncher::create(
        node.context(),
        &bind,
        &this_name,
        &that_name,
        rendezvous_route,
        puncher_options,
    )
    .await?;
    info!("Puncher address = {:?}", puncher.sender_address());

    // Wait for hole to open
    info!("Waiting for hole to open");
    puncher.wait_for_hole_open().await?;
    info!("Hole open!");

    tcp.create_outlet("outlet", "127.0.0.1:5000", outlet_options)
        .await?;

    info!("Ready");

    node.context().sleep(Duration::from_secs(60)).await;

    Ok(())
}
