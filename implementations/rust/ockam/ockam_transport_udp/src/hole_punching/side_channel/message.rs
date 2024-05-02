use minicbor::{Decode, Encode};
use ockam_core::{Decodable, Encodable, Message, Result};

#[derive(Encode, Decode, Debug, Clone)]
#[rustfmt::skip]
pub(crate) enum UdpHolePuncherNegotiationMessage {
    #[n(0)] Initiate {
        #[n(0)] initiator_udp_public_address: String,
        #[n(1)] current_remote_address: String,
    },
    #[n(1)] Acknowledge {
        #[n(0)] responder_udp_public_address: String,
        #[n(1)] current_remote_address: String,
    }
}

impl Encodable for UdpHolePuncherNegotiationMessage {
    fn encode(self) -> Result<Vec<u8>> {
        Ok(minicbor::to_vec(self)?)
    }
}

impl Decodable for UdpHolePuncherNegotiationMessage {
    fn decode(data: &[u8]) -> Result<Self> {
        Ok(minicbor::decode(data)?)
    }
}

impl Message for UdpHolePuncherNegotiationMessage {}
