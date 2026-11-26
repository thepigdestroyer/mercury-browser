// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

// Page lane: the screen wake lock that the engine takes through the embedder. Stub: the lock
// reports success without touching the OS until the lane's tasks fill it.

use std::sync::Arc;

use servo::{WakeLockDelegate, WakeLockType};

/// The OS side of a display wake lock.
pub trait DisplayAwake: Send + Sync {
    fn keep_awake(&self) -> Result<(), String>;
    fn allow_sleep(&self) -> Result<(), String>;
}

/// The screen wake-lock delegate of the engine. Stub: forwards nothing yet.
pub struct MercuryWakeLock(pub Arc<dyn DisplayAwake>);

impl WakeLockDelegate for MercuryWakeLock {
    fn acquire(&self, _type_: WakeLockType) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    fn release(&self, _type_: WakeLockType) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }
}
