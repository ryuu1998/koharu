use std::{sync::Arc, time::Duration};

use crate::{Stage, resources::ResourceMonitor, stages::Stages};

pub(crate) struct AcceleratorGate {
    resources: Arc<ResourceMonitor>,
    work: Option<Arc<tokio::sync::RwLock<()>>>,
}

pub(crate) enum AcceleratorPermit {
    Inference {
        _guard: tokio::sync::OwnedRwLockReadGuard<()>,
    },
    Recovery {
        _guard: tokio::sync::OwnedRwLockWriteGuard<()>,
    },
    Cpu,
}

impl AcceleratorGate {
    pub(crate) fn new(device: &koharu_ml::Device, resources: Arc<ResourceMonitor>) -> Self {
        let accelerated = device.backend != koharu_ml::Backend::Cpu
            && device.device_type != koharu_ml::DeviceType::Cpu;
        Self {
            resources,
            // Stage capacities bound ordinary inference independently. Only
            // memory recovery needs exclusive access to unload other models.
            work: accelerated.then(|| Arc::new(tokio::sync::RwLock::new(()))),
        }
    }

    pub(crate) async fn acquire(&self) -> AcceleratorPermit {
        let Some(work) = self.work.as_ref() else {
            return AcceleratorPermit::Cpu;
        };
        AcceleratorPermit::Inference {
            _guard: work.clone().read_owned().await,
        }
    }

    pub(crate) async fn recover(&self, stage: Stage, stages: &Stages) -> AcceleratorPermit {
        let Some(work) = self.work.as_ref() else {
            return AcceleratorPermit::Cpu;
        };
        let permit = AcceleratorPermit::Recovery {
            _guard: work.clone().write_owned().await,
        };
        if unload_other_models(stage, stages) {
            let mut changed = self.resources.subscribe();
            let _ = tokio::time::timeout(Duration::from_millis(600), changed.changed()).await;
        }
        permit
    }
}

fn unload_other_models(requested: Stage, stages: &Stages) -> bool {
    let mut unloaded = false;
    for stage in Stage::ALL {
        if stage != requested && stages.unload(stage) {
            unloaded = true;
            tracing::debug!(%stage, "unloaded model while recovering from memory pressure");
        }
    }
    unloaded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate() -> AcceleratorGate {
        AcceleratorGate {
            resources: ResourceMonitor::new(&koharu_ml::Device::cpu()),
            work: Some(Arc::new(tokio::sync::RwLock::new(()))),
        }
    }

    #[tokio::test]
    async fn independent_inference_jobs_do_not_wait_for_each_other() {
        let gate = gate();
        let detection = gate.acquire().await;
        let ocr = tokio::time::timeout(Duration::from_secs(1), gate.acquire())
            .await
            .expect("OCR must run while detection retains its guard");
        let inpainting = tokio::time::timeout(Duration::from_secs(1), gate.acquire())
            .await
            .expect("cleanup must run alongside detection and OCR");
        drop((detection, ocr, inpainting));
    }

    #[tokio::test]
    async fn recovery_drains_inference_and_blocks_new_model_work() {
        let gate = gate();
        let translator = koharu_translator::Translator::from_config(
            koharu_ml::Device::cpu(),
            koharu_config::Config::memory(koharu_translator::ProvidersConfig::default()),
        )
        .unwrap();
        let stages = Stages::new(
            &crate::PipelineConfig::default(),
            translator,
            &koharu_ml::Device::cpu(),
        )
        .unwrap();
        let inference = gate.acquire().await;
        let recovery = gate.recover(Stage::Detection, &stages);
        tokio::pin!(recovery);
        assert!(futures::poll!(recovery.as_mut()).is_pending());
        let next = gate.acquire();
        tokio::pin!(next);
        assert!(futures::poll!(next.as_mut()).is_pending());
        drop(inference);
        let exclusive = tokio::time::timeout(Duration::from_secs(1), recovery)
            .await
            .unwrap();
        assert!(futures::poll!(next.as_mut()).is_pending());
        drop(exclusive);
        tokio::time::timeout(Duration::from_secs(1), next)
            .await
            .unwrap();
    }
}
