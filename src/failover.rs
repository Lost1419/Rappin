//! Failover is restricted to the two documented Yapin API origins.
use std::{sync::Mutex, time::Duration};
use tokio::time::Instant;

pub(crate) struct Failover {
    fallback: Option<String>,
    until: Mutex<Option<Instant>>,
}
impl Failover {
    pub(crate) fn new(base: &str, enabled: bool) -> Self {
        Self {
            fallback: (enabled && base == crate::DEFAULT_API_BASE)
                .then(|| crate::utils::LEGACY_API_BASE.to_owned()),
            until: Mutex::new(None),
        }
    }
    pub(crate) fn base(&self, primary: &str) -> String {
        let until = self.until.lock().unwrap_or_else(|p| p.into_inner());
        if until.is_some_and(|until| until > Instant::now()) {
            if let Some(fallback) = &self.fallback {
                return fallback.clone();
            }
        }
        primary.to_owned()
    }
    /// Only the request which switches origins may immediately retry a safe read.
    pub(crate) fn activate(&self, attempted: &str) -> bool {
        let Some(fallback) = &self.fallback else {
            return false;
        };
        if attempted == fallback {
            return false;
        }
        *self.until.lock().unwrap_or_else(|p| p.into_inner()) =
            Some(Instant::now() + Duration::from_secs(600));
        true
    }
    #[cfg(test)]
    pub(crate) fn for_test(fallback: String) -> Self {
        Self {
            fallback: Some(fallback),
            until: Mutex::new(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_default_origin_can_fail_over_and_cooldown_expires() {
        for base in [
            "https://custom.example/api",
            crate::utils::LEGACY_API_BASE,
            "http://127.0.0.1:42",
        ] {
            let state = Failover::new(base, true);
            assert!(!state.activate(base));
            assert_eq!(state.base(base), base);
        }
        let disabled = Failover::new(crate::DEFAULT_API_BASE, false);
        assert!(!disabled.activate(crate::DEFAULT_API_BASE));
        let state = Failover::new(crate::DEFAULT_API_BASE, true);
        assert!(state.activate(crate::DEFAULT_API_BASE));
        assert_eq!(
            state.base(crate::DEFAULT_API_BASE),
            crate::utils::LEGACY_API_BASE
        );
        *state.until.lock().unwrap() = Some(Instant::now() - Duration::from_secs(1));
        assert_eq!(state.base(crate::DEFAULT_API_BASE), crate::DEFAULT_API_BASE);
    }
}
