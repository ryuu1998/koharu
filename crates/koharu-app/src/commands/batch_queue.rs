use std::{
    collections::{BTreeSet, VecDeque},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Clone, Debug, Deserialize, Serialize, Type)]
pub struct BatchEntry {
    pub path: PathBuf,
    pub output: PathBuf,
}

pub(super) struct WorkItem {
    pub path: PathBuf,
    pub output: PathBuf,
}

/// Only this queue may claim work. Events identify the source, never a mutable index.
pub(super) struct Queue {
    pending: VecDeque<WorkItem>,
    history: Vec<PathBuf>,
    active: Option<PathBuf>,
    sources: BTreeSet<PathBuf>,
    outputs: BTreeSet<PathBuf>,
    accepting: bool,
}

impl Default for Queue {
    fn default() -> Self {
        Self {
            pending: VecDeque::new(),
            history: Vec::new(),
            active: None,
            sources: BTreeSet::new(),
            outputs: BTreeSet::new(),
            accepting: true,
        }
    }
}

impl Queue {
    pub(super) fn append(&mut self, entries: Vec<BatchEntry>) -> Result<()> {
        ensure!(
            self.accepting,
            "this batch has finished; start another batch"
        );
        // Validate the entire edit before changing any queue state.
        let mut sources = self.sources.clone();
        let mut outputs = self.outputs.clone();
        let mut accepted = Vec::new();
        for entry in entries {
            ensure!(
                entry.path.is_file() && super::batch_source::is_cbz(&entry.path),
                "input must be a CBZ archive"
            );
            let source = path_key(&std::fs::canonicalize(&entry.path)?);
            if !sources.insert(source) {
                continue;
            }
            let name = entry.path.file_name().context("chapter has no filename")?;
            let directory = absolute_target(&entry.output)?;
            let mut target = directory.join(name);
            // A different source with the same basename gets its own output reservation.
            let stem = entry.path.file_stem().unwrap_or_default().to_string_lossy();
            let mut suffix = 2;
            while outputs.contains(&path_key(&absolute_target(&target)?)) {
                target = directory.join(format!("{stem} ({suffix}).cbz"));
                suffix += 1;
            }
            outputs.insert(path_key(&absolute_target(&target)?));
            accepted.push(WorkItem { path: entry.path, output: target });
        }
        ensure!(
            outputs.is_disjoint(&sources),
            "choose a different output folder; source archives cannot be replaced"
        );
        self.sources = sources;
        self.outputs = outputs;
        self.pending.extend(accepted);
        Ok(())
    }

    pub(super) fn claim(&mut self) -> Option<WorkItem> {
        let entry = self.pending.pop_front();
        self.active = entry.as_ref().map(|entry| entry.path.clone());
        if entry.is_none() {
            self.accepting = false;
        }
        entry
    }

    pub(super) fn finish(&mut self) {
        if let Some(path) = self.active.take() {
            self.history.push(path);
        }
    }

    pub(super) fn reorder(&mut self, path: &Path, before: Option<&Path>) -> Result<()> {
        ensure!(self.accepting, "this batch has finished");
        let index = self
            .pending
            .iter()
            .position(|entry| entry.path == path)
            .context("only waiting chapters can be moved")?;
        if before == Some(path) {
            return Ok(());
        }
        if let Some(before) = before {
            ensure!(
                self.pending.iter().any(|entry| entry.path == before),
                "the destination chapter is no longer waiting"
            );
        }
        let entry = self.pending.remove(index).expect("validated pending index");
        let target = before
            .and_then(|before| self.pending.iter().position(|entry| entry.path == before))
            .unwrap_or(self.pending.len());
        self.pending.insert(target, entry);
        Ok(())
    }

    pub(super) fn paths(&self) -> Vec<PathBuf> {
        self.history
            .iter()
            .chain(self.active.iter())
            .cloned()
            .chain(self.pending.iter().map(|entry| entry.path.clone()))
            .collect()
    }
}

fn path_key(path: &Path) -> PathBuf {
    #[cfg(windows)]
    { PathBuf::from(path.to_string_lossy().to_lowercase()) }
    #[cfg(not(windows))]
    { path.to_owned() }
}

fn absolute_target(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return Ok(std::fs::canonicalize(path)?);
    }
    let parent = path.parent().context("output folder needs a parent")?;
    let name = path.file_name().context("invalid output folder")?;
    Ok(absolute_target(parent)?.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_active_fixed_and_reorders_only_pending_work() {
        let dir = tempfile::tempdir().unwrap();
        let mut queue = Queue::default();
        let entries: Vec<_> = ["a.cbz", "b.cbz", "c.cbz"]
            .into_iter()
            .map(|name| {
                let path = dir.path().join(name);
                std::fs::write(&path, b"archive").unwrap();
                BatchEntry {
                    path,
                    output: dir.path().join("Translated"),
                }
            })
            .collect();
        queue.append(entries.clone()).unwrap();
        assert_eq!(queue.claim().unwrap().path, entries[0].path);
        assert!(queue.reorder(&entries[0].path, None).is_err());
        queue
            .reorder(&entries[2].path, Some(&entries[1].path))
            .unwrap();
        queue.append(vec![entries[1].clone()]).unwrap();
        queue.finish();
        assert_eq!(queue.claim().unwrap().path, entries[2].path);
        queue.finish();
        assert_eq!(queue.claim().unwrap().path, entries[1].path);
        queue.finish();
        assert!(queue.claim().is_none());
        assert!(queue.append(entries).is_err());
    }

    #[test]
    fn reserves_colliding_filenames_and_rejects_source_overwrites_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        let left = a.join("chapter.cbz");
        let right = b.join("chapter.cbz");
        std::fs::write(&left, b"a").unwrap();
        std::fs::write(&right, b"b").unwrap();
        let mut queue = Queue::default();
        assert!(
            queue
                .append(vec![BatchEntry {
                    path: left.clone(),
                    output: a
                }])
                .is_err()
        );
        assert!(queue.paths().is_empty());
        queue
            .append(vec![
                BatchEntry {
                    path: left,
                    output: dir.path().join("out"),
                },
                BatchEntry {
                    path: right,
                    output: dir.path().join("out"),
                },
            ])
            .unwrap();
        assert_ne!(queue.pending[0].output, queue.pending[1].output);
    }
}
