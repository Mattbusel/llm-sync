// SPDX-License-Identifier: MIT
//! Timestamps for last-write-wins data that respect cause and effect (feature `hlc`).
//!
//! [`LWWRegister`](crate::LWWRegister) and [`ORMap`](crate::ORMap) keep the
//! write with the highest timestamp. Using each machine's wall clock for that
//! goes wrong when clocks disagree: an agent whose clock is 300 ms slow can
//! read a value, decide to change it, write, and still lose to the value it
//! just read, because its "later" write carries an earlier time.
//!
//! [`AgentClock`] is a hybrid logical clock (HLC), from the
//! [uhlc](https://crates.io/crates/uhlc) crate used by Eclipse Zenoh. Its
//! timestamps stay close to wall-clock time, never go backwards, and after
//! [`observe`](AgentClock::observe)-ing a remote timestamp every new timestamp
//! is larger than it. Timestamps that claim to be further in the future than
//! `max_drift` are refused, so one machine with a broken clock cannot push
//! everyone's timestamps forward.
//!
//! ```
//! use llm_sync::{AgentClock, AgentState, LWWRegister};
//!
//! let clock = AgentClock::new();
//! let mut plan = LWWRegister::new();
//! plan.write("draft".to_string(), clock.now(), "agent-a");
//!
//! // After merging someone else's state, tell the clock what you saw.
//! let remote = AgentState::new();
//! clock.observe(remote.max_timestamp())?;
//! plan.write("final".to_string(), clock.now(), "agent-a");
//! assert_eq!(plan.read().map(String::as_str), Some("final"));
//! # Ok::<(), llm_sync::SyncError>(())
//! ```
//!
//! On `wasm32-unknown-unknown` there is no system clock: build the clock with
//! [`AgentClock::with_physical_clock`] and a function that reads `Date.now()`.

use crate::error::SyncError;
use std::time::Duration;
use uhlc::{HLCBuilder, Timestamp, HLC};

/// The 64-bit NTP time format the clock uses (re-exported from uhlc).
pub use uhlc::NTP64;

/// A hybrid logical clock for one agent.
pub struct AgentClock {
    hlc: HLC,
}

impl std::fmt::Debug for AgentClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentClock").field("id", &self.hlc.get_id().to_string()).finish()
    }
}

impl Default for AgentClock {
    fn default() -> Self {
        Self::new()
    }
}

/// Physical time for [`AgentClock::with_physical_clock`] from milliseconds
/// since the Unix epoch.
pub fn physical_time(ms: u64) -> NTP64 {
    NTP64::from(Duration::from_millis(ms))
}

/// Convert milliseconds since the Unix epoch into the clock's timestamp format
/// (the NTP64 format: seconds in the high 32 bits, fraction in the low 32).
pub fn millis_to_timestamp(ms: u64) -> u64 {
    NTP64::from(Duration::from_millis(ms)).as_u64()
}

/// Convert a timestamp from [`AgentClock::now`] back to milliseconds since the Unix epoch.
pub fn timestamp_to_millis(ts: u64) -> u64 {
    u64::try_from(NTP64(ts).to_duration().as_millis()).unwrap_or(u64::MAX)
}

impl AgentClock {
    /// A clock on the system wall clock with a random id and 500 ms
    /// maximum accepted drift.
    pub fn new() -> Self {
        Self { hlc: HLCBuilder::new().build() }
    }

    /// A clock that reads physical time from `now`, for platforms without
    /// `SystemTime` (wasm32) and for tests. Any non-capturing closure works:
    ///
    /// ```
    /// # fn date_now_ms() -> u64 { 1_700_000_000_000 }
    /// use llm_sync::clock::{physical_time, AgentClock};
    /// let clock = AgentClock::with_physical_clock(|| physical_time(date_now_ms()), std::time::Duration::from_millis(500));
    /// assert!(clock.now() > 0);
    /// ```
    pub fn with_physical_clock(now: fn() -> NTP64, max_drift: Duration) -> Self {
        Self { hlc: HLCBuilder::new().with_clock(now).with_max_delta(max_drift).build() }
    }

