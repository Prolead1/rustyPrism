//! The Smart Order Router hosted on a Disruptor pipeline.
//!
//! Orders arrive on the ingress queue, are translated to [`SorCommand`]s by the
//! ingester thread, published into the ring, and executed on the core thread by
//! [`SorProcessor`]. The router itself stays single-threaded and therefore
//! deterministic, while any number of caller threads can submit concurrently.
//!
//! ```text
//!   caller threads ──► ingress ──► ingester ──► ring ──► core (SmartOrderRouter)
//!          ▲                                                    │
//!          └──────────────── ExecutionReport (reply) ───────────┘
//! ```

use crate::disruptor::{
    spawn_sharded, DirectConfig, DirectPipeline, EventHandler, Handler, IngressSender,
    PipelineConfig, PipelineHandle, PublishError, ShardConfig, ShardProducer, ShardedPipeline,
};
use crate::router::fixed::Fixed;
use crate::router::sor::{ExecutionReport, OrderRequest, SmartOrderRouter};
use crate::router::symbol::SymbolId;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TryRecvError};
use std::sync::Arc;

/// A unit of work for the core: an order plus an optional reply channel.
#[derive(Debug)]
pub struct SorCommand {
    pub request: OrderRequest,
    pub reply: Option<SyncSender<ExecutionReport>>,
}

impl SorCommand {
    /// A command whose result is discarded (highest throughput).
    pub fn fire_and_forget(request: OrderRequest) -> Self {
        SorCommand {
            request,
            reply: None,
        }
    }

    /// A command whose caller waits for the resulting execution report.
    pub fn with_reply(request: OrderRequest, reply: SyncSender<ExecutionReport>) -> Self {
        SorCommand {
            request,
            reply: Some(reply),
        }
    }
}

/// Core-thread handler that owns the single-threaded router.
pub struct SorProcessor {
    router: SmartOrderRouter,
    processed: Arc<AtomicU64>,
}

impl EventHandler<SorCommand> for SorProcessor {
    fn on_event(&mut self, command: &SorCommand) {
        let report = self.router.execute(&command.request);
        self.processed.fetch_add(1, Ordering::Relaxed);
        if let Some(reply) = &command.reply {
            // The caller may have gone away; that is not fatal.
            let _ = reply.send(report);
        }
    }
}

/// Errors a caller can see when submitting to the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitError {
    /// The pipeline has been shut down.
    Closed,
    /// The core dropped the command before replying.
    Dropped,
    /// The ingress queue is full (only from `try_submit`).
    Full,
}

impl fmt::Display for SubmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubmitError::Closed => write!(f, "pipeline is closed"),
            SubmitError::Dropped => write!(f, "core dropped the command"),
            SubmitError::Full => write!(f, "ingress queue is full"),
        }
    }
}

impl std::error::Error for SubmitError {}

impl From<PublishError<SorCommand>> for SubmitError {
    fn from(error: PublishError<SorCommand>) -> Self {
        match error {
            PublishError::Full(_) => SubmitError::Full,
            PublishError::Closed(_) => SubmitError::Closed,
        }
    }
}

/// A running SOR pipeline.
pub struct SorPipeline {
    handle: PipelineHandle<SorCommand>,
    processed: Arc<AtomicU64>,
}

impl SorPipeline {
    /// Spawn the ingester and core threads around `router`.
    pub fn spawn(router: SmartOrderRouter, config: PipelineConfig) -> Self {
        let processed = Arc::new(AtomicU64::new(0));
        let handler = SorProcessor {
            router,
            processed: Arc::clone(&processed),
        };
        let handle = crate::disruptor::spawn(config, handler, |command: SorCommand| command);
        SorPipeline { handle, processed }
    }

