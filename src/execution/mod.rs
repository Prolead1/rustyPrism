//! End-to-end execution layer connecting the Smart Order Router to trading
//! venues over FIX.
//!
//! The [`gateway`] module defines the [`gateway::VenueGateway`] boundary and a
//! FIX-backed simulated venue. The [`executor`] module wires a router to a set
//! of gateways and closes the market-data feedback loop.

pub mod executor;
pub mod gateway;
pub mod sor_pipeline;

pub use executor::{IntegratedReport, IntegratedRouter};
pub use gateway::{ChildOrder, FixVenueGateway, OrderStatus, VenueExecution, VenueGateway};
pub use sor_pipeline::{
    CompactReport, DirectSorPipeline, ShardSorProducer, ShardedSorPipeline, SorCommand,
    SorPipeline, SorProcessor, SubmitError,
};
