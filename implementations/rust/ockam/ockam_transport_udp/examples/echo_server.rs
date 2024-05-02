use ockam_core::{Result, Routed, Worker};
use ockam_node::Context;
use ockam_transport_udp::{UdpBindArguments, UdpBindOptions, UdpTransport};
use tracing::debug;

#[ockam_macros::node]
async fn main(ctx: Context) -> Result<()> {
    let udp = UdpTransport::create(&ctx).await?;
    let bind = udp
        .bind(
            UdpBindArguments::new().with_bind_address("127.0.0.1:8000")?,
            UdpBindOptions::new(),
        )
        .await?;

    ctx.start_worker("echoer", Echoer).await?;

    ctx.flow_controls()
        .add_consumer("echoer", bind.flow_control_id());

    Ok(())
}
pub struct Echoer;

#[ockam_core::worker]
impl Worker for Echoer {
    type Message = String;
    type Context = Context;

    async fn handle_message(&mut self, ctx: &mut Context, msg: Routed<String>) -> Result<()> {
        debug!("Replying back to {}", &msg.return_route());
        ctx.send(msg.return_route(), msg.into_body()?).await
    }
}
