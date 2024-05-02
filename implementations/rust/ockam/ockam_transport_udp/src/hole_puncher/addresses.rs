use ockam_core::Address;

#[derive(Clone, Debug)]
/// FIXME
pub struct Addresses {
    heartbeat_address: Address,
    remote_address: Address,
    sender_address: Address,
    receiver_address: Address,
}

impl Addresses {
    pub(crate) fn generate(puncher_name: &str) -> Self {
        let heartbeat_address =
            Address::random_tagged(&format!("UdpHolePuncher.main.{}", puncher_name));
        let remote_address =
            Address::random_tagged(&format!("UdpHolePuncher.remote.{}", puncher_name));
        let sender_address =
            Address::random_tagged(&format!("UdpHolePuncher.sender.{}", puncher_name));
        let receiver_address =
            Address::random_tagged(&format!("UdpHolePuncher.receiver.{}", puncher_name));

        Self {
            heartbeat_address,
            remote_address,
            sender_address,
            receiver_address,
        }
    }
    /// FIXME
    pub fn heartbeat_address(&self) -> &Address {
        &self.heartbeat_address
    }
    /// FIXME
    pub fn remote_address(&self) -> &Address {
        &self.remote_address
    }
    /// FIXME
    pub fn sender_address(&self) -> &Address {
        &self.sender_address
    }
    /// FIXME
    pub fn receiver_address(&self) -> &Address {
        &self.receiver_address
    }
}
