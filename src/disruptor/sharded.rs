//! Sharded ingress: one SPSC ring pair per producer, merged by a single core.
//!
//! The fan-in [`super::pipeline`] funnels every producer through an MPSC queue
//! and an ingester thread (two hops). The [`super::direct`] pipeline is a single
//! hop but only supports one producer. This module keeps the single hop *and*
//! supports many producers by giving each producer its own SPSC command ring and
//! its own SPSC result ring, then having the core poll them:
//!
//! ```text
//!   P0 ─► cmd ring 0 ─┐                       ┌─► result ring 0 ─► P0
//!   P1 ─► cmd ring 1 ─┼─► core (handler) ─────┼─► result ring 1 ─► P1
//!   P2 ─► cmd ring 2 ─┘                       └─► result ring 2 ─► P2
//! ```
//!
//! Properties:
//!
//! * Each producer writes to its own ring: **one hop, no CAS contention**.
//! * A slow producer cannot block the others (no shared head-of-line).
//! * Each producer's own commands are processed in order; there is deliberately
//!   **no global order across producers**. The core scans shards in id order
//!   each pass, taking one item per ring, which bounds starvation.
//! * Each producer's [`ShardProducer`] is `!Sync`, so a ring still has exactly
//!   one producer thread — the invariant the lock-free ring relies on.
//!
//! For a single producer prefer [`super::direct`]; sharded adds a vector of
//! rings to poll and is intended for a handful of producers.

use super::pipeline::{wait, WaitStrategy};
use super::ring::{spsc, Consumer, Producer};
use super::Handler;
use core_affinity::CoreId;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

/// Sizing and wait policy for a [`ShardedPipeline`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShardConfig {
    pub command_capacity: usize,
    pub result_capacity: usize,
    pub wait_strategy: WaitStrategy,
    /// Pin the core thread to a dedicated core (best-effort).
    pub pin_core: bool,
}

impl Default for ShardConfig {
    fn default() -> Self {
        ShardConfig {
            command_capacity: 1024,
            result_capacity: 1024,
            wait_strategy: WaitStrategy::BusySpin,
            pin_core: false,
        }
    }
}

/// Producer-side handle for one shard. Move one handle to each producer thread.
pub struct ShardProducer<I, O> {
    id: usize,
    commands: Producer<I>,
    results: Consumer<O>,
    wait_strategy: WaitStrategy,
}

