pub(crate) mod common;

mod bind;
mod lifecycle;

pub use bind::*;

use ockam_core::compat::sync::Arc;
use ockam_core::{async_trait, Result};
use ockam_node::{Context, HasContext};

/// UDP Transport
#[derive(Clone, Debug)]
pub struct UdpTransport {
    ctx: Arc<Context>,
    // TODO: Add registry,
}

/// FIXME
#[async_trait]
pub trait UdpTransportExtension: HasContext {
    /// Create a TCP transport
    async fn create_udp_transport(&self) -> Result<UdpTransport> {
        UdpTransport::create(self.get_context()).await
    }
}

impl<A: HasContext> UdpTransportExtension for A {}
