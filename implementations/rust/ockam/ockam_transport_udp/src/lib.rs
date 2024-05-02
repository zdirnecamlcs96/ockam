//! This crate provides a UDP Transport for Ockam's Routing Protocol.
//!
#![deny(unsafe_code)]
#![warn(
    missing_docs,
    dead_code,
    trivial_casts,
    trivial_numeric_casts,
    unused_import_braces,
    unused_qualifications
)]
#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "std")]
extern crate core;

#[cfg(feature = "alloc")]
extern crate alloc;

mod hole_puncher;
mod options;
mod rendezvous_service;
mod transport;

mod workers;

pub use hole_puncher::*;
pub use options::UdpBindOptions;
pub use rendezvous_service::UdpRendezvousService;
pub use transport::UdpTransportExtension;
pub use transport::{UdpBindArguments, UdpTransport};

pub(crate) const CLUSTER_NAME: &str = "_internals.transport.udp";

/// Transport type for UDP addresses
pub const UDP: ockam_core::TransportType = ockam_core::TransportType::new(2);
