//! The Disruptor pipeline: ingress queue → ingester thread → SPSC ring →
//! core handler thread.
//!
//! This separates two concerns that LMAX calls the *upstream* and the *business
//! logic*:
//!
//! 1. **Ingress** is a bounded, lock-free multi-producer queue
//!    ([`crossbeam_queue::ArrayQueue`]). Network/FIX threads may publish
//!    concurrently; when it fills, producers feel backpressure instead of
//!    unbounded memory growth.
//! 2. **Ingester** is a single thread that drains the ingress queue in batches,
//!    translates raw input into typed commands, and publishes them into the
//!    ring.
//! 3. **Core** is a single thread that consumes the ring in strict publication
//!    order and invokes the [`EventHandler`], also in batches. Running the
//!    business logic on one thread keeps it deterministic and data-race free.
//!
//! Both threads wait with a configurable [`WaitStrategy`] instead of parking,
//! which removes the scheduler wake-up latency that dominated the earlier
//! `std::sync::mpsc` implementation. When `pin_threads` is enabled the ingester
//! and core are also assigned to dedicated cores (best-effort; see
//! [`available_core_ids`]).

use super::ring::{spsc, Consumer, Producer};
use core_affinity::CoreId;
use crossbeam_queue::ArrayQueue;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

/// Business logic invoked on the core thread.
pub trait EventHandler<E>: Send {
    /// Handle a single command.
    fn on_event(&mut self, event: &E);

    /// Handle a batch of commands in publication order.
    ///
    /// The default simply forwards to [`EventHandler::on_event`]; handlers that
    /// can amortise work across a batch should override it.
    fn on_batch(&mut self, events: &[E]) {
        for event in events {
            self.on_event(event);
        }
    }
}

/// How idle threads wait for work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitStrategy {
    /// Spin without yielding: lowest latency and tail, burns a core.
    BusySpin,
    /// Spin briefly, then yield: lower CPU, higher scheduling tail.
    Yield,
}

/// Pipeline sizing and runtime policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineConfig {
    /// SPSC ring capacity; rounded up to a power of two.
    pub ring_capacity: usize,
    /// Ingress queue capacity (maximum in-flight raw inputs).
    pub input_capacity: usize,
    /// Maximum commands processed per core dispatch / publish per ingest pass.
    pub batch_size: usize,
    /// Wait strategy used by the ingester and core threads.
    pub wait_strategy: WaitStrategy,
    /// Best-effort pinning of the ingester and core to dedicated cores.
    pub pin_threads: bool,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        PipelineConfig {
            ring_capacity: 1024,
            input_capacity: 1024,
            batch_size: 64,
            wait_strategy: WaitStrategy::Yield,
            pin_threads: false,
        }
    }
}

/// Why a publish was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishError<I> {
    /// The ingress queue is full.
    Full(I),
    /// The pipeline is shutting down.
    Closed(I),
}

impl<I> PublishError<I> {
    /// Recover the value that could not be published.
    pub fn into_inner(self) -> I {
        match self {
            PublishError::Full(value) | PublishError::Closed(value) => value,
        }
    }
}

/// Cloneable, multi-producer ingress handle.
pub struct IngressSender<I> {
    queue: Arc<ArrayQueue<I>>,
    closed: Arc<AtomicBool>,
    wait_strategy: WaitStrategy,
}

impl<I> Clone for IngressSender<I> {
    fn clone(&self) -> Self {
        IngressSender {
            queue: Arc::clone(&self.queue),
            closed: Arc::clone(&self.closed),
            wait_strategy: self.wait_strategy,
        }
    }
}

impl<I> IngressSender<I> {
    /// Publish, spinning with backoff while the queue is full.
    pub fn send(&self, input: I) -> Result<(), PublishError<I>> {
        let mut input = input;
        let mut spins = 0u32;
        loop {
            if self.closed.load(Ordering::Acquire) {
                return Err(PublishError::Closed(input));
            }
            match self.queue.push(input) {
                Ok(()) => return Ok(()),
                Err(returned) => {
                    input = returned;
                    wait(self.wait_strategy, &mut spins);
                }
            }
        }
    }

    /// Publish without blocking.
    pub fn try_send(&self, input: I) -> Result<(), PublishError<I>> {
        if self.closed.load(Ordering::Acquire) {
            return Err(PublishError::Closed(input));
        }
        self.queue.push(input).map_err(PublishError::Full)
    }

    /// Number of inputs currently queued.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

/// Handle to a running pipeline. Clone [`sender`](Self::sender) to fan in from
/// multiple producer threads.
pub struct PipelineHandle<I> {
    sender: IngressSender<I>,
    ingester: Option<JoinHandle<()>>,
    core: Option<JoinHandle<()>>,
}

impl<I> PipelineHandle<I> {
    /// Cloneable ingress sender for another producer thread.
    pub fn sender(&self) -> IngressSender<I> {
        self.sender.clone()
    }

    /// Publish raw input, blocking while the ingress queue is full.
    pub fn publish(&self, input: I) -> Result<(), PublishError<I>> {
        self.sender.send(input)
    }

