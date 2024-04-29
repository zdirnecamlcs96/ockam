use hello_ockam::Echoer;
use log::{error, info};
use ockam::identity::SecureChannelListenerOptions;
use ockam::Context;
use ockam::{node, Node, TcpOutletOptions, TcpTransportExtension};
use ockam_core::{route, Result};
use ockam_transport_tcp::TcpListenerOptions;
use ockam_transport_udp::{
    UdpBindArguments, UdpBindOptions, UdpHolePuncherNegotiationListener, UdpTransportExtension, UDP,
};

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
    let outlet_address = std::env::args().nth(1).unwrap_or("127.0.0.1:5000".to_string());
    let outlet_node_address = std::env::args().nth(2).unwrap_or("127.0.0.1:4321".to_string());
    let rendezvous_addr = std::env::args().nth(3).unwrap_or("127.0.0.1:4000".to_string());
    info!("rendezvous = {}", rendezvous_addr);

    let identity = node.create_identity().await?;

    let tcp = node.create_tcp_transport().await?;

    let tcp_listener = tcp.listen(outlet_node_address, TcpListenerOptions::new()).await?;

    let sc_options = SecureChannelListenerOptions::new().as_consumer(tcp_listener.flow_control_id());
    let sc_flow_control_id = sc_options.spawner_flow_control_id();
    let sc_options = sc_options.as_consumer(&sc_flow_control_id);

    let outlet_options = TcpOutletOptions::new().as_consumer(&sc_options.spawner_flow_control_id());
    node.create_secure_channel_listener(&identity, "api", sc_options)
        .await?;

    tcp.create_outlet("outlet", outlet_address, outlet_options).await?;

    let udp = node.create_udp_transport().await?;
    let udp_bind = udp
        .bind(
            UdpBindArguments::new().with_bind_address("0.0.0.0:0")?,
            UdpBindOptions::new(),
        )
        .await?;
    let rendezvous_route = route![(UDP, rendezvous_addr), RENDEZVOUS];
    UdpHolePuncherNegotiationListener::create(node.context(), "udp", &udp_bind, rendezvous_route).await?;

    node.flow_controls().add_consumer("udp", &sc_flow_control_id);

    node.context().start_worker("echo", Echoer).await?;

    node.flow_controls().add_consumer("echo", &sc_flow_control_id);

    Ok(())
}
