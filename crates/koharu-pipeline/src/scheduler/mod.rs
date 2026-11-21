//! The task that owns the runs, so nothing is locked. It admits ready jobs
//! while a permit for a page in flight is free, each a Tokio task in one
//! `JoinSet`, and hears how each went from it, a panic included. Calls wait
//! for the GPU thread in its own line, by priority. The stages are known here
//! only by `Stage` and by the model `stages::key` names.

mod run;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use koharu_scene::{PageId, ProjectId};
use parking_lot::RwLock;
use petgraph::{Direction::Incoming, stable_graph::NodeIndex};
use tokio::{
    runtime::Handle,
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot},
    task::{self, JoinError, JoinSet},
    time::{self, Instant},
};

pub(crate) use self::run::{Plan, Run};
use crate::{
    Event, Outcome, PipelineConfig, RunId, Stage,
    event::Events,
    gpu::{Gpu, Key, Turn},
    stages::{self, Inputs, Member, Task, translate::Conversations},
};

/// Pages admitted and not yet written: one reading its input, one on the
/// GPU, one writing its result, so the CPU works beside the GPU while few
/// pages are held.
const AHEAD: usize = 3;
/// The most pages one recognition call reads.
const RECOGNIZE_PAGES: usize = 8;
/// Batch pages may make at most six independent hosted requests at once.
const HOSTED_BATCH_REQUESTS: usize = 6;
/// How long a model no setting uses, one a run brought as Generative Fill's
/// does, stays after a run last ended, as Ollama keeps a model five minutes
/// after its last request, so filling again doesn't load it again.
const KEEP: Duration = Duration::from_secs(5 * 60);

pub(crate) enum Command {
    Run(Run),
    Pause(RunId, bool, oneshot::Sender<bool>),
    StopRun(RunId),
    UnloadWhenIdle,
    Stop,
    Unload,
    /// The settings changed: unload what nothing uses now.
    Settle,
    Close(ProjectId),
}

/// How a run is asked for, as BigQuery runs a job interactive or batch: an
/// interactive run, on blocks or an area, goes ahead of a batch run on pages.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Mode {
    Interactive,
    Batch,
}

/// A job's place in line; the smallest goes first.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Priority {
    mode: Mode,
    run: RunId,
    page: usize,
    stage: Stage,
}

/// A ready job: its run's place among the runs, and its node.
#[derive(Clone, Copy)]
struct Candidate {
    run: usize,
    job: NodeIndex,
    stage: Stage,
    priority: Priority,
}

/// A running task's stage, its jobs, and the page it fails on.
struct Running {
    stage: Stage,
    jobs: Vec<(RunId, NodeIndex)>,
    page: PageId,
}

pub(crate) struct Scheduler {
    commands: mpsc::UnboundedReceiver<Command>,
    events: Events,
    gpu: Gpu,
    config: Arc<RwLock<PipelineConfig>>,
    /// In the order they were asked for.
    runs: Vec<Run>,
    /// A permit for each page in flight, which its task holds.
    ahead: Arc<Semaphore>,
    tasks: JoinSet<Result<(), String>>,
    running: HashMap<task::Id, Running>,
    conversations: Conversations,
    /// When the models no setting uses leave: `KEEP` after a run last ended.
    idle: Option<Instant>,
    unload_when_idle: bool,
}

impl Scheduler {
    /// Starts the scheduler and the GPU thread. They end when every sender
    /// the pipeline holds drops.
    pub(crate) fn spawn(
        config: Arc<RwLock<PipelineConfig>>,
        events: Events,
    ) -> mpsc::UnboundedSender<Command> {
        let (sender, commands) = mpsc::unbounded_channel();
        let scheduler = Self {
            commands,
            events,
            gpu: Gpu::spawn(Handle::current()),
            config,
            runs: Vec::new(),
            ahead: Arc::new(Semaphore::new(AHEAD)),
            tasks: JoinSet::new(),
            running: HashMap::new(),
            conversations: Conversations::default(),
            idle: None,
            unload_when_idle: false,
        };
        tokio::spawn(scheduler.run());
        sender
    }

    async fn run(mut self) {
        loop {
            tokio::select! {
                command = self.commands.recv() => match command {
                    Some(command) => self.handle(command),
                    None => break,
                },
                Some(joined) = self.tasks.join_next_with_id() => self.done(joined),
                () = time::sleep_until(self.idle.unwrap_or_else(Instant::now)),
                    if self.idle.is_some() =>
                {
                    self.idle = None;
                    self.settle();
                }
            }
            self.admit();
            self.acknowledge_pauses();
            self.unload_if_idle();
        }
        // The pipeline is gone: what's running stops, and nothing is written.
        for run in &self.runs {
            run.cancel.cancel();
        }
        while self.tasks.join_next().await.is_some() {}
    }

