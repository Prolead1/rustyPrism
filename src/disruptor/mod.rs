//! LMAX Disruptor-style concurrency primitives.
//!
//! The core idea is a staged, single-writer pipeline that keeps business logic
//! on one deterministic thread while ingress absorbs bursts from many
//! producers:
//!
//! ```text
//!   producers ──► ingress queue ──► ingester thread ──► SPSC ring ──► core thread
//!   (network)     (bounded MPSC)     (translate)        (lock-free)    (handler)
//! ```
//!
//! * [`ring`] is a pre-allocated, cache-padded, lock-free SPSC ring buffer —
//!   the Disruptor's ring, with no locks and no steady-state allocation.
//! * [`pipeline`] wires an ingress channel, an ingester thread and a core
//!   handler into a reusable pipeline via [`pipeline::spawn`].
//!
//! The topology is generic over command type and handler, so the exchange side
//! can adopt the same structure with its own [`pipeline::EventHandler`].

pub mod pipeline;
pub mod ring;

pub use pipeline::{spawn, EventHandler, PipelineConfig, PipelineHandle, WaitStrategy};
pub use ring::{spsc, Consumer, Producer};
