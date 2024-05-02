use minicbor::{Decode, Encode};
use ockam_core::{Decodable, Encodable, Message, Result, Route};

/// Internal message type for UDP NAT Hole Puncher
#[derive(Encode, Decode, Debug, Clone)]
#[rustfmt::skip]
pub(crate) enum PunchMessage {
    #[n(0)] Ping,
    #[n(1)] Pong,
    #[n(2)] Payload {
        #[n(0)] onward_route: Route,
        #[n(1)] return_route: Route,
        #[n(2)] payload: Vec<u8>,
    }
}
impl Encodable for PunchMessage {
    fn encode(self) -> Result<Vec<u8>> {
        Ok(minicbor::to_vec(self)?)
    }
}

impl Decodable for PunchMessage {
    fn decode(data: &[u8]) -> Result<Self> {
        Ok(minicbor::decode(data)?)
    }
}

impl Message for PunchMessage {}