    fn emit(&self, event: Event) {
        self.events.send(event);
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Run(run) if run.graph.node_count() == 0 => self.emit(Event::Ended {
                run: run.id,
                outcome: Outcome::Done,
            }),
            Command::Run(run) => self.runs.push(run),
            Command::Pause(id, paused, reply) => {
                if let Some(run) = self.runs.iter_mut().find(|run| run.id == id) {
                    run.paused = paused;
                    if paused {
                        run.pause_waiters.push(reply);
                    } else {
                        for waiter in run.pause_waiters.drain(..) {
                            let _ = waiter.send(false);
                        }
                        let _ = reply.send(true);
                    }
                } else {
                    let _ = reply.send(false);
                }
            }
            Command::StopRun(id) => {
                if let Some(index) = self.runs.iter().position(|run| run.id == id) {
                    self.runs[index].cancel.cancel();
                    self.runs[index].paused = true;
                    for waiter in self.runs[index].pause_waiters.drain(..) {
                        let _ = waiter.send(false);
                    }
                    if !self
                        .running
                        .values()
                        .any(|task| task.jobs.iter().any(|(run, _)| *run == id))
                    {
                        self.runs.remove(index);
                        self.emit(Event::Ended {
                            run: id,
                            outcome: Outcome::Stopped,
                        });
                        self.rest();
                    }
                }
            }
            Command::Stop => {
                // Each task's call waiting for the GPU is dropped unrun.
                for run in std::mem::take(&mut self.runs) {
                    run.cancel.cancel();
                    self.emit(Event::Ended {
                        run: run.id,
                        outcome: Outcome::Stopped,
                    });
                }
                self.rest();
            }
            Command::UnloadWhenIdle => {
                self.unload_when_idle = true;
                self.unload_if_idle();
            }
            Command::Unload => self
                .gpu
                .run(Turn::Upkeep, |models| models.retain(&HashSet::new())),
            Command::Settle => self.settle(),
            Command::Close(project) => self.conversations.close(project),
        }
    }

    fn done(&mut self, joined: Result<(task::Id, Result<(), String>), JoinError>) {
        let id = match &joined {
            Ok((id, _)) => *id,
            Err(error) => error.id(),
        };
        let Running { stage, jobs, page } = self
            .running
            .remove(&id)
            .expect("every task is recorded when it starts");
        let result = match joined {
            Ok((_, result)) => result,
            Err(error) => Err(format!("{stage} failed: {error}")),
        };
        let runs = std::mem::take(&mut self.runs);
        let mut ended = false;
        match result {
            Ok(()) => {
                for mut run in runs {
                    if run.cancel.is_cancelled() {
                        if self
                            .running
                            .values()
                            .any(|task| task.jobs.iter().any(|(id, _)| *id == run.id))
                        {
                            self.runs.push(run);
                        } else {
                            ended = true;
                            self.emit(Event::Ended {
                                run: run.id,
                                outcome: Outcome::Stopped,
                            });
                        }
                        continue;
                    }
                    for &(id, job) in &jobs {
                        if id == run.id
                            && let Some(job) = run.graph.remove_node(job)
                        {
                            self.emit(Event::Finished {
                                run: id,
                                page: job.page,
                                stage: job.stage,
                            });
                        }
                    }
                    if run.graph.node_count() == 0 {
                        ended = true;
                        self.emit(Event::Ended {
                            run: run.id,
                            outcome: Outcome::Done,
                        });
                    } else {
                        self.runs.push(run);
                    }
                }
            }
            Err(reason) => {
                for run in runs {
                    if run.cancel.is_cancelled() {
                        if self
                            .running
                            .values()
                            .any(|task| task.jobs.iter().any(|(id, _)| *id == run.id))
                        {
                            self.runs.push(run);
                        } else {
                            ended = true;
                            self.emit(Event::Ended {
                                run: run.id,
                                outcome: Outcome::Stopped,
                            });
                        }
                        continue;
                    }
                    if jobs.iter().any(|&(id, _)| id == run.id) {
                        ended = true;
                        run.cancel.cancel();
                        self.emit(Event::Ended {
                            run: run.id,
                            outcome: Outcome::Failed {
                                page,
                                stage,
                                reason: reason.clone(),
                            },
                        });
                    } else {
                        self.runs.push(run);
                    }
                }
            }
        }
        if ended {
            self.rest();
        }
    }

    /// A run ended: the models no setting uses leave `KEEP` from now, and
    /// libtorch's cache now, if no run is left.
    fn rest(&mut self) {
        self.idle = Some(Instant::now() + KEEP);
        if self.runs.is_empty() {
            self.gpu.empty_cache();
        }
        self.unload_if_idle();
    }

    fn unload_if_idle(&mut self) {
        if self.unload_when_idle && self.runs.is_empty() && self.running.is_empty() {
            self.unload_when_idle = false;
            self.gpu
                .run(Turn::Upkeep, |models| models.retain(&HashSet::new()));
        }
    }

    fn acknowledge_pauses(&mut self) {
        for run in &mut self.runs {
            if run.paused
                && !self
                    .running
                    .values()
                    .any(|task| task.jobs.iter().any(|(id, _)| *id == run.id))
            {
                for waiter in run.pause_waiters.drain(..) {
                    let _ = waiter.send(true);
                }
            }
        }
    }

    /// Unloads the models neither the settings nor a queued run use, once
    /// the GPU's current call ends.
    fn settle(&self) {
        let mut wanted: HashSet<Key> = stages::chosen(&self.config.read()).collect();
        for run in &self.runs {
            wanted.extend(stages::chosen(&run.config));
        }
        self.gpu
            .run(Turn::Upkeep, move |models| models.retain(&wanted));
    }

    /// The ready jobs not yet started. A project's translation isn't ready
    /// while another holds its conversation.
    fn ready(&self) -> Vec<Candidate> {
        self.runs
            .iter()
            .enumerate()
            .flat_map(|(at, run)| {
                let translating = run
                    .graph
                    .node_weights()
                    .filter(|job| job.stage == Stage::Translate && job.running)
                    .count();
                run.graph.externals(Incoming).filter_map(move |job| {
                    let node = &run.graph[job];
                    let held = node.stage == Stage::Translate
                        && if run.independent_translation {
                            translating >= HOSTED_BATCH_REQUESTS
                        } else {
                            self.conversations.busy(run.project)
                        };
                    (!run.paused && !node.running && !held).then_some(Candidate {
                        run: at,
                        job,
                        stage: node.stage,
                        priority: Priority {
                            mode: run.mode,
                            run: run.id,
                            page: node.index,
                            stage: node.stage,
                        },
                    })
                })
            })
            .collect()
    }

    fn admit(&mut self) {
        loop {
            let ready = self.ready();
            // A stage with no model of Koharu's to load, a hosted
            // translation, doesn't wait for the GPU, so it isn't counted
            // among the pages in flight.
            let hosted = ready
                .iter()
                .filter(|candidate| {
                    stages::key(candidate.stage, &self.runs[candidate.run].config).is_none()
                })
                .min_by_key(|candidate| candidate.priority);
            if let Some(&hosted) = hosted {
                self.start(vec![hosted], None);
                continue;
            }
            let Some(&first) = ready.iter().min_by_key(|candidate| candidate.priority) else {
                return;
            };
            let Ok(permit) = Arc::clone(&self.ahead).try_acquire_owned() else {
                return;
            };
            let mut chosen = vec![first];
            if first.stage == Stage::Recognize {
                let model = stages::key(first.stage, &self.runs[first.run].config);
                let mut others: Vec<Candidate> = ready
                    .iter()
                    .filter(|other| {
                        other.stage == Stage::Recognize
                            && other.priority != first.priority
                            && stages::key(other.stage, &self.runs[other.run].config) == model
                    })
                    .copied()
                    .collect();
                others.sort_by_key(|other| other.priority);
                chosen.extend(others.into_iter().take(RECOGNIZE_PAGES - 1));
            }
            self.start(chosen, Some(permit));
        }
    }

    fn start(&mut self, chosen: Vec<Candidate>, permit: Option<OwnedSemaphorePermit>) {
        let members: Vec<Member> = chosen
            .iter()
            .map(|candidate| {
                let run = &mut self.runs[candidate.run];
                let job = &mut run.graph[candidate.job];
                job.running = true;
                Member {
                    run: run.id,
                    job: candidate.job,
                    page: job.page,
                    index: job.index,
                    blocks: job.blocks.clone(),
                    session: Arc::clone(&run.session),
                    cancel: run.cancel.clone(),
                }
            })
            .collect();
        let first = chosen[0];
        let run = &self.runs[first.run];
        let inputs = Inputs {
            config: Arc::clone(&run.config),
            area: run.area.clone(),
            conversation: (first.stage == Stage::Translate).then(|| {
                if run.independent_translation {
                    Conversations::independent()
                } else {
                    self.conversations
                        .lock(run.project)
                        .expect("a translation is ready only while its conversation is free")
                }
            }),
        };
        let running = Running {
            stage: first.stage,
            jobs: members
                .iter()
                .map(|member| (member.run, member.job))
                .collect(),
            page: members[0].page,
        };
        let task = Task {
            stage: first.stage,
            members,
            priority: first.priority,
            gpu: self.gpu.clone(),
            events: self.events.clone(),
        };
        let handle = self.tasks.spawn(async move {
            // The page leaves the flight when its task ends, however it ends.
            let _permit = permit;
            stages::run(task, inputs).await
        });
        self.running.insert(handle.id(), running);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scheduler_with_pages(count: usize) -> anyhow::Result<(tempfile::TempDir, Scheduler)> {
        let directory = tempfile::tempdir()?;
        let mut session = koharu_scene::Session::create(directory.path().join("test.khr"))?;
        let ops = (0..count)
            .map(|index| {
                serde_json::from_value(serde_json::json!({
                    "op": "add_page", "index": index, "size": {"width": 100, "height": 100}
                }))
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        session.apply(koharu_scene::Edit {
            label: "test".into(),
            ops,
        })?;
        let project = session.project();
        let stages = std::collections::BTreeSet::from([Stage::Translate]);
        let plan = Plan::new(&project, &crate::Target::Project, &stages)?;
        let run = Run::new(
            RunId(0),
            Arc::new(parking_lot::Mutex::new(session)),
            project.id(),
            PipelineConfig::default(),
            plan,
            &stages,
            true,
        );
        let (_, commands) = mpsc::unbounded_channel();
        Ok((
            directory,
            Scheduler {
                commands,
                events: Events::new(),
                gpu: Gpu::spawn(Handle::current()),
                config: Arc::new(RwLock::new(PipelineConfig::default())),
                runs: vec![run],
                ahead: Arc::new(Semaphore::new(AHEAD)),
                tasks: JoinSet::new(),
                running: HashMap::new(),
                conversations: Conversations::default(),
                idle: None,
                unload_when_idle: false,
            },
        ))
    }

    #[tokio::test]
    async fn independent_cloud_requests_stop_at_six_and_refill() -> anyhow::Result<()> {
        let (_directory, mut scheduler) = scheduler_with_pages(7)?;
        let jobs: Vec<_> = scheduler.runs[0].graph.node_indices().collect();
        for &job in jobs.iter().take(6) {
            scheduler.runs[0].graph[job].running = true;
        }
        assert!(scheduler.ready().is_empty());
        scheduler.runs[0].graph[jobs[0]].running = false;
        assert_eq!(scheduler.ready().len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn pause_waits_for_admitted_work_and_stop_releases_waiter() -> anyhow::Result<()> {
        let (_directory, mut scheduler) = scheduler_with_pages(1)?;
        let job = scheduler.runs[0].graph.node_indices().next().unwrap();
        let page = scheduler.runs[0].graph[job].page;
        let pending = tokio::spawn(std::future::pending::<()>());
        scheduler.running.insert(
            pending.id(),
            Running {
                stage: Stage::Translate,
                jobs: vec![(RunId(0), job)],
                page,
            },
        );
        let (reply, mut response) = oneshot::channel();
        scheduler.handle(Command::Pause(RunId(0), true, reply));
        scheduler.acknowledge_pauses();
        assert!(matches!(
            response.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert!(scheduler.ready().is_empty());
        scheduler.handle(Command::StopRun(RunId(0)));
        assert!(!response.await?);
        assert!(scheduler.runs[0].cancel.is_cancelled());
        pending.abort();
        Ok(())
    }

    #[test]
    fn interactive_runs_go_first_then_the_oldest_page_in_stage_order() {
        let pages = Priority {
            mode: Mode::Batch,
            run: RunId(0),
            page: 0,
            stage: Stage::Detect,
        };
        let interactive = Priority {
            mode: Mode::Interactive,
            run: RunId(1),
            page: 3,
            stage: Stage::Translate,
        };
        let later = Priority { page: 1, ..pages };
        let next_stage = Priority {
            stage: Stage::Recognize,
            ..pages
        };
        assert!(interactive < pages && pages < next_stage && next_stage < later);
    }
}
