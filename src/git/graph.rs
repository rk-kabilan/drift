//! Commit history for the graph view: one page of commits at a time, with the
//! lanes needed to draw the graph, and a commit's changed files on demand.

use std::collections::{BinaryHeap, HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use git2::{Delta, DiffFindOptions, DiffOptions, Oid, Repository};

use super::repo::{decode, short};
use super::ChangeKind;

/// Commits loaded per page; more load as the list is scrolled.
pub const PAGE: usize = 100;
/// Lanes beyond this are drawn on the last one, so a very wide history can't
/// push the commit text out of view.
pub const MAX_LANES: u8 = 12;

/// A ref pointing at a commit, shown as a label next to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefLabel {
    pub name: String,
    pub kind: RefKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    /// The checked-out branch.
    Head,
    Local,
    Remote,
    Tag,
}

/// One commit row of the graph.
#[derive(Clone, Debug)]
pub struct GraphCommit {
    pub oid: Oid,
    pub summary: Box<str>,
    pub author: Box<str>,
    pub time: i64,
    /// Lane of the commit's dot.
    pub lane: u8,
    /// Lines in the top half of the row: (lane at the top edge, lane at the middle).
    pub top: Box<[(u8, u8)]>,
    /// Lines in the bottom half: (lane at the middle, lane at the bottom edge).
    pub bottom: Box<[(u8, u8)]>,
    pub refs: Box<[RefLabel]>,
}

impl GraphCommit {
    pub fn short_sha(&self) -> String {
        short(&self.oid)
    }
}

/// Where the graph left off, so the next page continues its lanes.
///
/// The walk is lazy: it keeps only the commits waiting to be shown (newest
/// first) and the ones already queued, so a page reads about a page of
/// commits however long the history is. (libgit2's own revwalk reads the whole
/// history before returning the first commit.)
#[derive(Clone, Debug, Default)]
pub struct GraphCursor {
    /// Commits already loaded.
    pub loaded: usize,
    /// For each lane, the commit it is heading to.
    lanes: Vec<Option<Oid>>,
    /// Commits waiting to be shown, by commit time.
    queue: BinaryHeap<(i64, Oid)>,
    /// Every commit ever queued, so merges don't show a commit twice.
    queued: HashSet<Oid>,
    started: bool,
    /// The walk ran out of commits.
    pub done: bool,
}

/// A file changed by a commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitFile {
    pub path: String,
    /// The path in the parent, when the file was renamed.
    pub old_path: Option<String>,
    pub kind: ChangeKind,
}

impl CommitFile {
    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    pub fn dir(&self) -> &str {
        match self.path.rfind('/') {
            Some(ix) => &self.path[..ix],
            None => "",
        }
    }
}

/// The tips the graph starts from: HEAD and its upstream, so incoming commits
/// show after a fetch. Also used to tell when the graph needs reloading.
pub(super) fn tips(repo: &Repository) -> Vec<Oid> {
    let mut tips = Vec::new();
    let Ok(head) = repo.head() else {
        return tips;
    };
    if let Some(oid) = head.target() {
        tips.push(oid);
    }
    if head.is_branch() {
        if let Some(name) = head.shorthand() {
            if let Ok(branch) = repo.find_branch(name, git2::BranchType::Local) {
                if let Some(up) = branch.upstream().ok().and_then(|u| u.get().target()) {
                    if !tips.contains(&up) {
                        tips.push(up);
                    }
                }
            }
        }
    }
    tips
}

/// Loads the next page of history after `cursor`, and advances it.
pub(super) fn page(repo: &Repository, cursor: &mut GraphCursor) -> Result<Vec<GraphCommit>> {
    let tips = tips(repo);
    if tips.is_empty() || cursor.done {
        cursor.done = true;
        return Ok(Vec::new());
    }
    if !cursor.started {
        cursor.started = true;
        for tip in tips {
            if let Ok(c) = repo.find_commit(tip) {
                if cursor.queued.insert(tip) {
                    cursor.queue.push((c.time().seconds(), tip));
                }
            }
        }
    }
    let mut oids = Vec::with_capacity(PAGE);
    while oids.len() < PAGE {
        let Some((_, oid)) = cursor.queue.pop() else {
            cursor.done = true;
            break;
        };
        let commit = repo.find_commit(oid)?;
        for parent in commit.parents() {
            if cursor.queued.insert(parent.id()) {
                cursor.queue.push((parent.time().seconds(), parent.id()));
            }
        }
        oids.push(oid);
    }
    let labels = ref_labels(repo, &oids);
    let mut out = Vec::with_capacity(oids.len());
    for oid in oids {
        let commit = repo.find_commit(oid)?;
        let parents: Vec<Oid> = commit.parent_ids().collect();
        let (lane, top, bottom) = place(&mut cursor.lanes, oid, &parents);
        out.push(GraphCommit {
            oid,
            summary: commit.summary().unwrap_or("").into(),
            author: commit.author().name().unwrap_or("unknown").into(),
            time: commit.time().seconds(),
            lane,
            top: top.into(),
            bottom: bottom.into(),
            refs: labels.get(&oid).cloned().unwrap_or_default().into(),
        });
    }
    cursor.loaded += out.len();
    Ok(out)
}