    /// Submit an order and wait for the core to return its report.
    ///
    /// The caller spins rather than parking, so the round trip does not pay a
    /// scheduler wake-up on the reply path.
    pub fn submit(&self, request: OrderRequest) -> Result<ExecutionReport, SubmitError> {
        let (reply_tx, reply_rx) = sync_channel(1);
        self.handle
            .publish(SorCommand::with_reply(request, reply_tx))?;
        let mut spins = 0u32;
        loop {
            match reply_rx.try_recv() {
                Ok(report) => return Ok(report),
                Err(TryRecvError::Empty) => {
                    if spins < 1024 {
                        std::hint::spin_loop();
                        spins += 1;
                    } else {
                        std::thread::yield_now();
                    }
                }
                Err(TryRecvError::Disconnected) => return Err(SubmitError::Dropped),
            }
        }
    }

    /// Submit without waiting for the reply (returns once queued).
    pub fn try_submit(&self, request: OrderRequest) -> Result<(), SubmitError> {
        self.handle
            .try_publish(SorCommand::fire_and_forget(request))
            .map_err(SubmitError::from)
    }

    /// Number of commands processed by the core so far.
    pub fn processed(&self) -> u64 {
        self.processed.load(Ordering::Relaxed)
    }

    /// Clone the ingress sender to submit from another thread.
    pub fn sender(&self) -> IngressSender<SorCommand> {
        self.handle.sender()
    }

    /// Submit a raw command through the ingress sender.
    pub fn send(&self, command: SorCommand) -> Result<(), SubmitError> {
        self.handle.publish(command).map_err(SubmitError::from)
    }

    /// Drain and stop the pipeline.
    pub fn shutdown(self) {
        self.handle.shutdown();
    }
}

/// A compact, `Copy` execution summary for the allocation-free direct path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactReport {
    pub requested_qty: Fixed,
    pub filled_qty: Fixed,
    pub avg_price: Fixed,
    pub arrival_price: Fixed,
    pub realized_slippage_millibps: i64,
    pub total_fees: i128,
    pub num_fills: u32,
    pub decision_latency_ns: u64,
}

impl CompactReport {
    pub fn from_report(report: &ExecutionReport) -> Self {
        CompactReport {
            requested_qty: report.requested_qty,
            filled_qty: report.filled_qty,
            avg_price: report.avg_price,
            arrival_price: report.arrival_price,
            realized_slippage_millibps: report.realized_slippage_millibps,
            total_fees: report.total_fees,
            num_fills: report.fills.len() as u32,
            decision_latency_ns: report.decision_latency_ns,
        }
    }

    pub fn unfilled_qty(&self) -> Fixed {
        (self.requested_qty - self.filled_qty).max(Fixed::ZERO)
    }
}

/// Core-thread handler for the direct (single-producer) SOR pipeline.
///
/// It reuses one [`ExecutionReport`] scratch buffer across calls via
/// `execute_into`, so the hot path performs no heap allocation.
struct DirectSorProcessor {
    router: SmartOrderRouter,
    scratch: ExecutionReport,
    /// If set, the symbol is taken directly instead of hashed per order.
    fixed_symbol: Option<SymbolId>,
}

impl DirectSorProcessor {
    fn new(router: SmartOrderRouter, fixed_symbol: Option<SymbolId>) -> Self {
        DirectSorProcessor {
            router,
            scratch: ExecutionReport::default(),
            fixed_symbol,
        }
    }
}

impl Handler<OrderRequest, CompactReport> for DirectSorProcessor {
    fn handle(&mut self, input: OrderRequest) -> CompactReport {
        let symbol = match self.fixed_symbol {
            Some(symbol) => symbol,
            None => self.router.symbols().id(&input.symbol).unwrap_or(u32::MAX),
        };
        self.router.execute_into(&input, symbol, &mut self.scratch);
        CompactReport::from_report(&self.scratch)
    }
}

