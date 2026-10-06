//! The Disruptor pipeline: ingress queue → ingester thread → SPSC ring →
//! core handler thread.
//!
//! The pipeline separates two concerns that LMAX calls the *upstream* and the
//! *business logic*:
//!
//! 1. **Ingress** is a bounded multi-producer channel. Network/FIX threads may
//!    publish raw input concurrently; when it fills, producers feel
//!    backpressure instead of unbounded memory growth.
//! 2. **Ingester** is a single thread that drains the ingress queue, translates
//!    raw input into typed commands, and publishes them into the ring buffer.
//! 3. **Core** is a single thread that consumes the ring in strict publication
//!    order and invokes the [`EventHandler`]. Running the business logic on one
//!    thread keeps it deterministic and free of data races.
//!
//! The ring is a [`super::ring`] lock-free SPSC buffer: pre-allocated, padded
//! and allocation-free on the hot path.
//!
//! The same `spawn` works for any command/handler pair, so a future exchange
//! process can reuse the topology with its own handler.

use super::ring::{spsc, Consumer, Producer};
use std::sync::mpsc::{sync_channel, Receiver, SendError, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};

/// Business logic invoked on the core thread for every command, in order.
pub trait EventHandler<E>: Send {
    fn on_event(&mut self, event: &E);
}

/// How idle threads wait for the other side of the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitStrategy {
    /// Spin without yielding: lowest latency and tail, burns a core.
    BusySpin,
    /// Spin briefly, then yield: lower CPU, higher scheduling tail.
    Yield,
}

/// Pipeline sizing and wait policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineConfig {
    /// SPSC ring capacity; rounded up to a power of two.
    pub ring_capacity: usize,
    /// Ingress channel capacity (maximum in-flight raw inputs).
    pub input_capacity: usize,
    /// Wait strategy used by the ingester and core threads.
    pub wait_strategy: WaitStrategy,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        PipelineConfig {
            ring_capacity: 1024,
            input_capacity: 1024,
            wait_strategy: WaitStrategy::Yield,
        }
    }
}

/// Handle to a running pipeline. Clone its [`sender`](Self::sender) to fan in
/// from multiple producer threads.
pub struct PipelineHandle<I> {
    input: Option<SyncSender<I>>,
    ingester: Option<JoinHandle<()>>,
    core: Option<JoinHandle<()>>,
}

impl<I: Send + 'static> PipelineHandle<I> {
    /// Clone the ingress sender for another producer thread.
    pub fn sender(&self) -> SyncSender<I> {
        self.input
            .as_ref()
            .expect("pipeline already shut down")
            .clone()
    }

    /// Publish raw input, blocking while the ingress queue is full.
    pub fn publish(&self, input: I) -> Result<(), SendError<I>> {
        self.input
            .as_ref()
            .expect("pipeline already shut down")
            .send(input)
    }

    /// Publish raw input without blocking; returns it back if the queue is full.
    pub fn try_publish(&self, input: I) -> Result<(), TrySendError<I>> {
        self.input
            .as_ref()
            .expect("pipeline already shut down")
            .try_send(input)
    }

    /// Close the ingress queue and wait for both threads to drain and exit.
    pub fn shutdown(mut self) {
        // Dropping the sender closes the ingress channel, which causes the
        // ingester to publish the shutdown sentinel and terminate. The core
        // then drains the ring and exits.
        self.input = None;
        if let Some(ingester) = self.ingester.take() {
            let _ = ingester.join();
        }
        if let Some(core) = self.core.take() {
            let _ = core.join();
        }
    }
}

/// Spawn a pipeline with the given configuration, core handler and translator.
///
/// `translate` runs on the ingester thread and turns each raw input into a
/// typed command. The handler runs on the core thread in publication order.
pub fn spawn<I, C, H, T>(config: PipelineConfig, handler: H, mut translate: T) -> PipelineHandle<I>
where
    I: Send + 'static,
    C: Send + 'static,
    H: EventHandler<C> + 'static,
    T: FnMut(I) -> C + Send + 'static,
{
    let (input_tx, input_rx) = sync_channel::<I>(config.input_capacity.max(1));
    let (producer, consumer) = spsc::<Option<C>>(config.ring_capacity.max(2));

    let wait_strategy = config.wait_strategy;
    let ingester = thread::Builder::new()
        .name("disruptor-ingest".to_string())
        .spawn(move || ingest_loop(input_rx, producer, &mut translate, wait_strategy))
        .expect("failed to spawn ingester thread");

    let core = thread::Builder::new()
        .name("disruptor-core".to_string())
        .spawn(move || core_loop(consumer, handler, wait_strategy))
        .expect("failed to spawn core thread");

    PipelineHandle {
        input: Some(input_tx),
        ingester: Some(ingester),
        core: Some(core),
    }
}

