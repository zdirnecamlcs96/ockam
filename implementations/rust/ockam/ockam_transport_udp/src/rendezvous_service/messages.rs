use minicbor::{Decode, Encode};
use ockam_core::{Decodable, Encodable, Encoded, Message, Result, Route};

// TODO: Change this Request/Response protocol to use CBOR encoding for messages.

/// Request type for UDP Hole Punching Rendezvous service
#[derive(Encode, Decode, Debug)]
#[rustfmt::skip]
pub enum RendezvousRequest {
    /// Update service's internal table with the
    /// details of the sending node.
    #[n(0)] Update {
        /// Name of sending node's puncher
        #[n(0)] puncher_name: String,
    },
    /// Query service's internal table for the public
    /// route to the named node.
    #[n(1)] Query {
        /// Name of puncher to lookup
        #[n(0)] puncher_name: String,
    },
    /// Ping service to see if it is reachable and working.
    #[n(2)] Ping,
    /// Get my public IP and port
    #[n(3)] GetMyAddress,
}

impl Encodable for RendezvousRequest {
    fn encode(self) -> Result<Encoded> {
        Ok(minicbor::to_vec(self)?)
    }
}

impl Decodable for RendezvousRequest {
    fn decode(e: &[u8]) -> Result<Self> {
        Ok(minicbor::decode(e)?)
    }
}

impl Message for RendezvousRequest {}

/// Response type for UDP Hole Punching Rendezvous service
#[derive(Encode, Decode, Debug)]
#[rustfmt::skip]
pub enum RendezvousResponse {
    #[n(0)] Query(#[n(0)] Option<Route>),
    #[n(1)] Pong,
    #[n(2)] GetMyAddress(#[n(0)] String),
}

impl Encodable for RendezvousResponse {
    fn encode(self) -> Result<Encoded> {
        Ok(minicbor::to_vec(self)?)
    }
}

impl Decodable for RendezvousResponse {
    fn decode(e: &[u8]) -> Result<Self> {
        Ok(minicbor::decode(e)?)
    }
}

impl Message for RendezvousResponse {}
