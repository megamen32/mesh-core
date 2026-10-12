//! Common mesh implementation extracted from GrepMesh.
//! Consumers retain authorization, their tool names and local business state.
pub mod topology;
pub mod topology_cache;
pub mod request;

#[cfg(feature = "transport")]
pub mod transport;
