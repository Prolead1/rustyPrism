//! A direct, single-producer pipeline for the lowest possible latency.
//!
//! The fan-in [`super::pipeline`] topology pays for concurrency: a command
//! travels caller → ingester thread → ring → core thread (two thread hops), and
//! the reply comes back through a channel. When there is a single producer
//! thread — a feed handler, a strategy, one gateway session — that ingester hop
//! is pure overhead.
//!
//! [`DirectPipeline`] removes it. The caller produces straight into an SPSC
//! command ring and the core consumes it, so a synchronous round trip is a
//! single caller↔core exchange. The result comes back over a second SPSC ring,
//! so there is no channel allocation and no parking on either side.
//!
//! ```text
//!   caller ──► command ring ──► core (handler)
//!      ▲                            │
//!      └──────── result ring ───────┘
//! ```
//!
//! [`DirectPipeline`] holds the [`Producer`](super::ring::Producer)/
//! [`Consumer`](super::ring::Consumer) halves directly, so it is `!Sync` and the
//! single-producer guarantee is enforced by the type system.

use super::pipeline::wait;
use super::pipeline::WaitStrategy;
use super::ring::{spsc, Consumer, Producer};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

/// Business logic for the direct pipeline: consumes an input, returns an output.
pub trait DirectHandler<I, O>: Send {
    fn handle(&mut self, input: I) -> O;
}

/// Sizing and wait policy for a [`DirectPipeline`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectConfig {
    pub command_capacity: usize,
    pub result_capacity: usize,
    pub wait_strategy: WaitStrategy,
}

impl Default for DirectConfig {
    fn default() -> Self {
        DirectConfig {
            command_capacity: 1024,
            result_capacity: 1024,
            wait_strategy: WaitStrategy::BusySpin,
        }
    }
}

/// A single-producer, single-consumer, synchronous request/response pipeline.
pub struct DirectPipeline<I, O> {
    commands: Producer<I>,
    results: Consumer<O>,
    closed: Arc<AtomicBool>,
    wait_strategy: WaitStrategy,
    core: Option<JoinHandle<()>>,
}

impl<I, O> DirectPipeline<I, O>
where
    I: Send + 'static,
    O: Send + 'static,
{
    /// Spawn the core thread and return the caller-side handle.
    pub fn spawn<H>(config: DirectConfig, handler: H) -> Self
    where
        H: DirectHandler<I, O> + 'static,
    {
        let (command_producer, command_consumer) = spsc::<I>(config.command_capacity.max(2));
        let (result_producer, result_consumer) = spsc::<O>(config.result_capacity.max(2));
        let closed = Arc::new(AtomicBool::new(false));
        let core_closed = Arc::clone(&closed);
        let strategy = config.wait_strategy;

        let core = thread::Builder::new()
            .name("direct-core".to_string())
            .spawn(move || {
                direct_core_loop(
                    command_consumer,
                    result_producer,
                    handler,
                    core_closed,
                    strategy,
                )
            })
            .expect("failed to spawn direct core thread");

        DirectPipeline {
            commands: command_producer,
            results: result_consumer,
            closed,
            wait_strategy: strategy,
            core: Some(core),
        }
    }

    /// Publish an input and spin for its output.
    pub fn submit(&self, input: I) -> O {
        self.commands.publish(input);
        let mut spins = 0u32;
        loop {
            if let Some(output) = self.results.try_consume() {
                return output;
            }
            wait(self.wait_strategy, &mut spins);
        }
    }

    /// Stop the core once all in-flight work has drained.
    pub fn shutdown(mut self) {
        self.closed.store(true, Ordering::Release);
        if let Some(core) = self.core.take() {
            let _ = core.join();
        }
    }
}

fn direct_core_loop<I, O, H>(
    commands: Consumer<I>,
    results: Producer<O>,
    mut handler: H,
    closed: Arc<AtomicBool>,
    strategy: WaitStrategy,
) where
    I: Send + 'static,
    O: Send + 'static,
    H: DirectHandler<I, O>,
{
    let mut spins = 0u32;
    loop {
        match commands.try_consume() {
            Some(input) => {
                let output = handler.handle(input);
                results.publish(output);
                spins = 0;
            }
            None => {
                if closed.load(Ordering::Acquire) {
                    break;
                }
                wait(strategy, &mut spins);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Doubler;

    impl DirectHandler<u64, u64> for Doubler {
        fn handle(&mut self, input: u64) -> u64 {
            input * 2
        }
    }

    #[test]
    fn test_direct_round_trip() {
        let pipeline = DirectPipeline::spawn(DirectConfig::default(), Doubler);
        for value in 0..1_000u64 {
            assert_eq!(pipeline.submit(value), value * 2);
        }
        pipeline.shutdown();
    }

    #[test]
    fn test_direct_preserves_state_and_order() {
        struct Accumulator {
            running: u64,
        }
        impl DirectHandler<u64, u64> for Accumulator {
            fn handle(&mut self, input: u64) -> u64 {
                self.running += input;
                self.running
            }
        }

        let pipeline = DirectPipeline::spawn(DirectConfig::default(), Accumulator { running: 0 });
        let mut expected = 0;
        for value in 1..=100u64 {
            expected += value;
            assert_eq!(pipeline.submit(value), expected);
        }
        pipeline.shutdown();
    }
}
