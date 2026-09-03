//! Per-key request limits, in the process.
//!
//! The Worker had Cloudflare's rate limiting bindings; this has a map. The limits are
//! the same ones, because they bound the same attacks: `/api/subscribe` makes this
//! server send mail to whatever address a stranger types, and unrated that is a mail
//! bomb aimed by anyone at anyone, from a self-hosted sender whose reputation is the
//! whole asset. Per-IP stops one source spraying confirmation mail at many addresses;
//! per-address stops many sources converging on one inbox.
//!
//! Fixed windows of sixty seconds, so what these bound is burst rate, not daily volume.
//! nginx in front carries a second, coarser limit keyed on the client address, which is
//! what drops a flood before it becomes a connection; this one knows the address in the
//! request body, which nginx does not.

use std::collections::HashMap;
use std::sync::Mutex;

pub struct Limiter {
    window_secs: u64,
    buckets: Mutex<HashMap<String, (u64, u32)>>,
}

/// One source spraying confirmation mail at many addresses.
pub const SUBSCRIBE_PER_IP: u32 = 5;
/// Many sources converging on one inbox. Tighter: a real person submits the form once,
/// and twice only if they think it failed.
pub const SUBSCRIBE_PER_EMAIL: u32 = 2;
/// Confirm and unsubscribe, per IP. Looser, because these do not send mail.
pub const API_PER_IP: u32 = 15;

impl Limiter {
    pub fn new(window_secs: u64) -> Self {
        Self {
            window_secs,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Consume one unit for `key` at `now`. `true` means allowed.
    pub fn allow(&self, key: &str, limit: u32, now: u64) -> bool {
        let window = now / self.window_secs;
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());

        // Old windows are dropped as they are met, and the whole map is swept once it
        // grows past what any honest population of clients produces, so a scan cannot
        // turn this into a memory leak.
        if buckets.len() > 10_000 {
            buckets.retain(|_, (w, _)| *w == window);
        }

        let entry = buckets.entry(key.to_string()).or_insert((window, 0));
        if entry.0 != window {
            *entry = (window, 0);
        }
        if entry.1 >= limit {
            return false;
        }
        entry.1 += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limit_is_the_count_per_window() {
        let l = Limiter::new(60);
        for _ in 0..3 {
            assert!(l.allow("k", 3, 60));
        }
        assert!(!l.allow("k", 3, 60));
        assert!(!l.allow("k", 3, 119), "same window");
        assert!(l.allow("k", 3, 120), "next window");
    }

    #[test]
    fn keys_are_independent() {
        let l = Limiter::new(60);
        assert!(l.allow("a", 1, 0));
        assert!(!l.allow("a", 1, 0));
        assert!(l.allow("b", 1, 0));
    }
}
