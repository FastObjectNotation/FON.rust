use std::sync::{Arc, Mutex};

use rayon::{ThreadPool, ThreadPoolBuilder};

#[cfg(test)]
pub(crate) static BUILDS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);


// The most recently built pool and the thread count it was built with. One slot is
// enough for the usual case (the same `max_threads` on every call) and keeps at most
// one extra set of worker threads alive.
static CACHED: Mutex<Option<(usize, Arc<ThreadPool>)>> = Mutex::new(None);


// Run `op` on a dedicated pool of `max_threads` workers when that is positive, so
// every rayon call inside `op` is capped to that many threads. The cap never exceeds
// the machine's parallelism: asking for more would only add idle OS threads. `<= 0`
// (and a pool that cannot be built) runs `op` on the global pool.
pub(crate) fn run<R, F>(max_threads: i32, op: F) -> R
where
    R: Send,
    F: FnOnce() -> R + Send,
{
    if max_threads <= 0 {
        return op();
    }
    let cores: usize = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    match pool_with((max_threads as usize).min(cores)) {
        Some(pool) => pool.install(op),
        None => op(),
    }
}


fn pool_with(threads: usize) -> Option<Arc<ThreadPool>> {
    let mut slot = match CACHED.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some((cached, pool)) = slot.as_ref() {
        if *cached == threads {
            return Some(Arc::clone(pool));
        }
    }
    #[cfg(test)]
    BUILDS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let pool = Arc::new(ThreadPoolBuilder::new().num_threads(threads).build().ok()?);
    *slot = Some((threads, Arc::clone(&pool)));
    Some(pool)
}


#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::{run, BUILDS};


    #[test]
    fn repeated_calls_reuse_one_pool() {
        let before = BUILDS.load(Ordering::SeqCst);
        for i in 0..200 {
            assert_eq!(run(2, || i * 2), i * 2);
        }
        let built = BUILDS.load(Ordering::SeqCst) - before;
        assert!(built <= 1, "built {built} pools for 200 calls with max_threads=2");
    }
}