/// Lowest-latency SOR pipeline for a single producer thread.
///
/// Unlike [`SorPipeline`] there is no ingester thread: the caller publishes
/// straight into the command ring and the core publishes straight back. A
/// synchronous `submit` is therefore a single caller↔core exchange with no
/// per-order allocation (the core reuses a scratch report) and no parking.
/// The result is a `Copy` [`CompactReport`] rather than the heap-owning
/// [`ExecutionReport`].
pub struct DirectSorPipeline {
    inner: DirectPipeline<OrderRequest, CompactReport>,
}

impl DirectSorPipeline {
    pub fn spawn(router: SmartOrderRouter, config: DirectConfig) -> Self {
        Self::build(router, None, config)
    }

    /// Spawn for a single known symbol, skipping the per-order symbol hash.
    pub fn spawn_for_symbol(
        router: SmartOrderRouter,
        symbol: SymbolId,
        config: DirectConfig,
    ) -> Self {
        Self::build(router, Some(symbol), config)
    }

    fn build(
        router: SmartOrderRouter,
        fixed_symbol: Option<SymbolId>,
        config: DirectConfig,
    ) -> Self {
        DirectSorPipeline {
            inner: DirectPipeline::spawn(config, DirectSorProcessor::new(router, fixed_symbol)),
        }
    }

    /// Route one order and return its compact execution summary.
    pub fn submit(&self, request: OrderRequest) -> CompactReport {
        self.inner.submit(request)
    }

    /// Stop the core once all in-flight work has drained.
    pub fn shutdown(self) {
        self.inner.shutdown();
    }
}

/// Producer-side handle for one shard of a [`ShardedSorPipeline`].
///
/// `!Sync`: move exactly one handle to each producer thread.
pub struct ShardSorProducer {
    inner: ShardProducer<OrderRequest, CompactReport>,
}

impl ShardSorProducer {
    /// Shard index, stable for the life of the pipeline.
    pub fn id(&self) -> usize {
        self.inner.id()
    }

    /// Route one order and return its compact execution summary.
    pub fn submit(&self, request: OrderRequest) -> CompactReport {
        self.inner.submit(request)
    }

    /// Best-effort pinning of the current producer thread.
    pub fn pin_to(&self, core: core_affinity::CoreId) -> bool {
        self.inner.pin_to(core)
    }
}

/// Multi-producer, single-core SOR pipeline using one SPSC ring pair per
/// producer (see [`crate::disruptor::sharded`]).
///
/// Each producer writes directly into its own command ring and reads its own
/// result ring, so a submit is a single producer↔core hop with no cross-producer
/// contention. There is no global ordering across producers; a backtest that
/// needs a defined order should use a single producer.
pub struct ShardedSorPipeline {
    core: ShardedPipeline,
    producers: Vec<ShardSorProducer>,
}

impl ShardedSorPipeline {
    /// Spawn `num_producers` shards served by one core running `router`.
    pub fn spawn(router: SmartOrderRouter, config: ShardConfig, num_producers: usize) -> Self {
        Self::build(router, None, config, num_producers)
    }

    /// Spawn for a single known symbol, skipping the per-order symbol hash.
    pub fn spawn_for_symbol(
        router: SmartOrderRouter,
        symbol: SymbolId,
        config: ShardConfig,
        num_producers: usize,
    ) -> Self {
        Self::build(router, Some(symbol), config, num_producers)
    }

    fn build(
        router: SmartOrderRouter,
        fixed_symbol: Option<SymbolId>,
        config: ShardConfig,
        num_producers: usize,
    ) -> Self {
        let (core, producers) = spawn_sharded(
            config,
            DirectSorProcessor::new(router, fixed_symbol),
            num_producers,
        );
        ShardedSorPipeline {
            core,
            producers: producers
                .into_iter()
                .map(|inner| ShardSorProducer { inner })
                .collect(),
        }
    }

    /// Move the producer handles out so they can be distributed across threads.
    pub fn take_producers(&mut self) -> Vec<ShardSorProducer> {
        std::mem::take(&mut self.producers)
    }

