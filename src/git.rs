//! Git utilities.
// TODO: maybe use `git2` crate

use std::ffi::OsStr;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};

use crate::utils::{CmdError, PathExt};
use crate::{cmd, cmd_output};

/// The git command.
pub const GIT: &str = "git";

/// This is a workaround for setting the locale to `C` which guarantees that the output
/// will be in english.
const LC_ALL: [(&str, &str); 1] = [("LC_ALL", "C.UTF-8")];

/// A logical git repository which may or may not exist.
pub struct Repository {
    git_dir: PathBuf,
    worktree: PathBuf,
    remote_name: Option<String>,
}

impl Repository {
    /// Create a logical repository from the git worktree `dir`.
    ///
    /// Note the git dir must be `.git`.
    pub fn new(dir: impl AsRef<Path>) -> Repository {
        Repository {
            // FIXME: the name of the git dir can be configured
            git_dir: dir.as_ref().join(".git"),
            worktree: dir.as_ref().to_owned(),
            remote_name: None,
        }
    }

    /// Try to open an existing git repository.
    pub fn open(dir: impl AsRef<Path>) -> anyhow::Result<Repository> {
        let dir = dir.as_ref();
        let base_err = || anyhow::anyhow!("'{}' is not a git respository", dir.display());

        let top_level_dir =
            cmd_output!(GIT, "rev-parse", "--show-toplevel"; current_dir=(dir), envs=(LC_ALL))
                .context(base_err())?;
        let top_level_dir = Path::new(&top_level_dir)
            .canonicalize()
            .context(base_err())?;

        if !dir
            .canonicalize()
            .map(|p| p.eq(&top_level_dir))
            .unwrap_or(false)
        {
            return Err(base_err());
        }

        let git_dir = Path::new(
            &cmd_output!(GIT, "rev-parse", "--git-dir"; current_dir=(dir), envs=(LC_ALL))?,
        )
        .abspath_relative_to(&dir);

        Ok(Repository {
            git_dir,
            worktree: dir.to_owned(),
            remote_name: None,
        })
    }

    pub fn worktree(&self) -> &Path {
        &self.worktree
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
    pub fn get_remotes(&self) -> Result<Vec<(String, String)>, CmdError> {
        Ok(
            cmd_output!(GIT, @self.git_args(), "remote", "show"; envs=(LC_ALL))?
                .lines()
                .filter_map(|l| {
                    let remote = l.trim().to_owned();
                    cmd_output!(GIT, @self.git_args(), "remote", "get-url", &remote; envs=(LC_ALL))
                        .ok()
                        .map(|url| (remote, url))
                })
                .collect(),
        )
    }

    /// Get the default branch name of `remote`.
    pub fn get_default_branch_of(&self, remote: &str) -> Result<String, anyhow::Error> {
        let output = cmd_output!(GIT, @self.git_args(), "remote", "show", remote; envs=(LC_ALL))?;
        output
            .lines()
            .map(str::trim)
            .find_map(|l| l.strip_prefix("HEAD branch: "))
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("'git remote show' yielded invalid output: '{}'", output))
    }

    /// Get the default branch of this repository's origin.
    ///
    /// Returns [`None`] if [`Self::origin`] returns [`None`].
    pub fn get_default_branch(&self) -> Result<Option<String>, anyhow::Error> {
        if let Some(r) = self.origin() {
            Ok(Some(self.get_default_branch_of(r)?))
        } else {
            Ok(None)
        }
    }

    /// Query whether the work-tree is clean ignoring any untracked files and recursing
    /// through all submodules.
    pub fn is_clean(&self) -> Result<bool, CmdError> {
        Ok(
            cmd_output!(GIT, @self.git_args(), "status", "-s", "-uno", "--ignore-submodules=untracked", "--ignored=no"; envs=(LC_ALL))?
                .trim()
                .is_empty()
        )
    }

    /// Get a human readable name based on all available refs in the `refs/` namespace.
    ///
    /// Calls `git describe --all --exact-match`.
    pub fn describe(&self) -> Result<String, CmdError> {
        cmd_output!(GIT, @self.git_args(), "describe", "--all", "--exact-match"; envs=(LC_ALL))
    }

    /// Clone the repository with the default options and return if the repository was modified.
    pub fn clone(&mut self, url: &str) -> Result<bool, anyhow::Error> {
        self.clone_ext(url, CloneOptions::default())
    }

    /// Whether the repository has currently checked out `git_ref`.
    pub fn is_ref(&self, git_ref: &Ref) -> bool {
        match git_ref {
            Ref::Branch(b) => self.describe().ok().map(|s| s == format!("heads/{}", b)),
            Ref::Tag(t) => self.describe().ok().map(|s| s == format!("tags/{}", t)),
            Ref::Commit(c) => {
                cmd_output!(GIT, @self.git_args(), "rev-parse", "HEAD"; envs=(LC_ALL))
                    .ok()
                    .map(|s| s == *c)
            }
        }
        .unwrap_or(false)
    }

    pub fn is_shallow(&self) -> bool {
        self.git_dir.join("shallow").exists()
    }

