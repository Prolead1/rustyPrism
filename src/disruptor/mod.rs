//! LMAX Disruptor-style concurrency primitives.
//!
//! The core idea is a staged pipeline that keeps business logic on one
//! deterministic thread while ingress absorbs load from one or many producers.
//! Three topologies are provided, all built on the same lock-free SPSC ring:
//!
//! ```text
//!   direct   caller ──────────────────────► command ring ──► core (SOR)
//!              ▲                                                 │
//!              └────────────── result ring ◄─────────────────────┘
//!
//!   sharded  P0 ─► cmd ring 0 ─┐                    ┌─► result ring 0 ─► P0
//!            P1 ─► cmd ring 1 ─┼─► core (SOR) ──────┼─► result ring 1 ─► P1
//!            P2 ─► cmd ring 2 ─┘                    └─► result ring 2 ─► P2
//!
//!   fan-in   P0 ─┐
//!            P1 ─┼─► MPSC ingress ─► ingester ─► ring ─► core
//!            P2 ─┘
//! ```
//!
//! * [`ring`] — the pre-allocated, cache-padded, lock-free SPSC ring buffer.
//! * [`direct`] — single producer, one hop, allocation-free.
//! * [`sharded`] — one SPSC ring pair per producer, merged by a single core;
//!   one hop each, no cross-producer contention.
//! * [`pipeline`] — lock-free MPSC ingress into a single ring, for many
//!   producers when a single ordered stream (and multiple consumers) is wanted.
//!
//! All topologies are generic over the command and result types and share the
//! [`Handler`] trait, so the exchange side can reuse them.

pub mod direct;
pub mod pipeline;
pub mod ring;
pub mod sharded;

pub use direct::{DirectConfig, DirectPipeline};
pub use pipeline::{
    available_core_ids, spawn, EventHandler, IngressSender, PipelineConfig, PipelineHandle,
    PublishError, WaitStrategy,
};
pub use ring::{spsc, Consumer, Producer};
pub use sharded::{spawn_sharded, ShardConfig, ShardProducer, ShardedPipeline};

/// Single-item business logic shared by the direct and sharded topologies.
pub trait Handler<I, O>: Send {
    /// Consume one input and produce its output.
    fn handle(&mut self, input: I) -> O;
}
