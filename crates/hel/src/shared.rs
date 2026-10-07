//! H12: state that tool calls and subagents may touch from several threads at once.
//! `Shared` keeps the `borrow` / `borrow_mut` names the code used with `RefCell`, backed by a
//! mutex. Both lock the whole value, so a second borrow on the same thread waits forever where a
//! `RefCell` would have panicked; callers never hold one borrow across another.

use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Default)]
pub struct Shared<T>(Mutex<T>);

impl<T> Shared<T> {
    pub fn new(value: T) -> Self {
        Self(Mutex::new(value))
    }

    /// A panic in another thread leaves the value as it was; the guard is still usable.
    pub fn borrow(&self) -> MutexGuard<'_, T> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn borrow_mut(&self) -> MutexGuard<'_, T> {
        self.borrow()
    }
}

impl<T: Clone> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self::new(self.borrow().clone())
    }
}

impl<T: Serialize> Serialize for Shared<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.borrow().serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Shared<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::deserialize(deserializer).map(Self::new)
    }
}