    /// Number of producer shards.
    pub fn shard_count(&self) -> usize {
        self.core.shard_count()
    }

    /// Stop the core once all producers have stopped submitting.
    pub fn shutdown(self) {
        self.core.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disruptor::WaitStrategy;
    use crate::order::Side;
    use crate::router::fixed::Fixed;

    const SYMBOL: &str = "AAPL";

    fn pipeline() -> SorPipeline {
        let router = SmartOrderRouter::simulated(&[SYMBOL], 100.0, 42);
        SorPipeline::spawn(router, PipelineConfig::default())
    }

    fn request(quantity: f64) -> OrderRequest {
        OrderRequest::market(SYMBOL, Side::Buy, quantity, 100.0).with_adv(5_000_000.0)
    }

    #[test]
    fn test_submit_returns_report() {
        let pipeline = pipeline();
        let report = pipeline.submit(request(1_000.0)).unwrap();
        assert_eq!(report.filled_qty, Fixed::from_f64(1_000.0));
        assert_eq!(pipeline.processed(), 1);
        pipeline.shutdown();
    }

    #[test]
    fn test_many_submissions() {
        let pipeline = pipeline();
        for _ in 0..100 {
            let report = pipeline.submit(request(100.0)).unwrap();
            assert!(report.filled_qty.is_positive());
        }
        assert_eq!(pipeline.processed(), 100);
        pipeline.shutdown();
    }

    #[test]
    fn test_fire_and_forget() {
        let pipeline = pipeline();
        for _ in 0..1_000 {
            pipeline.try_submit(request(10.0)).unwrap();
        }
        // Wait for the core to drain.
        let mut spins = 0;
        while pipeline.processed() < 1_000 {
            std::thread::yield_now();
            spins += 1;
            assert!(spins < 1_000_000, "core did not drain");
        }
        pipeline.shutdown();
    }

    #[test]
    fn test_concurrent_submitters() {
        use std::sync::Arc;
        let pipeline = Arc::new(pipeline());
        let mut threads = Vec::new();
        for _ in 0..4 {
            let pipeline = Arc::clone(&pipeline);
            threads.push(std::thread::spawn(move || {
                for _ in 0..250 {
                    pipeline.submit(request(10.0)).unwrap();
                }
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(pipeline.processed(), 1_000);
        Arc::try_unwrap(pipeline).ok().unwrap().shutdown();
    }

    #[test]
    fn test_shutdown_is_clean() {
        let pipeline = pipeline();
        pipeline.submit(request(10.0)).unwrap();
        pipeline.shutdown();
    }

    #[test]
    fn test_direct_pipeline_round_trip() {
        let router = SmartOrderRouter::simulated(&[SYMBOL], 100.0, 42);
        let pipeline = DirectSorPipeline::spawn(
            router,
            DirectConfig {
                wait_strategy: WaitStrategy::BusySpin,
                ..DirectConfig::default()
            },
        );
        for _ in 0..100 {
            let report = pipeline.submit(request(100.0));
            assert!(report.filled_qty.is_positive());
        }
        pipeline.shutdown();
    }

    #[test]
    fn test_sharded_pipeline_multiple_producers() {
        let router = SmartOrderRouter::simulated(&[SYMBOL], 100.0, 42);
        let symbol = router.symbols().id(SYMBOL).unwrap();
        let mut pipeline = ShardedSorPipeline::spawn_for_symbol(
            router,
            symbol,
            ShardConfig {
                wait_strategy: WaitStrategy::BusySpin,
                ..ShardConfig::default()
            },
            4,
        );
        assert_eq!(pipeline.shard_count(), 4);
        let producers = pipeline.take_producers();
        assert_eq!(producers.len(), 4);

        let threads: Vec<_> = producers
            .into_iter()
            .map(|producer| {
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        let report = producer.submit(request(100.0));
                        assert!(report.filled_qty.is_positive());
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        pipeline.shutdown();
    }
}
