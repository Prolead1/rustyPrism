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

use crate::disruptor::{EventHandler, IngressSender, PipelineConfig, PipelineHandle, PublishError};
use crate::router::sor::{ExecutionReport, OrderRequest, SmartOrderRouter};
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