/// Drain ingress, translate, and publish to the ring until the channel closes.
fn ingest_loop<I, C, T>(
    input_rx: Receiver<I>,
    producer: Producer<Option<C>>,
    translate: &mut T,
    strategy: WaitStrategy,
) where
    I: Send + 'static,
    C: Send + 'static,
    T: FnMut(I) -> C,
{
    while let Ok(input) = input_rx.recv() {
        let command = translate(input);
        publish_waiting(&producer, Some(command), strategy);
    }
    // Ordered shutdown sentinel: the core sees it only after every preceding
    // command has been consumed.
    publish_waiting(&producer, None, strategy);
}

/// Publish, honouring the wait strategy while the ring is full.
fn publish_waiting<C>(producer: &Producer<Option<C>>, value: Option<C>, strategy: WaitStrategy)
where
    C: Send + 'static,
{
    let mut value = value;
    let mut spins = 0u32;
    loop {
        match producer.try_publish(value) {
            Ok(()) => return,
            Err(returned) => {
                value = returned;
                wait(strategy, &mut spins);
            }
        }
    }
}

/// Consume the ring in order and dispatch to the handler until the sentinel.
fn core_loop<C, H>(consumer: Consumer<Option<C>>, mut handler: H, strategy: WaitStrategy)
where
    C: Send + 'static,
    H: EventHandler<C>,
{
    let mut spins = 0u32;
    loop {
        match consumer.try_consume() {
            Some(Some(command)) => {
                handler.on_event(&command);
                spins = 0;
            }
            // Shutdown sentinel: the ingester is done and the ring is drained.
            Some(None) => break,
            None => wait(strategy, &mut spins),
        }
    }
}

#[inline]
fn wait(strategy: WaitStrategy, spins: &mut u32) {
    match strategy {
        WaitStrategy::BusySpin => std::hint::spin_loop(),
        WaitStrategy::Yield => {
            if *spins < 128 {
                std::hint::spin_loop();
                *spins += 1;
            } else {
                std::thread::yield_now();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Recorder {
        seen: Arc<Mutex<Vec<u64>>>,
    }

    impl EventHandler<u64> for Recorder {
        fn on_event(&mut self, event: &u64) {
            self.seen.lock().unwrap().push(*event);
        }
    }

    #[test]
    fn test_pipeline_processes_in_order() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let handler = Recorder {
            seen: Arc::clone(&seen),
        };
        let handle = spawn(PipelineConfig::default(), handler, |value: u64| value);

        for value in 0..1_000 {
            handle.publish(value).unwrap();
        }
        handle.shutdown();

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1_000);
        assert!(seen.iter().copied().eq(0..1_000));
    }

    #[test]
    fn test_pipeline_translates_input() {
        struct Sum {
            total: Arc<Mutex<u64>>,
        }
        impl EventHandler<u64> for Sum {
            fn on_event(&mut self, event: &u64) {
                *self.total.lock().unwrap() += *event;
            }
        }

        let total = Arc::new(Mutex::new(0));
        let handle = spawn(
            PipelineConfig {
                ring_capacity: 16,
                input_capacity: 16,
                ..PipelineConfig::default()
            },
            Sum {
                total: Arc::clone(&total),
            },
            |raw: (u64, u64)| raw.0 * raw.1,
        );

        for a in 1..=10u64 {
            handle.publish((a, a)).unwrap();
        }
        handle.shutdown();
        assert_eq!(*total.lock().unwrap(), (1..=10).map(|a| a * a).sum::<u64>());
    }

    #[test]
    fn test_multiple_producers_fan_in() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let handle = spawn(
            PipelineConfig {
                ring_capacity: 256,
                input_capacity: 256,
                ..PipelineConfig::default()
            },
            Recorder {
                seen: Arc::clone(&seen),
            },
            |value: u64| value,
        );

        let mut threads = Vec::new();
        for t in 0..4u64 {
            let sender = handle.sender();
            threads.push(thread::spawn(move || {
                for i in 0..250u64 {
                    sender.send(t * 1_000 + i).unwrap();
                }
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        handle.shutdown();

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1_000);
        // Every value delivered exactly once.
        let mut sorted = seen.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 1_000);
    }

    #[test]
    fn test_backpressure_does_not_drop() {
        struct Counter(Arc<Mutex<usize>>);
        impl EventHandler<u64> for Counter {
            fn on_event(&mut self, _event: &u64) {
                *self.0.lock().unwrap() += 1;
            }
        }

        let count = Arc::new(Mutex::new(0));
        // Tiny ingress and ring to force the blocking path.
        let handle = spawn(
            PipelineConfig {
                ring_capacity: 2,
                input_capacity: 2,
                ..PipelineConfig::default()
            },
            Counter(Arc::clone(&count)),
            |value: u64| value,
        );
        for value in 0..5_000 {
            handle.publish(value).unwrap();
        }
        handle.shutdown();
        assert_eq!(*count.lock().unwrap(), 5_000);
    }
}
