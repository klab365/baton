//! Process-local leases for filesystem repository transactions.
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, OnceLock, Weak},
};

#[derive(Default)]
struct LockCell {
    held: Mutex<bool>,
    available: Condvar,
}

pub struct RepositoryLock(Arc<LockCell>);

impl Drop for RepositoryLock {
    fn drop(&mut self) {
        let mut held = self
            .0
            .held
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *held = false;
        self.0.available.notify_one();
    }
}

fn registry() -> &'static Mutex<HashMap<PathBuf, Weak<LockCell>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Weak<LockCell>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cell(key: PathBuf) -> Arc<LockCell> {
    let mut locks = registry().lock().unwrap_or_else(|error| error.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(LockCell::default());
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

pub fn lock(key: PathBuf) -> RepositoryLock {
    let lock = cell(key);
    let mut held = lock.held.lock().unwrap_or_else(|error| error.into_inner());
    while *held {
        held = lock
            .available
            .wait(held)
            .unwrap_or_else(|error| error.into_inner());
    }
    *held = true;
    drop(held);
    RepositoryLock(lock)
}

pub fn try_lock(key: PathBuf) -> Result<RepositoryLock, ()> {
    let lock = cell(key);
    let mut held = lock.held.lock().unwrap_or_else(|error| error.into_inner());
    if *held {
        return Err(());
    }
    *held = true;
    drop(held);
    Ok(RepositoryLock(lock))
}
