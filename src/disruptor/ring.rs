//! A lock-free single-producer / single-consumer (SPSC) ring buffer.
//!
//! This is the storage primitive behind the Disruptor. It is bounded, fully
//! pre-allocated, cache-line padded and wait-free on the fast path. There is no
//! lock and no allocation after construction.
//!
//! # Safety model
//!
//! The buffer uses `UnsafeCell<MaybeUninit<E>>` with acquire/release atomics to
//! hand entries from exactly one producer to exactly one consumer. The design
//! mirrors the LMAX Disruptor's claim/publish protocol:
//!
//! * the producer claims slot `tail & mask` only while `tail - head < capacity`,
//!   so it never overwrites an entry the consumer has not yet taken;
//! * the producer publishes `tail` with `Release` *after* writing the slot;
//! * the consumer observes `tail` with `Acquire`, reads the slot, then advances
//!   `head` with `Release` to let the producer reclaim the space.
//!
//! The [`Producer`] and [`Consumer`] handles are deliberately `!Sync` (via a
//! `PhantomData<Cell<()>>`) so a single one of each cannot be shared across
//! threads, which enforces the SPSC invariant at the type level.
//!
//! All `unsafe` in the crate is confined to this file.

#![allow(unsafe_code)]

use std::cell::{Cell, UnsafeCell};
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Pads a value to a 64-byte cache line to avoid false sharing between the
/// producer's `tail` and the consumer's `head`.
#[repr(align(64))]
struct CachePadded<T>(T);

impl<T> CachePadded<T> {
    fn new(value: T) -> Self {
        CachePadded(value)
    }
}

struct Inner<E> {
    mask: usize,
    capacity: usize,
    buffer: Box<[UnsafeCell<MaybeUninit<E>>]>,
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
}

// SAFETY: `Inner` is shared between the single producer and single consumer.
// The acquire/release protocol below makes concurrent access sound whenever
// `E: Send`.
unsafe impl<E: Send> Sync for Inner<E> {}

impl<E> Inner<E> {
    fn with_capacity(capacity: usize) -> Self {
        // A power-of-two capacity lets us index with a bitmask.
        let capacity = capacity.next_power_of_two().max(2);
        let buffer = (0..capacity)
            .map(|_| UnsafeCell::new(MaybeUninit::uninit()))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Inner {
            mask: capacity - 1,
            capacity,
            buffer,
            head: CachePadded::new(AtomicUsize::new(0)),
            tail: CachePadded::new(AtomicUsize::new(0)),
        }
    }
}

impl<E> Drop for Inner<E> {
    /// Drop entries that were published but never consumed.
    fn drop(&mut self) {
        let head = self.head.0.load(Ordering::Relaxed);
        let tail = self.tail.0.load(Ordering::Relaxed);
        let mut index = head;
        while index != tail {
            // SAFETY: `[head, tail)` are published, un-consumed entries. Both
            // endpoints are being dropped, so there is no concurrent access.
            unsafe {
                let slot = self.buffer.get_unchecked(index & self.mask).get();
                (*slot).assume_init_drop();
            }
            index = index.wrapping_add(1);
        }
    }
}

/// The producing half of an SPSC ring. Not `Sync`: only one thread may produce.
pub struct Producer<E> {
    inner: Arc<Inner<E>>,
    _not_sync: PhantomData<Cell<()>>,
}