    /// Clone the repository with `options` and return if the repository was modified.
    pub fn clone_ext(&mut self, url: &str, options: CloneOptions) -> Result<bool, anyhow::Error> {
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

            if !self.is_ref(&force_ref) {
                (true, true, true)
            } else {
                match force_ref {
                    Ref::Branch(_) if !options.force_clean || self.is_clean()? => {
                        let modified = if let Some(reset_mode) = options.branch_update_action {
                            cmd!(GIT, @self.git_args(), "reset", reset_mode.to_string())?;
                            cmd!(GIT, @self.git_args(), "pull", "--ff-only")?;
                            true
                        } else {
                            false
                        };

                        (false, false, modified)
                    }
                    Ref::Commit(_) | Ref::Tag(_) if !options.force_clean || self.is_clean()? => {
                        (false, false, false)
                    }
                    _ => (true, true, true),
                }
            }
        } else {
            (true, true, true)
        };

        if should_remove {
            remove_dir_all::remove_dir_all(&self.worktree)?;
        }

        if should_clone {
            let depth = options.depth.map(|i| i.to_string());
            let (depth, branch) = match &options.force_ref {
                None | Some(Ref::Commit(_)) => (None, None),
                Some(Ref::Branch(s) | Ref::Tag(s)) => (
                    depth
                        .as_deref()
                        .map(|i| ["--depth", i, "--shallow-submodules"]),
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
    pub fn apply(
        &self,
        patches: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> Result<(), CmdError> {
        cmd!(GIT, @self.git_args(), "apply"; args=(patches.into_iter()), current_dir=(&self.worktree))?;
        Ok(())
    }

    /// Apply all patches to this repository only if they were not applied already.
    ///
    /// Uses [`is_applied`](Self::is_applied) to determine if the patches were already applied.
    pub fn apply_once(
        &self,
        patches: impl Iterator<Item = impl AsRef<OsStr>> + Clone,
    ) -> Result<(), CmdError> {
        if !self.is_applied(patches.clone())? {
            self.apply(patches)?;
        }
        Ok(())
    }

    /// Whether all `patches` are already applied to this repository.
    ///
    /// This runs `git apply --check --reverse <patches..>` which if it succeeds means
    /// that git could reverse all `patches` successfully and implies that all patches
    /// were already applied.
    pub fn is_applied(
        &self,
        patches: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> Result<bool, CmdError> {
        Ok(cmd!(
            GIT, @self.git_args(), "apply", "--check", "-R";
            status,
            args=(patches.into_iter()),
            current_dir=(&self.worktree)
        )?
        .success())
    }
}

/// The mode passed to `git reset HEAD --<mode>`.
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

#[derive(Debug, Default)]
#[must_use]
pub struct CloneOptions {
    /// Force the working directory to be this specific tag, branch or commit.
    ///
    /// On a missmatch between this value and the state of the physical repository, it is
    /// deleted and cloned from scratch.
    ///
    /// If this option specifies a branch name which maches the current branch of the
    /// physical repository and [`branch_update_action`](Self::branch_update_action) is
    /// not [`None`] then [`Repository::clone_ext`] will try to update the repository with
    /// the following commands:
    /// - `git reset HEAD <reset mode>` (where `reset mode` is the value of
    ///   [`branch_update_action`](Self::branch_update_action))
    /// - `git pull --ff-only`
    /// If these operations fail an error is returned from [`Repository::clone_ext`].
    pub force_ref: Option<Ref>,
    /// The mode that is passed to `git reset` when the branch is updated.
    /// If `None` the working directory with branch is never updated.
    pub branch_update_action: Option<ResetMode>,
    /// If the working directory is not clean and `force_clean` is `true`, the git repo
    /// will be cloned from scratch.
    pub force_clean: bool,
    /// The depth that should be cloned, if `None` the full repository is cloned.
    ///
    /// Note that this option is ignored when [`force_ref`](Self::force_ref) specifies a
    /// commit.
    pub depth: Option<NonZeroU64>,
}

impl CloneOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Force the working directory to be this specific tag, branch or commit.
    ///
    /// On a missmatch between this value and the state of the physical repository, it is
    /// deleted and cloned from scratch.
    ///
    /// If this option specifies a branch name which maches the current branch of the
    /// physical repository and [`branch_update_action`](Self::branch_update_action) is
    /// not [`None`] then [`Repository::clone_ext`] will try to update the repository with
    /// the following commands:
    /// - `git reset HEAD <reset mode>` (where `reset mode` is the value of
    ///   [`branch_update_action`](Self::branch_update_action))
    /// - `git pull --ff-only`
    /// If these operations fail an error is returned from [`Repository::clone_ext`].
    pub fn force_ref(mut self, force_ref: Ref) -> Self {
        self.force_ref = Some(force_ref);
        self
    }

    /// The mode that is passed to `git reset` when the branch is updated.
    /// If `None` the working directory with branch is never updated.
    ///
    /// See [`force_ref`](Self::force_ref) for more info.
    pub fn branch_update_action(mut self, reset_mode: ResetMode) -> Self {
        self.branch_update_action = Some(reset_mode);
        self
    }

    /// If the working directory is not clean and `force_clean` is `true`, the git repo
    /// will be cloned from scratch.
    pub fn force_clean(mut self) -> Self {
        self.force_clean = true;
        self
    }

    /// The depth that should be cloned, if `None` the full repository is cloned.
    ///
    /// `depth` must be greater than zero or else this method will panic.
    ///
    /// Note that this option is ignored when [`force_ref`](Self::force_ref) specifies a
    /// commit.
    pub fn depth(mut self, depth: u64) -> Self {
        self.depth = Some(NonZeroU64::new(depth).expect("depth must be greater than zero"));
        self
    }
}