type Edges = Vec<(u8, u8)>;

/// Puts a commit on a lane and works out the lines above and below its dot.
/// `lanes` holds, for each lane, the commit it leads to; it is updated for the
/// next row.
fn place(lanes: &mut Vec<Option<Oid>>, oid: Oid, parents: &[Oid]) -> (u8, Edges, Edges) {
    let clamp = |i: usize| (i.min(MAX_LANES as usize - 1)) as u8;
    let lane = match lanes.iter().position(|l| *l == Some(oid)) {
        Some(i) => i,
        None => free_lane(lanes),
    };

    // Lanes heading elsewhere pass straight through this row.
    let passing: Vec<usize> = lanes
        .iter()
        .enumerate()
        .filter(|(_, l)| l.is_some_and(|t| t != oid))
        .map(|(i, _)| i)
        .collect();

    // Top half: every lane coming in. Those heading to this commit bend into it.
    let mut top = Vec::new();
    for (i, l) in lanes.iter().enumerate() {
        match l {
            Some(target) if *target == oid => top.push((clamp(i), clamp(lane))),
            Some(_) => top.push((clamp(i), clamp(i))),
            None => {}
        }
    }
    for l in lanes.iter_mut() {
        if *l == Some(oid) {
            *l = None;
        }
    }

    // Bottom half: the first parent continues this lane; other parents join a
    // lane already heading to them, or start a new one.
    let mut bottom = Vec::new();
    let mut targets: Vec<usize> = Vec::new();
    if let Some(first) = parents.first() {
        if let Some(i) = lanes.iter().position(|l| *l == Some(*first)) {
            // Another lane already leads to the first parent: merge into it.
            targets.push(i);
        } else {
            lanes[lane] = Some(*first);
            targets.push(lane);
        }
    }
    for p in parents.iter().skip(1) {
        let i = match lanes.iter().position(|l| *l == Some(*p)) {
            Some(i) => i,
            None => {
                let i = free_lane(lanes);
                lanes[i] = Some(*p);
                i
            }
        };
        targets.push(i);
    }
    for &i in &passing {
        bottom.push((clamp(i), clamp(i)));
    }
    for t in targets {
        let edge = (clamp(lane), clamp(t));
        if !bottom.contains(&edge) {
            bottom.push(edge);
        }
    }
    while lanes.last() == Some(&None) {
        lanes.pop();
    }
    (clamp(lane), top, bottom)
}

fn free_lane(lanes: &mut Vec<Option<Oid>>) -> usize {
    match lanes.iter().position(Option::is_none) {
        Some(i) => i,
        None => {
            lanes.push(None);
            lanes.len() - 1
        }
    }
}

/// Branch and tag names for the given commits.
fn ref_labels(repo: &Repository, oids: &[Oid]) -> HashMap<Oid, Vec<RefLabel>> {
    let wanted: std::collections::HashSet<Oid> = oids.iter().copied().collect();
    let head = repo
        .head()
        .ok()
        .filter(|h| h.is_branch())
        .and_then(|h| h.name().map(str::to_string));
    let mut out: HashMap<Oid, Vec<RefLabel>> = HashMap::new();
    let Ok(refs) = repo.references() else {
        return out;
    };
    for r in refs.flatten() {
        let Some(name) = r.name() else { continue };
        let (kind, short_name) = if let Some(n) = name.strip_prefix("refs/heads/") {
            let kind = if head.as_deref() == Some(name) {
                RefKind::Head
            } else {
                RefKind::Local
            };
            (kind, n)
        } else if let Some(n) = name.strip_prefix("refs/remotes/") {
            if n.ends_with("/HEAD") {
                continue;
            }
            (RefKind::Remote, n)
        } else if let Some(n) = name.strip_prefix("refs/tags/") {
            (RefKind::Tag, n)
        } else {
            continue;
        };
        let Some(oid) = r.peel_to_commit().ok().map(|c| c.id()) else {
            continue;
        };
        if wanted.contains(&oid) {
            out.entry(oid).or_default().push(RefLabel {
                name: short_name.to_string(),
                kind,
            });
        }
    }
    for labels in out.values_mut() {
        labels.sort_by_key(|l| match l.kind {
            RefKind::Head => 0,
            RefKind::Local => 1,
            RefKind::Remote => 2,
            RefKind::Tag => 3,
        });
    }
    out
}