    /// A new timestamp, larger than every timestamp this clock produced or observed.
    pub fn now(&self) -> u64 {
        self.hlc.new_timestamp().get_time().as_u64()
    }

    /// Take a timestamp seen in remote data into account, so the next
    /// [`now`](Self::now) is larger than it. 0 (no timestamps yet) is a no-op.
    ///
    /// # Errors
    /// [`SyncError::InvalidState`] if `remote` is further ahead of this
    /// machine's clock than the allowed drift (default 500 ms). The clock is
    /// not changed in that case.
    pub fn observe(&self, remote: u64) -> Result<(), SyncError> {
        if remote == 0 {
            return Ok(());
        }
        let ts = Timestamp::new(NTP64(remote), *self.hlc.get_id());
        self.hlc.update_with_timestamp(&ts).map_err(|e| SyncError::InvalidState(format!("remote clock too far ahead: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LWWRegister;
    use std::sync::atomic::{AtomicU64, Ordering};

    static FAST: AtomicU64 = AtomicU64::new(1_000_000);
    static SLOW: AtomicU64 = AtomicU64::new(1_000_000 - 300);
    fn fast() -> NTP64 { physical_time(FAST.load(Ordering::SeqCst)) }
    fn slow() -> NTP64 { physical_time(SLOW.load(Ordering::SeqCst)) }

    #[test]
    fn slow_clock_still_writes_after_what_it_saw() {
        let a = AgentClock::with_physical_clock(fast, Duration::from_millis(500));
        let b = AgentClock::with_physical_clock(slow, Duration::from_millis(500)); // 300 ms behind

        let mut reg = LWWRegister::new();
        reg.write("A's plan".to_string(), a.now(), "a");

        // Without an HLC, B's later write uses its slow wall clock and loses.
        let mut naive = reg.clone();
        naive.write("B's fix".to_string(), slow().as_u64(), "b");
        assert_eq!(naive.read().map(String::as_str), Some("A's plan"));

        // With the HLC, B observes A's timestamp first and its write wins.
        let mut reg_b = reg.clone();
        b.observe(reg_b.timestamp()).unwrap();
        reg_b.write("B's fix".to_string(), b.now(), "b");
        assert_eq!(reg_b.read().map(String::as_str), Some("B's fix"));
        assert_eq!(reg.merge(&reg_b).read(), reg_b.merge(&reg).read());
    }

    #[test]
    fn never_goes_backwards_and_refuses_far_future() {
        static FROZEN: AtomicU64 = AtomicU64::new(5_000_000);
        fn frozen() -> NTP64 { physical_time(FROZEN.load(Ordering::SeqCst)) }
        let c = AgentClock::with_physical_clock(frozen, Duration::from_millis(500));
        let t1 = c.now();
        let t2 = c.now(); // same physical millisecond
        assert!(t2 > t1);
        FROZEN.store(4_000_000, Ordering::SeqCst); // wall clock jumps back
        assert!(c.now() > t2);
        FROZEN.store(5_000_000, Ordering::SeqCst);
        // 10 s ahead of our clock: refused, and the clock is unchanged
        let before = c.now();
        assert!(c.observe(millis_to_timestamp(5_010_000)).is_err());
        assert!(c.now() < millis_to_timestamp(5_001_000).max(before + 10));
        // 200 ms ahead: accepted
        let near = millis_to_timestamp(5_000_200);
        c.observe(near).unwrap();
        assert!(c.now() > near);
        c.observe(0).unwrap();
    }

    #[test]
    fn millis_round_trip() {
        let ts = millis_to_timestamp(1_700_000_000_123);
        assert_eq!(timestamp_to_millis(ts), 1_700_000_000_123);
        let sys = AgentClock::new().now();
        assert!(timestamp_to_millis(sys) > 1_700_000_000_000);
    }
}
