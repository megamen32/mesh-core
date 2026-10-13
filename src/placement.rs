//! Work placement uses capacity and load supplied by the owning runtime.
//! No probing, inference, credentials, or queue ownership lives in this module.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerRating {
    pub host_id: String,
    /// Comparable units for the same workload; callers identify the basis.
    pub performance: f64,
    pub utilization_percent: f64,
    pub observed_at: u64,
    pub available: bool,
}

impl WorkerRating {
    pub fn effective_performance(&self, now: u64, max_age_seconds: u64) -> Option<f64> {
        if !self.available
            || self.host_id.is_empty()
            || self.observed_at > now.saturating_add(5)
            || now.saturating_sub(self.observed_at) > max_age_seconds
            || !self.performance.is_finite()
            || self.performance <= 0.0
            || !self.utilization_percent.is_finite()
            || !(0.0..=100.0).contains(&self.utilization_percent)
        {
            return None;
        }
        Some(self.performance * (1.0 - self.utilization_percent / 100.0))
            .filter(|score| *score > 0.0)
    }
}

/// Return indices, preserving caller-owned routing and failure receipts.
pub fn rank_workers(workers: &[WorkerRating], now: u64, max_age_seconds: u64) -> Vec<usize> {
    let mut ranked: Vec<_> = workers
        .iter()
        .enumerate()
        .filter_map(|(index, worker)| {
            worker
                .effective_performance(now, max_age_seconds)
                .map(|score| (index, score))
        })
        .collect();
    ranked.sort_by(|(ai, a), (bi, b)| {
        b.total_cmp(a)
            .then_with(|| {
                workers[*ai]
                    .utilization_percent
                    .total_cmp(&workers[*bi].utilization_percent)
            })
            .then_with(|| workers[*ai].host_id.cmp(&workers[*bi].host_id))
    });
    ranked.into_iter().map(|(index, _)| index).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn worker(host: &str, performance: f64, load: f64) -> WorkerRating {
        WorkerRating {
            host_id: host.into(),
            performance,
            utilization_percent: load,
            observed_at: 100,
            available: true,
        }
    }
    #[test]
    fn slower_idle_worker_beats_faster_loaded_worker() {
        assert_eq!(
            rank_workers(
                &[worker("fast", 400.0, 80.0), worker("free", 200.0, 10.0)],
                100,
                10
            ),
            vec![1, 0]
        );
    }
    #[test]
    fn busy_missing_stale_and_invalid_capacity_do_not_receive_work() {
        let mut busy = worker("busy", 900.0, 0.0);
        busy.available = false;
        let stale = worker("old", 900.0, 0.0);
        assert!(rank_workers(
            &[
                busy,
                stale,
                worker("invalid", f64::NAN, 0.0),
                worker("badload", 100.0, 101.0)
            ],
            111,
            10
        )
        .is_empty());
        assert!(rank_workers(&[worker("full", 100.0, 100.0)], 100, 10).is_empty());
    }
    #[test]
    fn node_loss_leaves_other_ranked_nodes_and_ties_are_stable() {
        let mut failed = worker("fast", 500.0, 0.0);
        failed.available = false;
        assert_eq!(
            rank_workers(
                &[failed, worker("b", 100.0, 0.0), worker("a", 100.0, 0.0)],
                100,
                10
            ),
            vec![2, 1]
        );
    }
}