/// Files a commit changed, compared with its first parent.
pub(super) fn commit_files(repo: &Repository, oid: Oid) -> Result<Vec<CommitFile>> {
    let commit = repo.find_commit(oid)?;
    let tree = commit.tree()?;
    let parent_tree = commit.parent(0).ok().map(|p| p.tree()).transpose()?;
    let mut opts = DiffOptions::new();
    opts.include_typechange(true);
    let mut diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))?;
    diff.find_similar(Some(DiffFindOptions::new().renames(true)))?;
    let mut out = Vec::new();
    for delta in diff.deltas() {
        let path = |f: git2::DiffFile| f.path().map(|p| p.to_string_lossy().replace('\\', "/"));
        let new_path = path(delta.new_file());
        let old_path = path(delta.old_file());
        let kind = match delta.status() {
            Delta::Added | Delta::Copied => ChangeKind::Added,
            Delta::Deleted => ChangeKind::Deleted,
            Delta::Renamed => ChangeKind::Renamed,
            _ => ChangeKind::Modified,
        };
        let Some(p) = new_path.clone().or(old_path.clone()) else {
            continue;
        };
        out.push(CommitFile {
            old_path: (kind == ChangeKind::Renamed).then_some(old_path).flatten(),
            path: p,
            kind,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// A file's text before and after a commit (empty where it doesn't exist).
pub(super) fn commit_texts(
    repo: &Repository,
    oid: Oid,
    path: &str,
    old_path: Option<&str>,
) -> Result<(String, String)> {
    let commit = repo.find_commit(oid)?;
    let read = |tree: Option<git2::Tree>, path: &str| -> String {
        tree.and_then(|t| t.get_path(Path::new(path)).ok())
            .and_then(|e| repo.find_blob(e.id()).ok())
            .map(|b| decode(b.content()))
            .unwrap_or_default()
    };
    let new = read(commit.tree().ok(), path);
    let old = read(
        commit.parent(0).ok().and_then(|p| p.tree().ok()),
        old_path.unwrap_or(path),
    );
    Ok((old, new))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(n: u8) -> Oid {
        Oid::from_bytes(&[n; 20]).unwrap()
    }

    #[test]
    fn straight_history_stays_on_one_lane() {
        let mut lanes = Vec::new();
        let (l, top, bottom) = place(&mut lanes, oid(3), &[oid(2)]);
        assert_eq!((l, top.len(), bottom), (0, 0, vec![(0, 0)]));
        let (l, top, bottom) = place(&mut lanes, oid(2), &[oid(1)]);
        assert_eq!((l, top, bottom), (0, vec![(0, 0)], vec![(0, 0)]));
        let (l, _, bottom) = place(&mut lanes, oid(1), &[]);
        assert_eq!((l, bottom.len()), (0, 0));
        assert!(lanes.is_empty());
    }

    #[test]
    fn merge_opens_and_closes_a_lane() {
        // 4 merges 3 (first parent) and 2; both come from 1.
        let mut lanes = Vec::new();
        let (_, _, bottom) = place(&mut lanes, oid(4), &[oid(3), oid(2)]);
        assert_eq!(bottom, vec![(0, 0), (0, 1)]);
        let (l, _, _) = place(&mut lanes, oid(3), &[oid(1)]);
        assert_eq!(l, 0);
        let (l, top, bottom) = place(&mut lanes, oid(2), &[oid(1)]);
        assert_eq!(l, 1);
        assert_eq!(top, vec![(0, 0), (1, 1)]);
        // Lane 1 merges back into lane 0, which already heads to 1.
        assert_eq!(bottom, vec![(0, 0), (1, 0)]);
        let (l, top, _) = place(&mut lanes, oid(1), &[]);
        assert_eq!((l, top), (0, vec![(0, 0)]));
    }
}
