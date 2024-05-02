//! Example for manual testing of Hole Punchers
//!
//! This example creates...
//! - (1) a node with a Rendezvous service
//! - (2) a node with an Echoer service and a puncher named `alice`
//! - (3) a node with an Echoer service and a puncher named `bob`
//!
//! The Rendezvous service will run until manually stopped with `Ctrl+C`.
//!
//! `alice` and `bob` punch holes through to each other and send and receive
//! messages to the other's Echoer.
//!
//! # Steps
//!
//! Open 3 shells (ideally, on different internet connected computers in
//! different networks) and in each change directory and, optionally, set
//! logging level.
//!
//! ```shell
//! cd ockam/implementations/rust/ockam/ockam_transport_udp
//! export OCKAM_LOG=info
//! ```
//!
//! In shell 1, start Rendezvous service on UDP port 4000.
//! It will run until stopped with `Ctrl+C`.
//!
//! ```shell
//! cargo run --example rendezvous_server -- 0.0.0.0:4000
//! ```
//!
//! In shell 2, start `bob` puncher.
//! Replace `<RS_IP>` with the public IP address of the Rendezvous service.
//! It will attempt to open a hole to `alice`, message the remote Echoer several
//! times, and then quit.
//!
//! ```shell
//! cargo run --example puncher -- bob alice <RS_IP>:4000
//! ```
//!
//! In shell 3, start `alice` puncher.
//! Replace `<RS_IP>` with the public IP address of the Rendezvous service.
//! It will attempt to open a hole to `bob`, message the remote Echoer several
//! times, and then quit.
//!
//! ```shell
//! cargo run --example puncher -- alice bob <RS_IP>:4000
//! ```
//!
//! On success, puncher process will exit with a zero exit code and
//! the log, if enabled, will show messages being exchanged by `alice` and
//! `bob`.
//!
//! On failure, puncher process will exit with a non-zero exit code and
//! the log, if enabled, will show more details.

use ockam::{
    errcode::{Kind, Origin},
    workers::Echoer,
};
use ockam_core::{route, Address, Error, Result};
use ockam_node::Context;
use ockam_transport_udp::{
    UdpBindArguments, UdpBindOptions, UdpHolePuncher, UdpHolePuncherOptions, UdpTransport,
};
use rand::Rng;
use std::ops::Range;
use tracing::{error, info};

/// Address of Echoer service
const ECHOER: &str = "echoer";

const MESSAGE_COUNT: usize = 10;
const SLEEP_SHORT_RANGE_MILLIS: Range<u64> = 100..1000;
const SLEEP_LONG_MILLIS: u64 = 3000;

#[ockam_macros::node]
async fn main(mut ctx: Context) -> Result<()> {
    let res = do_main(&mut ctx).await;
    match res {
        Ok(()) => Ok(()),
        Err(e) => {
            error!("ERROR: {:?}", e);
            ctx.stop().await?;
            Err(e)
        }
    }
}

async fn do_main(ctx: &mut Context) -> Result<()> {
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

    // Create transport, echoer service and puncher
    let udp = UdpTransport::create(ctx).await?;

    let bind = udp
        .bind(UdpBindArguments::new(), UdpBindOptions::new())
        .await?;

    ctx.start_worker(ECHOER, Echoer).await?;

    let mut puncher = UdpHolePuncher::create(
        ctx,
        &bind,
        "".to_string(),
        Address::random_local(),
        Address::random_local(),
        UdpHolePuncherOptions::new(),
        false,
    )
    .await?;
    ctx.flow_controls()
        .add_consumer(ECHOER, puncher.flow_control_id());
    info!("Puncher address = {:?}", puncher.sender_address());

    // Wait for hole to open
    info!("Waiting for hole to open");
    puncher.wait_for_hole_open().await?;
    info!("Hole open!");

    // Exchange messages with peer
    let r = route![puncher.sender_address(), ECHOER];
    for i in 1..=MESSAGE_COUNT {
        // Try to send messages to remote echoer
        let msg = format!(
            "Testing {} => {}, {} of {}",
            this_name, that_name, i, MESSAGE_COUNT
        );
        info!("Sending: {:?}", msg);
        let res = ctx
            .send_and_receive::<String>(r.clone(), msg.clone())
            .await?;
        info!("Received: {:?}", res);

        // Validate received message
        if res != msg {
            return Err(Error::new(
                Origin::Application,
                Kind::Other,
                format!(
                    "Message sent does not match message received: '{}' vs '{}'",
                    res, msg
                ),
            ));
        }

        let millis = {
            let mut rng = rand::thread_rng();
            rng.gen_range(SLEEP_SHORT_RANGE_MILLIS)
        };
        info!("Sleeping {}mS", millis);
        tokio::time::sleep(tokio::time::Duration::from_millis(millis)).await;
    }

    // Sleep before shutdown (in case peer needs us to exist a bit longer)
    info!("Sleeping {}mS before shutdown", SLEEP_LONG_MILLIS);
    tokio::time::sleep(tokio::time::Duration::from_millis(SLEEP_LONG_MILLIS)).await;

    // Shutdown
    info!("Done");
    ctx.stop().await?;
    Ok(())
}