impl<E: Send> Producer<E> {
    /// Attempt to publish without blocking. Returns the value back if full.
    pub fn try_publish(&self, value: E) -> Result<(), E> {
        let tail = self.inner.tail.0.load(Ordering::Relaxed);
        let head = self.inner.head.0.load(Ordering::Acquire);
        if tail.wrapping_sub(head) >= self.inner.capacity {
            return Err(value);
        }
        // SAFETY: single producer, and the slot is free (checked above). The
        // consumer cannot observe it until `tail` is published below.
        unsafe {
            let slot = self
                .inner
                .buffer
                .get_unchecked(tail & self.inner.mask)
                .get();
            slot.write(MaybeUninit::new(value));
        }
        self.inner
            .tail
            .0
            .store(tail.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    /// Publish, spinning with backoff while the ring is full. Never allocates.
    pub fn publish(&self, value: E) {
        let mut value = value;
        let mut spins = 0u32;
        loop {
            match self.try_publish(value) {
                Ok(()) => return,
                Err(returned) => {
                    value = returned;
                    backoff(&mut spins);
                }
            }
        }
    }

    pub fn capacity(&self) -> usize {
        self.inner.capacity
    }
}

/// The consuming half of an SPSC ring. Not `Sync`: only one thread may consume.
pub struct Consumer<E> {
    inner: Arc<Inner<E>>,
    _not_sync: PhantomData<Cell<()>>,
}

impl<E: Send> Consumer<E> {
    /// Attempt to consume without blocking.
    pub fn try_consume(&self) -> Option<E> {
        let head = self.inner.head.0.load(Ordering::Relaxed);
        let tail = self.inner.tail.0.load(Ordering::Acquire);
        if head == tail {
            return None;
        }
        // SAFETY: `head != tail` and `tail` was published with `Release`, so the
        // slot is fully initialised and this consumer owns it.
        let value = unsafe {
            let slot = self
                .inner
                .buffer
                .get_unchecked(head & self.inner.mask)
                .get();
            slot.read().assume_init()
        };
        self.inner
            .head
            .0
            .store(head.wrapping_add(1), Ordering::Release);
        Some(value)
    }

    /// Consume, spinning with backoff while the ring is empty.
    pub fn consume(&self) -> E {
        let mut spins = 0u32;
        loop {
            if let Some(value) = self.try_consume() {
                return value;
            }
            backoff(&mut spins);
        }
    }

    pub fn capacity(&self) -> usize {
        self.inner.capacity
    }
}

/// Create a connected SPSC ring with the given capacity (rounded up to a power
/// of two, minimum two).
pub fn spsc<E>(capacity: usize) -> (Producer<E>, Consumer<E>) {
    let inner = Arc::new(Inner::with_capacity(capacity));
    (
        Producer {
            inner: Arc::clone(&inner),
            _not_sync: PhantomData,
        },
        Consumer {
            inner,
            _not_sync: PhantomData,
        },
    )
}

/// Spin briefly, then yield, to balance latency against CPU burn.
#[inline]
fn backoff(spins: &mut u32) {
    if *spins < 128 {
        std::hint::spin_loop();
        *spins += 1;
    } else {
        std::thread::yield_now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_fifo_single_threaded() {
        let (producer, consumer) = spsc::<u32>(4);
        for value in 0..3 {
            producer.try_publish(value).unwrap();
        }
        assert_eq!(consumer.try_consume(), Some(0));
        assert_eq!(consumer.try_consume(), Some(1));
        assert_eq!(consumer.try_consume(), Some(2));
        assert_eq!(consumer.try_consume(), None);
    }

    #[test]
    fn test_full_returns_value() {
        let (producer, _consumer) = spsc::<u32>(4);
        for value in 0..4 {
            producer.try_publish(value).unwrap();
        }
        assert_eq!(producer.try_publish(99), Err(99));
    }

    #[test]
    fn test_empty_returns_none() {
        let (_producer, consumer) = spsc::<u32>(4);
        assert_eq!(consumer.try_consume(), None);
    }

    #[test]
    fn test_wraparound() {
        let (producer, consumer) = spsc::<u32>(4);
        for round in 0..100 {
            producer.try_publish(round).unwrap();
            assert_eq!(consumer.try_consume(), Some(round));
        }
    }

    #[test]
    fn test_drops_unconsumed_entries() {
        use std::sync::atomic::AtomicUsize;
        use std::sync::Arc;

        #[derive(Debug)]
        struct Counted(Arc<AtomicUsize>);
        impl Drop for Counted {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let drops = Arc::new(AtomicUsize::new(0));
        {
            let (producer, consumer) = spsc::<Counted>(4);
            for _ in 0..3 {
                producer.try_publish(Counted(Arc::clone(&drops))).unwrap();
            }
            // Consume one, leaving two in the ring.
            let _ = consumer.try_consume();
        }
        assert_eq!(drops.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_spsc_stress_preserves_order() {
        const COUNT: u64 = 200_000;
        let (producer, consumer) = spsc::<u64>(1024);

        let handle = thread::spawn(move || {
            for value in 0..COUNT {
                producer.publish(value);
            }
        });

        let mut expected = 0u64;
        while expected < COUNT {
            let value = consumer.consume();
            assert_eq!(value, expected);
            expected += 1;
        }
        handle.join().unwrap();
    }
}