impl<I, O> ShardProducer<I, O>
where
    I: Send + 'static,
    O: Send + 'static,
{
    /// Shard index, stable for the life of the pipeline.
    pub fn id(&self) -> usize {
        self.id
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

    /// Best-effort pinning of the current thread to `core`.
    ///
    /// Call from the producer thread itself.
    pub fn pin_to(&self, core: CoreId) -> bool {
        core_affinity::set_for_current(core)
    }
}

/// Core-side owner of the sharded pipeline.
pub struct ShardedPipeline {
    shard_count: usize,
    closed: Arc<AtomicBool>,
    core: Option<JoinHandle<()>>,
}

impl ShardedPipeline {
    /// Number of shards in this pipeline.
    pub fn shard_count(&self) -> usize {
        self.shard_count
    }

    /// Stop the core once all producers have stopped submitting.
    ///
    /// All [`ShardProducer`] handles must be dropped before calling this.
    pub fn shutdown(mut self) {
        self.closed.store(true, Ordering::Release);
        if let Some(core) = self.core.take() {
            let _ = core.join();
        }
    }
}

/// Spawn a sharded pipeline with `num_producers` shards.
///
/// Returns the core owner and one producer handle per shard. Distribute the
/// handles across your producer threads (one each).
pub fn spawn_sharded<I, O, H>(
    config: ShardConfig,
    handler: H,
    num_producers: usize,
) -> (ShardedPipeline, Vec<ShardProducer<I, O>>)
where
    I: Send + 'static,
    O: Send + 'static,
    H: Handler<I, O> + 'static,
{
    let num = num_producers.max(1);
    let mut command_consumers: Vec<Consumer<I>> = Vec::with_capacity(num);
    let mut result_producers: Vec<Producer<O>> = Vec::with_capacity(num);
    let mut producers: Vec<ShardProducer<I, O>> = Vec::with_capacity(num);

    for id in 0..num {
        let (command_producer, command_consumer) = spsc::<I>(config.command_capacity.max(2));
        let (result_producer, result_consumer) = spsc::<O>(config.result_capacity.max(2));
        command_consumers.push(command_consumer);
        result_producers.push(result_producer);
        producers.push(ShardProducer {
            id,
            commands: command_producer,
            results: result_consumer,
            wait_strategy: config.wait_strategy,
        });
    }

    let closed = Arc::new(AtomicBool::new(false));
    let core_closed = Arc::clone(&closed);
    let strategy = config.wait_strategy;
    let pin_core = config.pin_core;
    let core = thread::Builder::new()
        .name("sharded-core".to_string())
        .spawn(move || {
            sharded_core_loop(
                command_consumers,
                result_producers,
                handler,
                core_closed,
                strategy,
                pin_core,
            )
        })
        .expect("failed to spawn sharded core thread");

    (
        ShardedPipeline {
            shard_count: num,
            closed,
            core: Some(core),
        },
        producers,
    )
}

fn sharded_core_loop<I, O, H>(
    commands: Vec<Consumer<I>>,
    results: Vec<Producer<O>>,
    mut handler: H,
    closed: Arc<AtomicBool>,
    strategy: WaitStrategy,
    pin_core: bool,
) where
    I: Send + 'static,
    O: Send + 'static,
    H: Handler<I, O>,
{
    if pin_core {
        if let Some(cores) = core_affinity::get_core_ids() {
            if let Some(core) = cores.first() {
                core_affinity::set_for_current(*core);
            }
        }
    }

    let shards = commands.len();
    let mut spins = 0u32;
    loop {
        let mut did_work = false;
        // One item per shard per pass keeps producers from starving each other.
        for index in 0..shards {
            if let Some(input) = commands[index].try_consume() {
                let output = handler.handle(input);
                results[index].publish(output);
                did_work = true;
            }
        }
        if did_work {
            spins = 0;
            continue;
        }
        if closed.load(Ordering::Acquire) && commands.iter().all(|shard| shard.is_empty()) {
            break;
        }
        wait(strategy, &mut spins);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Doubler;

    impl Handler<u64, u64> for Doubler {
        fn handle(&mut self, input: u64) -> u64 {
            input * 2
        }
    }

    #[test]
    fn test_sharded_round_trip() {
        let (core, producers) = spawn_sharded(ShardConfig::default(), Doubler, 3);
        assert_eq!(producers.len(), 3);
        for producer in &producers {
            for value in 0..100u64 {
                assert_eq!(producer.submit(value), value * 2);
            }
        }
        drop(producers);
        core.shutdown();
    }

    #[test]
    fn test_shards_run_concurrently_and_are_isolated() {
        struct Echo;
        impl Handler<u64, u64> for Echo {
            fn handle(&mut self, input: u64) -> u64 {
                input
            }
        }

        let (core, producers) = spawn_sharded(ShardConfig::default(), Echo, 4);
        assert_eq!(core.shard_count(), 4);

        let mut threads = Vec::new();
        for producer in producers {
            threads.push(thread::spawn(move || {
                let id = producer.id();
                for i in 0..1_000u64 {
                    assert_eq!(producer.submit(i), i);
                }
                id
            }));
        }
        let mut ids: Vec<usize> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![0, 1, 2, 3]);
        core.shutdown();
    }

    #[test]
    fn test_per_producer_order_is_preserved() {
        struct Sequence(u64);
        impl Handler<u64, u64> for Sequence {
            fn handle(&mut self, input: u64) -> u64 {
                self.0 = input;
                input
            }
        }

        let (core, producers) = spawn_sharded(ShardConfig::default(), Sequence(0), 2);
        for producer in &producers {
            for value in (0..1_000u64).rev() {
                // The result ring is per-producer, so the replies must line up
                // with what this producer submitted.
                assert_eq!(producer.submit(value), value);
            }
        }
        drop(producers);
        core.shutdown();
    }
}
