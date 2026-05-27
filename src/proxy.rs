use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::RwLock;

const MAX_FAILURES: u32 = 5;

#[derive(Debug, Clone)]
pub struct ProxyAddr {
    pub addr: String,
}

pub struct ProxyPool {
    proxies: Vec<ProxyAddr>,
    next_index: AtomicUsize,
    failures: RwLock<Vec<u32>>,
}

pub struct Lease {
    pub proxy_addr: Option<String>,
    pub proxy_index: Option<usize>,
}

impl ProxyPool {
    pub fn new(addrs: Vec<String>) -> Arc<Self> {
        let proxies: Vec<ProxyAddr> = addrs
            .into_iter()
            .map(|addr| ProxyAddr { addr })
            .collect();
        let failures = vec![0u32; proxies.len()];
        Arc::new(Self {
            proxies,
            next_index: AtomicUsize::new(0),
            failures: RwLock::new(failures),
        })
    }

    pub async fn lease(&self) -> Lease {
        if self.proxies.is_empty() {
            return Lease {
                proxy_addr: None,
                proxy_index: None,
            };
        }

        let failures = self.failures.read().await;
        let len = self.proxies.len();
        let start = self.next_index.fetch_add(1, Ordering::Relaxed);

        for i in 0..len {
            let idx = (start + i) % len;
            if failures[idx] < MAX_FAILURES {
                return Lease {
                    proxy_addr: Some(self.proxies[idx].addr.clone()),
                    proxy_index: Some(idx),
                };
            }
        }

        // All proxies exceeded failure threshold, reset and try again
        drop(failures);
        {
            let mut failures = self.failures.write().await;
            for f in failures.iter_mut() {
                *f = 0;
            }
        }

        let idx = start % len;
        Lease {
            proxy_addr: Some(self.proxies[idx].addr.clone()),
            proxy_index: Some(idx),
        }
    }

    pub async fn report_failure(&self, index: usize) {
        let mut failures = self.failures.write().await;
        if index < failures.len() {
            failures[index] += 1;
        }
    }

    pub async fn report_success(&self, index: usize) {
        let mut failures = self.failures.write().await;
        if index < failures.len() {
            failures[index] = 0;
        }
    }
}
