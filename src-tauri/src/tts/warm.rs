//! Keeps an engine loaded for a while after a reading, so the next reading
//! doesn't wait for the model to load again.

use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

/// Holds one idle value until it is taken, replaced, or left unused for
/// `idle`. Then the value is dropped, which stops its engine.
pub struct KeepWarm<T> {
    /// For the log, e.g. `Kokoro engine`.
    name: &'static str,
    idle: Duration,
    slot: Mutex<Slot<T>>,
}

struct Slot<T> {
    value: Option<T>,
    /// Bumped by every put, so an old timer never drops a newer value.
    generation: u64,
}

impl<T: Send + 'static> KeepWarm<T> {
    pub const fn new(name: &'static str, idle: Duration) -> KeepWarm<T> {
        KeepWarm {
            name,
            idle,
            slot: Mutex::new(Slot {
                value: None,
                generation: 0,
            }),
        }
    }

    /// The idle value, if there is one.
    pub fn take(&self) -> Option<T> {
        self.lock().value.take()
    }

    /// Keep `value` for the next reading, in place of any value already held.
    pub fn put(&'static self, value: T) {
        let (old, generation) = {
            let mut slot = self.lock();
            slot.generation += 1;
            (slot.value.replace(value), slot.generation)
        };
        // Dropped outside the lock: stopping an engine waits for it to exit.
        drop(old);
        std::thread::spawn(move || {
            std::thread::sleep(self.idle);
            let expired = {
                let mut slot = self.lock();
                if slot.generation == generation {
                    slot.value.take()
                } else {
                    None
                }
            };
            if expired.is_some() {
                log::info!(
                    "{} unloaded after {} s idle",
                    self.name,
                    self.idle.as_secs()
                );
            }
        });
    }

    fn lock(&self) -> MutexGuard<'_, Slot<T>> {
        self.slot.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    /// A value that counts how many times one like it was dropped.
    struct Probe(u32, Arc<AtomicUsize>);

    impl Drop for Probe {
        fn drop(&mut self) {
            self.1.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn keep(idle_ms: u64) -> (&'static KeepWarm<Probe>, Arc<AtomicUsize>) {
        let warm = Box::leak(Box::new(KeepWarm::new(
            "test",
            Duration::from_millis(idle_ms),
        )));
        (warm, Arc::new(AtomicUsize::new(0)))
    }

    #[test]
    fn a_value_put_back_is_taken_once() {
        let (warm, drops) = keep(60_000);
        assert!(warm.take().is_none());
        warm.put(Probe(1, drops.clone()));
        assert_eq!(warm.take().map(|p| p.0), Some(1));
        assert!(warm.take().is_none());
    }

    #[test]
    fn a_new_value_replaces_and_drops_the_old_one() {
        let (warm, drops) = keep(60_000);
        warm.put(Probe(1, drops.clone()));
        warm.put(Probe(2, drops.clone()));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(warm.take().map(|p| p.0), Some(2));
    }

    #[test]
    fn an_unused_value_is_dropped_after_the_idle_time() {
        let (warm, drops) = keep(50);
        warm.put(Probe(1, drops.clone()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while drops.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(warm.take().is_none());
    }

    #[test]
    fn using_a_value_restarts_its_idle_time() {
        let (warm, drops) = keep(400);
        warm.put(Probe(1, drops.clone()));
        std::thread::sleep(Duration::from_millis(250));
        let probe = warm.take().unwrap();
        warm.put(probe);
        // Past the first put's idle time, well inside the second's.
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(warm.take().map(|p| p.0), Some(1));
    }
}
