//! Work placement uses capacity and load supplied by the owning runtime.
//! No probing, inference, credentials, or queue ownership lives in this module.
use serde::{Deserialize, Serialize};

/// Routing class only. No file paths, content, credentials or model invocation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskDescriptor {
    pub input_type: String,
    pub input_bytes: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl TaskDescriptor {
    pub fn task_class(&self) -> Option<String> {
        if self.input_bytes == 0 || self.input_bytes > 32 * 1024 * 1024 {
            return None;
        }
        let kind = self.input_type.to_ascii_lowercase();
        if kind == "pdf" {
            if self.width.is_some() || self.height.is_some() {
                return None;
            }
            let bucket = match self.input_bytes {
                0..=1_048_576 => "1m",
                1_048_577..=4_194_304 => "4m",
                4_194_305..=16_777_216 => "16m",
                _ => "32m",
            };
            return Some(format!("ocr:pdf:bytes:{bucket}"));
        }
        let kind = match kind.as_str() {
            "jpg" | "jpeg" => "jpeg",
            "tif" | "tiff" => "tiff",
            "png" => "png",
            "webp" => "webp",
            "bmp" => "bmp",
            "gif" => "gif",
            "avif" => "avif",
            _ => return None,
        };
        let (width, height) = (self.width?, self.height?);
        let pixels = width as u64 * height as u64;
        let bucket = match pixels {
            0 => return None,
            1..=4_000_000 => "4mp",
            4_000_001..=16_000_000 => "16mp",
            16_000_001..=64_000_000 => "64mp",
            _ => "tiled",
        };
        Some(format!("ocr:image:{kind}:pixels:{bucket}"))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskWorkerRating {
    pub host_id: String,
    pub backend: String,
    /// Native jobs/second for this exact task class; None means unmeasured.
    pub performance: Option<f64>,
    pub utilization_percent: f64,
    pub observed_at: u64,
    pub available: bool,
}

/// Cold free devices get a first useful job. Afterwards compare only measured
/// rates for the requested class. Caller keeps one reservation per host.
pub fn rank_task_workers(workers: &[TaskWorkerRating], now: u64, max_age: u64) -> Vec<usize> {
    let mut ranked: Vec<_> = workers.iter().enumerate().filter_map(|(index, w)| {
        if !w.available || w.host_id.is_empty()
            || !matches!(w.backend.as_str(), "cpu" | "cuda" | "coreml")
            || w.observed_at > now.saturating_add(5)
            || now.saturating_sub(w.observed_at) > max_age
            || !w.utilization_percent.is_finite()
            || !(0.0..100.0).contains(&w.utilization_percent)
            || w.performance.is_some_and(|p| !p.is_finite() || p <= 0.0)
        { return None; }
        Some((index, w.performance.map(|p| p * (1.0 - w.utilization_percent / 100.0))))
    }).collect();
    ranked.sort_by(|(ai, a), (bi, b)| {
        match (a, b) {
            // Explore unmeasured hardware with real work, not an invented score.
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (Some(a), Some(b)) => b.total_cmp(a),
            (None, None) => std::cmp::Ordering::Equal,
        }.then_with(|| workers[*ai].utilization_percent.total_cmp(&workers[*bi].utilization_percent))
         .then_with(|| workers[*ai].host_id.cmp(&workers[*bi].host_id))
         .then_with(|| workers[*ai].backend.cmp(&workers[*bi].backend))
    });
    ranked.into_iter().map(|(index, _)| index).collect()
}

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
    #[test]
    fn task_class_matches_format_size_and_never_borrows_pdf_or_giant_speed() {
        let mut d = TaskDescriptor { input_type: "PNG".into(), input_bytes: 20000, width: Some(2000), height: Some(1000) };
        assert_eq!(d.task_class().as_deref(), Some("ocr:image:png:pixels:4mp"));
        d.height = Some(800000);
        assert_eq!(d.task_class().as_deref(), Some("ocr:image:png:pixels:tiled"));
        d.input_type = "pdf".into();
        assert!(d.task_class().is_none());
        d.width = None; d.height = None;
        assert_eq!(d.task_class().as_deref(), Some("ocr:pdf:bytes:1m"));
        d.input_bytes = 0; assert!(d.task_class().is_none());
    }
    #[test]
    fn cold_cpu_and_gpu_are_real_exploration_not_hardware_scores() {
        let sample = |host: &str, backend: &str, performance, load| TaskWorkerRating {
            host_id: host.into(), backend: backend.into(), performance,
            utilization_percent: load, observed_at: 100, available: true,
        };
        let workers = [sample("fast", "cuda", Some(10.0), 10.0), sample("cold", "cpu", None, 20.0), sample("cold", "coreml", None, 5.0)];
        assert_eq!(rank_task_workers(&workers, 100, 10), [2,1,0]);
        let workers = [sample("fast", "cuda", Some(10.0), 90.0), sample("free", "cpu", Some(2.0), 0.0)];
        assert_eq!(rank_task_workers(&workers, 100, 10), [1,0]);
        let mut busy = sample("busy", "cpu", None, 0.0); busy.available = false;
        assert!(rank_task_workers(&[busy, sample("bad", "cpu", Some(f64::NAN), 0.0)], 100, 10).is_empty());
    }
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
