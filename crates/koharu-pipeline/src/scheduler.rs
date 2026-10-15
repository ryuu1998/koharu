use std::collections::BTreeMap;

use koharu_scene::EntityId;

use crate::Stage;

#[derive(Clone, Copy, Eq, PartialEq)]
enum WorkState {
    Pending,
    Running,
    Finished,
}

struct StageWork {
    stage: Stage,
    state: WorkState,
}

struct PageWork {
    page: EntityId,
    stages: Vec<StageWork>,
}

impl PageWork {
    fn finished(&self) -> bool {
        self.stages
            .iter()
            .all(|work| work.state == WorkState::Finished)
    }

    fn ready(&self, index: usize) -> bool {
        let Some(prerequisite) = prerequisite(self.stages[index].stage) else {
            return true;
        };
        self.stages
            .iter()
            .find(|work| work.stage == prerequisite)
            .is_none_or(|work| work.state == WorkState::Finished)
    }
}

pub(crate) struct Scheduler {
    capacities: BTreeMap<Stage, usize>,
    running: BTreeMap<Stage, usize>,
    pages: Vec<PageWork>,
    page_index: BTreeMap<EntityId, usize>,
    head: usize,
    total: usize,
}

impl Scheduler {
    pub(crate) fn new(
        pages: &[EntityId],
        stages: &[Stage],
        capacity: impl Fn(Stage) -> usize,
    ) -> Self {
        let capacities = stages
            .iter()
            .map(|stage| (*stage, capacity(*stage).max(1)))
            .collect::<BTreeMap<_, _>>();
        let pages = pages
            .iter()
            .map(|page| PageWork {
                page: *page,
                stages: stages
                    .iter()
                    .map(|stage| StageWork {
                        stage: *stage,
                        state: WorkState::Pending,
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();
        let total = pages.len().saturating_mul(stages.len());
        Self {
            capacities,
            running: BTreeMap::new(),
            page_index: pages
                .iter()
                .enumerate()
                .map(|(index, page)| (page.page, index))
                .collect(),
            pages,
            head: 0,
            total,
        }
    }

    pub(crate) fn total(&self) -> usize {
        self.total
    }

    pub(crate) fn start_next(&mut self) -> Option<(EntityId, Stage)> {
        for page_index in self.head..self.pages.len() {
            let stage_index =
                self.pages[page_index]
                    .stages
                    .iter()
                    .enumerate()
                    .find_map(|(index, work)| {
                        (work.state == WorkState::Pending
                            && self.running.get(&work.stage).copied().unwrap_or(0)
                                < self.capacities[&work.stage]
                            && self.pages[page_index].ready(index))
                        .then_some(index)
                    });
            let Some(stage_index) = stage_index else {
                continue;
            };
            let page = &mut self.pages[page_index];
            let work = &mut page.stages[stage_index];
            work.state = WorkState::Running;
            *self.running.entry(work.stage).or_default() += 1;
            return Some((page.page, work.stage));
        }
        None
    }

    pub(crate) fn complete_stage(&mut self, page: EntityId, stage: Stage) -> bool {
        let Some(&page_index) = self.page_index.get(&page) else {
            return false;
        };
        let page = &mut self.pages[page_index];
        let was_finished = page.finished();
        if let Some(work) = page.stages.iter_mut().find(|work| work.stage == stage) {
            if work.state == WorkState::Running {
                *self
                    .running
                    .get_mut(&stage)
                    .expect("running stage has a reservation") -= 1;
            }
            work.state = WorkState::Finished;
        }
        let page_finished = !was_finished && page.finished();
        if page_finished {
            while self.head < self.pages.len() && self.pages[self.head].finished() {
                self.head += 1;
            }
        }
        page_finished
    }
}

const fn prerequisite(stage: Stage) -> Option<Stage> {
    match stage {
        Stage::Detection => None,
        Stage::Ocr | Stage::Inpainting => Some(Stage::Detection),
        Stage::Translation => Some(Stage::Ocr),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages(count: usize) -> Vec<EntityId> {
        (0..count).map(|_| EntityId::new()).collect()
    }

    #[test]
    fn starts_pages_in_order_and_models_independently() {
        let pages = pages(2);
        let mut scheduler = Scheduler::new(&pages, &Stage::ALL, |_| 1);

        let first = scheduler.start_next().unwrap();
        assert_eq!(first, (pages[0], Stage::Detection));
        assert!(scheduler.start_next().is_none());

        assert!(!scheduler.complete_stage(pages[0], Stage::Detection));
        let ocr = scheduler.start_next().unwrap();
        let inpainting = scheduler.start_next().unwrap();
        let next_page = scheduler.start_next().unwrap();
        assert_eq!(ocr, (pages[0], Stage::Ocr));
        assert_eq!(inpainting, (pages[0], Stage::Inpainting));
        assert_eq!(next_page, (pages[1], Stage::Detection));

        assert!(!scheduler.complete_stage(pages[0], Stage::Ocr));
        let translation = scheduler.start_next().unwrap();
        assert_eq!(translation, (pages[0], Stage::Translation));
        assert_eq!(scheduler.running[&Stage::Detection], 1);
        assert_eq!(scheduler.running[&Stage::Inpainting], 1);
    }

    #[test]
    fn detection_continues_while_ocr_and_cleanup_are_busy() {
        let pages = pages(22);
        let stages = [Stage::Detection, Stage::Ocr, Stage::Inpainting];
        let mut scheduler = Scheduler::new(&pages, &stages, |_| 1);

        assert_eq!(scheduler.start_next(), Some((pages[0], Stage::Detection)));
        assert!(!scheduler.complete_stage(pages[0], Stage::Detection));
        assert_eq!(scheduler.start_next(), Some((pages[0], Stage::Ocr)));
        assert_eq!(scheduler.start_next(), Some((pages[0], Stage::Inpainting)));

        for page in &pages[1..] {
            assert_eq!(scheduler.start_next(), Some((*page, Stage::Detection)));
            assert!(!scheduler.complete_stage(*page, Stage::Detection));
        }
        assert!(scheduler.start_next().is_none());

        assert!(!scheduler.complete_stage(pages[0], Stage::Ocr));
        assert!(scheduler.complete_stage(pages[0], Stage::Inpainting));
        assert_eq!(scheduler.start_next(), Some((pages[1], Stage::Ocr)));
    }

    #[test]
    fn cloud_pages_are_bounded_and_refill_after_out_of_order_completion() {
        let pages = pages(8);
        let mut scheduler = Scheduler::new(&pages, &[Stage::Translation], |_| 6);
        for page in &pages[..6] {
            assert_eq!(scheduler.start_next(), Some((*page, Stage::Translation)));
        }
        assert!(scheduler.start_next().is_none());
        assert!(scheduler.complete_stage(pages[3], Stage::Translation));
        assert_eq!(scheduler.start_next(), Some((pages[6], Stage::Translation)));
        assert!(scheduler.start_next().is_none());
        assert_eq!(scheduler.running[&Stage::Translation], 6);
    }

    #[test]
    fn stalled_cloud_requests_do_not_stop_local_stage_queues() {
        let pages = pages(30);
        let mut scheduler = Scheduler::new(&pages, &Stage::ALL, |stage| {
            if stage == Stage::Translation { 6 } else { 1 }
        });
        let mut completed = BTreeMap::<Stage, usize>::new();
        let mut requests = Vec::new();
        while let Some((page, stage)) = scheduler.start_next() {
            if stage == Stage::Translation {
                requests.push(page);
            } else {
                scheduler.complete_stage(page, stage);
                *completed.entry(stage).or_default() += 1;
            }
        }
        for stage in [Stage::Detection, Stage::Ocr, Stage::Inpainting] {
            assert_eq!(completed[&stage], 30);
        }
        assert_eq!(requests, pages[..6]);
        assert!(scheduler.start_next().is_none());
        assert!(scheduler.complete_stage(requests[3], Stage::Translation));
        assert_eq!(scheduler.start_next(), Some((pages[6], Stage::Translation)));
    }

    #[test]
    fn local_translation_stays_serial() {
        let pages = pages(2);
        let mut scheduler = Scheduler::new(&pages, &[Stage::Translation], |_| 1);
        assert_eq!(scheduler.start_next(), Some((pages[0], Stage::Translation)));
        assert!(scheduler.start_next().is_none());
        assert!(scheduler.complete_stage(pages[0], Stage::Translation));
        assert_eq!(scheduler.start_next(), Some((pages[1], Stage::Translation)));
    }
}
