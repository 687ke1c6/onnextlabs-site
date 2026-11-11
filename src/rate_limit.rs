//! In-memory sliding-window rate limiting for the subscribe endpoint.
//! Per-IP limits stop a single client hammering it; the global cap bounds total
//! write volume if many IPs are used at once. State resets on restart, which is fine here.

use std::{
    collections::{HashMap, VecDeque},
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

pub struct Limit {
    pub max: usize,
    pub window: Duration,
}

pub struct RateLimiter {
    per_ip: Limit,
    global: Limit,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    by_ip: HashMap<IpAddr, VecDeque<Instant>>,
    global: VecDeque<Instant>,
}

fn prune(hits: &mut VecDeque<Instant>, now: Instant, window: Duration) {
    while hits
        .front()
        .is_some_and(|&t| now.duration_since(t) >= window)
    {
        hits.pop_front();
    }
}

/// Time until the oldest hit leaves the window.
fn retry_after(hits: &VecDeque<Instant>, now: Instant, window: Duration) -> Duration {
    hits.front()
        .map(|&t| window.saturating_sub(now.duration_since(t)))
        .unwrap_or_default()
}

impl RateLimiter {
    pub fn new(per_ip: Limit, global: Limit) -> Self {
        Self {
            per_ip,
            global,
            state: Mutex::default(),
        }
    }

    /// Records a hit for `ip` if allowed; otherwise returns how long to wait.
    pub fn check(&self, ip: IpAddr) -> Result<(), Duration> {
        self.check_at(ip, Instant::now())
    }

    fn check_at(&self, ip: IpAddr, now: Instant) -> Result<(), Duration> {
        let mut s = self.state.lock().unwrap();

        let hits = s.by_ip.entry(ip).or_default();
        prune(hits, now, self.per_ip.window);
        if hits.len() >= self.per_ip.max {
            return Err(retry_after(hits, now, self.per_ip.window));
        }

        prune(&mut s.global, now, self.global.window);
        if s.global.len() >= self.global.max {
            return Err(retry_after(&s.global, now, self.global.window));
        }

        s.global.push_back(now);
        s.by_ip.get_mut(&ip).unwrap().push_back(now);
        Ok(())
    }

    /// Drops IPs with no hits in the window so the map doesn't grow forever.
    pub fn sweep(&self) {
        let now = Instant::now();
        let window = self.per_ip.window;
        self.state.lock().unwrap().by_ip.retain(|_, hits| {
            prune(hits, now, window);
            !hits.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter() -> RateLimiter {
        RateLimiter::new(
            Limit {
                max: 2,
                window: Duration::from_secs(60),
            },
            Limit {
                max: 3,
                window: Duration::from_secs(60),
            },
        )
    }

    #[test]
    fn per_ip_limit_then_recovers() {
        let rl = limiter();
        let ip: IpAddr = "1.2.3.4".parse().unwrap();
        let t0 = Instant::now();
        assert!(rl.check_at(ip, t0).is_ok());
        assert!(rl.check_at(ip, t0).is_ok());
        assert_eq!(
            rl.check_at(ip, t0 + Duration::from_secs(10)),
            Err(Duration::from_secs(50))
        );
        assert!(rl.check_at(ip, t0 + Duration::from_secs(60)).is_ok());
    }

    #[test]
    fn global_limit_spans_ips() {
        let rl = limiter();
        let t0 = Instant::now();
        for i in 0..3 {
            assert!(rl.check_at(IpAddr::from([10, 0, 0, i]), t0).is_ok());
        }
        assert!(rl.check_at(IpAddr::from([10, 0, 0, 9]), t0).is_err());
    }

    #[test]
    fn rejected_hits_do_not_count() {
        let rl = limiter();
        let ip: IpAddr = "::1".parse().unwrap();
        let t0 = Instant::now();
        rl.check_at(ip, t0).unwrap();
        rl.check_at(ip, t0).unwrap();
        for _ in 0..10 {
            assert!(rl.check_at(ip, t0).is_err());
        }
        assert!(rl.check_at(ip, t0 + Duration::from_secs(60)).is_ok());
    }
}