    /// Publish raw input without blocking.
    pub fn try_publish(&self, input: I) -> Result<(), PublishError<I>> {
        self.sender.try_send(input)
    }

    /// Close ingress and wait for both threads to drain and exit.
    ///
    /// Call only after all producer threads have stopped sending.
    pub fn shutdown(mut self) {
        self.sender.closed.store(true, Ordering::Release);
        if let Some(ingester) = self.ingester.take() {
            let _ = ingester.join();
        }
        if let Some(core) = self.core.take() {
            let _ = core.join();
        }
    }
}

/// Number of logical cores the affinity layer can see (0 if unsupported).
pub fn available_core_ids() -> usize {
    core_affinity::get_core_ids()
        .map(|ids| ids.len())
        .unwrap_or(0)
}

/// Spawn a pipeline with the given configuration, core handler and translator.
pub fn spawn<I, C, H, T>(config: PipelineConfig, handler: H, mut translate: T) -> PipelineHandle<I>
where
    I: Send + 'static,
    C: Send + 'static,
    H: EventHandler<C> + 'static,
    T: FnMut(I) -> C + Send + 'static,
{
    let queue = Arc::new(ArrayQueue::new(config.input_capacity.max(1)));
    let closed = Arc::new(AtomicBool::new(false));
    let (producer, consumer) = spsc::<Option<C>>(config.ring_capacity.max(2));

    let (ingester_core, core_core) = pinning_plan(config.pin_threads);
    let strategy = config.wait_strategy;
    let batch_size = config.batch_size.max(1);

    let ingester_queue = Arc::clone(&queue);
    let ingester_closed = Arc::clone(&closed);
    let ingester = thread::Builder::new()
        .name("disruptor-ingest".to_string())
        .spawn(move || {
            ingest_loop(
                ingester_queue,
                ingester_closed,
                producer,
                &mut translate,
                strategy,
                batch_size,
                ingester_core,
            )
        })
        .expect("failed to spawn ingester thread");

    let core = thread::Builder::new()
        .name("disruptor-core".to_string())
        .spawn(move || core_loop(consumer, handler, strategy, batch_size, core_core))
        .expect("failed to spawn core thread");

    PipelineHandle {
        sender: IngressSender {
            queue,
            closed,
            wait_strategy: strategy,
        },
        ingester: Some(ingester),
        core: Some(core),
    }
}

/// Pick dedicated cores for the ingester and core if the platform supports it.
fn pinning_plan(enabled: bool) -> (Option<CoreId>, Option<CoreId>) {
    if !enabled {
        return (None, None);
    }
    match core_affinity::get_core_ids() {
        Some(cores) if !cores.is_empty() => {
            let count = cores.len();
            // Leave core 0 for the OS and producers.
            let ingester = cores[1 % count];
            let core = cores[(2 % count).max(1).min(count - 1)];
            (Some(ingester), Some(core))
        }
        _ => (None, None),
    }
}

/// Drain ingress in batches, translate, and publish to the ring until closed.
fn ingest_loop<I, C, T>(
    ingress: Arc<ArrayQueue<I>>,
    closed: Arc<AtomicBool>,
    producer: Producer<Option<C>>,
    translate: &mut T,
    strategy: WaitStrategy,
    batch_size: usize,
    pin: Option<CoreId>,
) where
    I: Send + 'static,
    C: Send + 'static,
    T: FnMut(I) -> C,
{
    if let Some(core) = pin {
        core_affinity::set_for_current(core);
    }

    let mut commands: Vec<C> = Vec::with_capacity(batch_size);
    let mut spins = 0u32;
    loop {
        if closed.load(Ordering::Acquire) && ingress.is_empty() {
            break;
        }
        commands.clear();
        match pop_waiting(&ingress, &closed, strategy, &mut spins) {
            Some(input) => commands.push(translate(input)),
            None => break,
        }
        while commands.len() < batch_size {
            match ingress.pop() {
                Some(input) => commands.push(translate(input)),
                None => break,
            }
        }
        for command in commands.drain(..) {
            publish_waiting(&producer, Some(command), strategy);
        }
    }
    // Ordered shutdown sentinel: the core sees it only after every preceding
    // command has been consumed.
    publish_waiting(&producer, None, strategy);
}

/// Block (spinning) until an item is available or the pipeline is closed.
fn pop_waiting<I>(
    ingress: &ArrayQueue<I>,
    closed: &AtomicBool,
    strategy: WaitStrategy,
    spins: &mut u32,
) -> Option<I> {
    loop {
        if let Some(input) = ingress.pop() {
            return Some(input);
        }
        if closed.load(Ordering::Acquire) && ingress.is_empty() {
            return None;
        }
        wait(strategy, spins);
    }
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

/// Consume the ring in batches and dispatch to the handler until the sentinel.
fn core_loop<C, H>(
    consumer: Consumer<Option<C>>,
    mut handler: H,
    strategy: WaitStrategy,
    batch_size: usize,
    pin: Option<CoreId>,
) where
    C: Send + 'static,
    H: EventHandler<C>,
{
    if let Some(core) = pin {
        core_affinity::set_for_current(core);
    }

    let mut batch: Vec<C> = Vec::with_capacity(batch_size);
    let mut spins = 0u32;
    loop {
        batch.clear();

        // Wait for the first command, or the shutdown sentinel.
        let first = loop {
            match consumer.try_consume() {
                Some(Some(command)) => break Some(command),
                Some(None) => break None,
                None => wait(strategy, &mut spins),
            }
        };
        let Some(command) = first else {
            break;
        };
        batch.push(command);

        // Drain up to a batch without waiting.
        let mut shutdown = false;
        while batch.len() < batch_size {
            match consumer.try_consume() {
                Some(Some(command)) => batch.push(command),
                Some(None) => {
                    shutdown = true;
                    break;
                }
                None => break,
            }
        }

        handler.on_batch(&batch);
        if shutdown {
            break;
        }
    }
}

/// Spin briefly, then yield, to balance latency against CPU burn.
#[inline]
pub(super) fn wait(strategy: WaitStrategy, spins: &mut u32) {
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

impl<I> fmt::Debug for IngressSender<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IngressSender")
            .field("len", &self.queue.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc as StdArc, Mutex};

    struct Recorder {
        seen: StdArc<Mutex<Vec<u64>>>,
    }

    impl EventHandler<u64> for Recorder {
        fn on_event(&mut self, event: &u64) {
            self.seen.lock().unwrap().push(*event);
        }
    }

    #[test]
    fn test_pipeline_processes_in_order() {
        let seen = StdArc::new(Mutex::new(Vec::new()));
        let handler = Recorder {
            seen: StdArc::clone(&seen),
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
    fn test_batching_dispatches_in_order() {
        #[derive(Default)]
        struct BatchRecorder {
            seen: StdArc<Mutex<Vec<u64>>>,
            batch_sizes: StdArc<Mutex<Vec<usize>>>,
        }
        impl EventHandler<u64> for BatchRecorder {
            fn on_event(&mut self, event: &u64) {
                self.seen.lock().unwrap().push(*event);
            }
            fn on_batch(&mut self, events: &[u64]) {
                self.batch_sizes.lock().unwrap().push(events.len());
                for event in events {
                    self.seen.lock().unwrap().push(*event);
                }
            }
        }

        let recorder = BatchRecorder::default();
        let seen = StdArc::clone(&recorder.seen);
        let batch_sizes = StdArc::clone(&recorder.batch_sizes);
        let handle = spawn(
            PipelineConfig {
                batch_size: 32,
                ..PipelineConfig::default()
            },
            recorder,
            |value: u64| value,
        );
        for value in 0..1_000 {
            handle.publish(value).unwrap();
        }
        handle.shutdown();

        assert!(seen.lock().unwrap().iter().copied().eq(0..1_000));
        let sizes = batch_sizes.lock().unwrap();
        assert_eq!(sizes.iter().sum::<usize>(), 1_000);
        assert!(sizes.iter().any(|&size| size > 1), "no batching observed");
    }

    #[test]
    fn test_pipeline_translates_input() {
        struct Sum {
            total: StdArc<Mutex<u64>>,
        }
        impl EventHandler<u64> for Sum {
            fn on_event(&mut self, event: &u64) {
                *self.total.lock().unwrap() += *event;
            }
        }

        let total = StdArc::new(Mutex::new(0));
        let handle = spawn(
            PipelineConfig {
                ring_capacity: 16,
                input_capacity: 16,
                ..PipelineConfig::default()
            },
            Sum {
                total: StdArc::clone(&total),
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
        let seen = StdArc::new(Mutex::new(Vec::new()));
        let handle = spawn(
            PipelineConfig {
                ring_capacity: 256,
                input_capacity: 256,
                ..PipelineConfig::default()
            },
            Recorder {
                seen: StdArc::clone(&seen),
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
        let mut sorted = seen.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 1_000);
    }

    #[test]
    fn test_backpressure_does_not_drop() {
        struct Counter(StdArc<Mutex<usize>>);
        impl EventHandler<u64> for Counter {
            fn on_event(&mut self, _event: &u64) {
                *self.0.lock().unwrap() += 1;
            }
        }

        let count = StdArc::new(Mutex::new(0));
        let handle = spawn(
            PipelineConfig {
                ring_capacity: 2,
                input_capacity: 2,
                batch_size: 1,
                ..PipelineConfig::default()
            },
            Counter(StdArc::clone(&count)),
            |value: u64| value,
        );
        for value in 0..5_000 {
            handle.publish(value).unwrap();
        }
        handle.shutdown();
        assert_eq!(*count.lock().unwrap(), 5_000);
    }

    #[test]
    fn test_closed_publish_is_rejected() {
        let handle = spawn(
            PipelineConfig::default(),
            Recorder {
                seen: StdArc::new(Mutex::new(Vec::new())),
            },
            |value: u64| value,
        );
        let sender = handle.sender();
        handle.shutdown();
        assert!(matches!(sender.try_send(1), Err(PublishError::Closed(1))));
    }
}
