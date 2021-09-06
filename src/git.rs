//! Git utilities

use std::ffi::OsStr;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use crate::fs::remove_dir_all;
use crate::{cmd, cmd_output};

pub const GIT: &str = "git";

pub struct Repository {
    git_dir: PathBuf,
    worktree: PathBuf,
    remote_name: Option<String>,
}

impl Repository {
    pub fn new(dir: impl AsRef<Path>) -> Repository {
        Repository {
            git_dir: dir.as_ref().to_owned().join(".git"),
            worktree: dir.as_ref().to_owned(),
            remote_name: None,
        }
    }

    /// Get the remote name from which this repository was cloned.
    pub fn origin(&self) -> Option<&String> {
        self.remote_name.as_ref()
    }

    fn git_args(&self) -> [&OsStr; 4] {
        [
            OsStr::new("--git-dir"),
            self.git_dir.as_os_str(),
            OsStr::new("--work-tree"),
            self.worktree.as_os_str(),
        ]
    }

    /// Get all remote names and their urls.
    pub fn get_remotes(&self) -> Result<Vec<(String, String)>> {
        Ok(cmd_output!("git", @self.git_args(), "remote", "show")?
            .lines()
            .filter_map(|l| {
                let remote = l.trim().to_owned();
                cmd_output!(GIT, @self.git_args(), "remote", "get-url", &remote)
                    .ok()
                    .map(|url| (remote, url))
            })
            .collect())
    }

    /// Get the default branch name of `remote`.
    pub fn get_default_branch_of(&self, remote: &str) -> Result<String> {
        cmd_output!(GIT, @self.git_args(), "symbolic-ref", format!("refs/remotes/{}/HEAD", remote))?
            .rsplit('/')
            .next()
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("'git symbolic-ref' yielded invalid output"))
    }

    /// Get the default branch of this repository's origin.
    ///
    /// Returns [`None`] if [`Self::origin`] returns [`None`].
    pub fn get_default_branch(&self) -> Result<Option<String>> {
        if let Some(r) = self.origin() {
            Ok(Some(self.get_default_branch_of(r)?))
        } else {
            Ok(None)
        }
    }

    /// Query whether the work-tree is clean ignoring any untracked files and recursing
    /// through all submodules.
    pub fn is_clean(&self) -> Result<bool> {
        Ok(
            cmd_output!(GIT, @self.git_args(), "status", "-s", "-uno", "--ignore-submodules=untracked", "--ignored=no")?
                .trim()
                .is_empty()
        )
    }

    /// Get a human readable name based on all available refs in the `refs/` namespace.
    ///
    /// Calls `git describe --all --exact-match`.
    pub fn describe(&self) -> Result<String> {
        cmd_output!(GIT, @self.git_args(), "describe", "--all", "--exact-match")
    }

    /// Clone the repository with the default options and return if the repository was modified.
    pub fn clone(&mut self, url: &str) -> Result<bool> {
        self.clone_ext(url, CloneOptions::default())
    }

    /// Clone the repository with `options` and return if the repository was modified.
    pub fn clone_ext(&mut self, url: &str, options: CloneOptions) -> Result<bool> {
        let (should_remove, should_clone, modified) = if !self.git_dir.exists() {
            (self.worktree.exists(), true, true)
        } else if let Some((remote, _)) = self
            .get_remotes()
            .ok()
            .and_then(|r| r.into_iter().find(|(_, r_url)| r_url == url))
        {
            let force_ref = if let Some(force_ref) = &options.force_ref {
                force_ref.clone()
            } else {
                Ref::Branch(self.get_default_branch_of(&remote)?)
            };
            self.remote_name = Some(remote);

            match force_ref {
                Ref::Branch(b) => {
                    if self.describe()? == format!("heads/{}", b)
                        && (!options.force_clean || self.is_clean()?)
                    {
                        let modified = if let Some(reset_mode) = options.branch_update_action {
                            cmd!(GIT, @self.git_args(), "reset", reset_mode.to_string())?;
                            cmd!(GIT, @self.git_args(), "pull", "--ff-only")?;
                            true
                        } else {
                            false
                        };

                        (false, false, modified)
                    } else {
                        (true, true, true)
                    }
                }
                Ref::Tag(t) => {
                    if self.describe()? == format!("tags/{}", t)
                        && (!options.force_clean || self.is_clean()?)
                    {
                        (false, false, false)
                    } else {
                        (true, true, true)
                    }
                }
                Ref::Commit(c) => {
                    if cmd_output!(GIT, @self.git_args(), "rev-parse", "HEAD")? == c
                        && (!options.force_clean || self.is_clean()?)
                    {
                        (false, false, false)
                    } else {
                        (true, true, true)
                    }
                }
            }
        } else {
            (true, true, true)
        };

        if should_remove {
            remove_dir_all(&self.worktree)?;
        }

        if should_clone {
            let (depth, branch) = match &options.force_ref {
                None | Some(Ref::Commit(_)) => (None, None),
                Some(Ref::Branch(s) | Ref::Tag(s)) => (
                    options.depth.map(|i| ["--depth".to_owned(), i.to_string()]),
                    Some(["--branch", s]),
                ),
            };

            let depth = depth.iter().flatten();
            let branch = branch.iter().flatten();

            cmd!(GIT, "clone", "--recursive", @depth, @branch, &url, &self.worktree)?;

            if let Some(Ref::Commit(s)) = options.force_ref {
                cmd!(GIT, @self.git_args(), "checkout", s)?;
            }
            self.remote_name = Some(String::from("origin"));
        }

        Ok(modified)
    }

    /// Apply all patches to this repository.
    pub fn apply(&self, patches: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Result<()> {
        cmd!(GIT, @self.git_args(), "apply"; args=(patches.into_iter()))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
    Merge,
    Keep,
}

impl std::fmt::Display for ResetMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Soft => "--soft",
            Self::Mixed => "--mixed",
            Self::Hard => "--hard",
            Self::Merge => "--merge",
            Self::Keep => "--keep",
        })
    }
}

#[derive(Debug, Clone)]
pub enum Ref {
    Tag(String),
    Branch(String),
    Commit(String),
}

#[derive(Debug)]
pub struct CloneOptions {
    /// Force the working directory to be this specific tag, branch or commit.
    ///
    /// TODO: document what it does (ie. commit missmatch, branch/tag missmatch).
    pub force_ref: Option<Ref>,
    /// The mode that is passed to `git reset` when the branch is updated.
    /// If `None` that working directory with branch is never updated.
    pub branch_update_action: Option<ResetMode>,
    /// If the working directory is not clean and `force_clean` is `true`, the git repo
    /// will be cloned from scratch.
    pub force_clean: bool,
    /// The depth that should be cloned, if `None` the full repository is cloned.
    pub depth: Option<NonZeroU64>,
}

impl Default for CloneOptions {
    fn default() -> Self {
        Self {
            force_ref: None,
            branch_update_action: None,
            force_clean: false,
            depth: None,
        }
    }
}

impl CloneOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn force_ref(mut self, force_ref: Ref) -> Self {
        self.force_ref = Some(force_ref);
        self
    }

    pub fn branch_update_action(mut self, reset_mode: ResetMode) -> Self {
        self.branch_update_action = Some(reset_mode);
        self
    }

    pub fn force_clean(mut self) -> Self {
        self.force_clean = true;
        self
    }

    pub fn depth(mut self, depth: u64) -> Self {
        self.depth = Some(NonZeroU64::new(depth).expect("depth must be greater than zero"));
        self
    }
}
