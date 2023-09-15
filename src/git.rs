//! Git repository manipulation through the git CLI.
// TODO: maybe use `git2` crate

use std::ffi::OsStr;
use std::fmt::Display;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};

use crate::cmd;
use crate::cmd::CmdError;
use crate::utils::PathExt;

/// The git command.
pub const GIT: &str = "git";

/// A list of environment variables to set/unset so that git is guaranteed to output
/// english.
///
/// Note: `LANGUAGE` must be unset, otherwise it will override `LC_ALL` if it is set to
/// anything other than `C` (we use `C.UTF-8`).
const LC_ALL: [(&str, &str); 2] = [("LC_ALL", "C.UTF-8"), ("LANGUAGE", "")];

/// A logical git repository which may or may not exist.
#[derive(Debug, Clone)]
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
            cmd!(GIT, "rev-parse", "--show-toplevel"; current_dir=(dir), envs=(LC_ALL))
                .stdout()
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
            &cmd!(GIT, "rev-parse", "--git-dir"; current_dir=(dir), envs=(LC_ALL)).stdout()?,
        )
        .abspath_relative_to(dir);

        Ok(Repository {
            git_dir,
            worktree: dir.to_owned(),
            remote_name: None,
        })
    }

    /// Get the path to the worktree of this git repository.
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
        Ok(cmd!(GIT, @self.git_args(), "remote", "show"; envs=(LC_ALL))
            .stdout()?
            .lines()
            .filter_map(|l| {
                let remote = l.trim().to_owned();
                cmd!(GIT, @self.git_args(), "remote", "get-url", &remote; envs=(LC_ALL))
                    .stdout()
                    .ok()
                    .map(|url| (remote, url))
            })
            .collect())
    }

    /// Get the default branch name of `remote`.
    pub fn get_default_branch_of(&self, remote: &str) -> Result<String, anyhow::Error> {
        let output =
            cmd!(GIT, @self.git_args(), "remote", "show", remote; envs=(LC_ALL)).stdout()?;
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
            cmd!(GIT, @self.git_args(), "status", "-s", "-uno", "--ignore-submodules=untracked", "--ignored=no"; envs=(LC_ALL))
                .stdout()?
                .trim()
                .is_empty()
        )
    }

    /// Get the exact ref from all `refs/` directly referencing the current commit.
    ///
    /// E.g.
    /// - branch `<branch>`: `heads/<branch>`
    /// - tag `<tag>`: `tags/<tag>`
    ///
    /// Calls `git describe --all --exact-match`.
    pub fn describe_exact_ref(&self) -> Result<String, CmdError> {
        cmd!(GIT, @self.git_args(), "describe", "--all", "--exact-match"; envs=(LC_ALL)).stdout()
    }

    /// Get a [`Ref`] for the current commit.
    ///
    /// Calls `git describe --all --exact-match --always --abbrev=40`
    pub fn get_ref(&self) -> Result<Ref, CmdError> {
        let mut cmd = cmd!(GIT, @self.git_args(), "describe", "--all", "--exact-match", "--always", "--abbrev=40"; envs=(LC_ALL));
        let ref_or_commit = cmd.stdout()?;
        if let Some(branch) = ref_or_commit.strip_prefix("heads/") {
            Ok(Ref::Branch(branch.to_owned()))
        } else if let Some(tag) = ref_or_commit.strip_prefix("tags/") {
            Ok(Ref::Tag(tag.to_owned()))
        } else if ref_or_commit.contains('/') {
            Err(CmdError::Unsuccessful(
                format!("{:?}", cmd.cmd),
                -1,
                Some(anyhow!(
                    "could not parse ref '{}': not a branch, tag or commit",
                    ref_or_commit
                )),
            ))
        } else {
            Ok(Ref::Commit(ref_or_commit))
        }
    }

    /// Get the current branch name if the current checkout is the top of the branch.
    pub fn get_branch_name(&self) -> Result<Option<String>, CmdError> {
        Ok(self
            .describe_exact_ref()?
            .strip_prefix("heads/")
            .map(Into::into))
    }

    /// Clone the repository with the default options and return if the repository was modified.
    pub fn clone(&mut self, url: &str) -> Result<bool, anyhow::Error> {
        self.clone_ext(url, CloneOptions::default())
    }

    /// Whether the repository has currently checked out `git_ref`.
    pub fn is_ref(&self, git_ref: &Ref) -> bool {
        match git_ref {
            Ref::Branch(b) => self
                .describe_exact_ref()
                .ok()
                .map(|s| s == format!("heads/{b}")),
            Ref::Tag(t) => self
                .describe_exact_ref()
                .ok()
                .map(|s| s == format!("tags/{t}")),
            Ref::Commit(c) => cmd!(GIT, @self.git_args(), "rev-parse", "HEAD"; envs=(LC_ALL))
                .stdout()
                .ok()
                .map(|s| s == *c),
        }
        .unwrap_or(false)
    }

    /// Whether this repo is a shallow clone.
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
                            cmd!(GIT, @self.git_args(), "reset", reset_mode.to_string()).run()?;
                            cmd!(GIT, @self.git_args(), "pull", "--ff-only").run()?;
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

            cmd!(GIT, "clone", "--recursive", @depth, @branch, &url, &self.worktree).run()?;

            if let Some(Ref::Commit(s)) = options.force_ref {
                cmd!(GIT, @self.git_args(), "checkout", s).run()?;
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
        cmd!(GIT, @self.git_args(), "apply"; args=(patches), current_dir=(&self.worktree)).run()?;
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
            args=(patches),
            current_dir=(&self.worktree)
        )
        .status()?
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

/// A reference to a git tag, branch or commit.
#[derive(Debug, Clone)]
pub enum Ref {
    Tag(String),
    Branch(String),
    Commit(String),
}

impl Ref {
    /// Parse a [`git::Ref`] from a ref string.
    ///
    /// The ref string can have the following format:
    /// - `commit:<hash>`: Uses the commit `<hash>` of the repository. Note that
    ///                    this will clone the whole repository not just one commit.
    /// - `tag:<tag>`: Uses the tag `<tag>` of the repository.
    /// - `branch:<branch>`: Uses the branch `<branch>` of the repository.
    /// - `v<major>.<minor>` or `<major>.<minor>`: Uses the tag `v<major>.<minor>` of the repository.
    /// - `<branch>`: Uses the branch `<branch>` of the repository.
    pub fn parse(ref_str: impl AsRef<str>) -> Self {
        let ref_str = ref_str.as_ref().trim();
        assert!(
            !ref_str.is_empty(),
            "Ref str ('{ref_str}') must be non-empty"
        );

        match ref_str.split_once(':') {
            Some(("commit", c)) => Self::Commit(c.to_owned()),
            Some(("tag", t)) => Self::Tag(t.to_owned()),
            Some(("branch", b)) => Self::Branch(b.to_owned()),
            _ => match ref_str.chars().next() {
                Some(c) if c.is_ascii_digit() => Self::Tag("v".to_owned() + ref_str),
                Some('v')
                    if ref_str.len() > 1 && ref_str.chars().nth(1).unwrap().is_ascii_digit() =>
                {
                    Self::Tag(ref_str.to_owned())
                }
                Some(_) => Self::Branch(ref_str.to_owned()),
                _ => unreachable!(),
            },
        }
    }
}

impl Display for Ref {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tag(s) => write!(f, "Tag {s}"),
            Self::Branch(s) => write!(f, "Branch {s}"),
            Self::Commit(s) => write!(f, "Commit {s}"),
        }
    }
}

/// Options for how a repository should be cloned by [`Repository::clone_ext`].
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

pub mod sdk {
    use std::collections::hash_map::DefaultHasher;
    use std::fs;
    use std::hash::{Hash, Hasher};
    use std::path::Path;

    use anyhow::{anyhow, Context, Result};

    use crate::git;

    /// A distinct version of the SDK repository to be installed.
    #[derive(Debug, Clone)]
    pub struct RemoteSdk {
        /// Optional custom URL to the git repository.
        pub repo_url: Option<String>,
        /// A [`git::Ref`] for the commit, tag or branch to be used.
        pub git_ref: git::Ref,
    }

    impl RemoteSdk {
        /// Clone the repository or open if it exists and matches [`RemoteSdk::git_ref`].
        pub fn open_or_clone(
            &self,
            install_dir: &Path,
            options: git::CloneOptions,
            default_repo: &str,
            managed_repo_dir_base: &str,
        ) -> Result<git::Repository> {
            // Only append a hash of the git remote URL to the parent folder name of the
            // repository if this is not the default remote.
            let folder_name = if let Some(hash) = self.url_hash() {
                format!("{managed_repo_dir_base}-{hash}")
            } else {
                managed_repo_dir_base.to_owned()
            };
            let repos_dir = install_dir.join(folder_name);
            if !repos_dir.exists() {
                fs::create_dir(&repos_dir).with_context(|| {
                    anyhow!("could not create folder '{}'", repos_dir.display())
                })?;
            }

            let repo_path = repos_dir.join(self.repo_dir());
            let mut repository = git::Repository::new(repo_path);

            repository.clone_ext(
                self.repo_url(default_repo),
                options.force_ref(self.git_ref.clone()),
            )?;

            Ok(repository)
        }

        /// Return the URL of the GIT repository.
        /// If `repo_url` is [`None`], then the default SDK repository is returned.
        fn repo_url<'a>(&'a self, default_repo: &'a str) -> &'a str {
            self.repo_url.as_deref().unwrap_or(default_repo)
        }

        /// Create a hash when a custom repo_url is specified.
        fn url_hash(&self) -> Option<String> {
            // This uses the default hasher from the standard library, which is not guaranteed
            // to be the same across versions, but if the hash algorithm changes and assuming
            // a different hash, the logic above will happily clone the repo in a different
            // directory. It also uses a 64 bit hash by which the chance for collisions is
            // pretty small (assuming a good hash function) and even if there is a collision
            // it will still work (and also even if the ref is the same), though the cloned
            // repo will be in the same folder as a repo from another remote URL.
            // Cargo actually does something similar for the out-dirs though it uses the
            // deprecated `std::hash::SipHasher` instead.
            let mut hasher = DefaultHasher::new();
            self.repo_url.as_ref()?.hash(&mut hasher);
            Some(format!("{:x}", hasher.finish()))
        }

        /// Translate the ref name to a directory name.
        ///
        /// This heaviliy sanitizes that name as it translates an arbitrary git tag, branch or
        /// commit to a folder name, as such we allow only alphanumeric ASCII characters and
        /// most punctuation.
        fn repo_dir(&self) -> String {
            // Most of the time this returns either a tag in the form of `v<version>` or a
            // branch name like `release/v<version>`, implementing special logic to prevent
            // the very rare case that a tag and branch with the same name exists is not worth
            // it and can also be worked around without this logic.
            let ref_name = match &self.git_ref {
                git::Ref::Branch(n) | git::Ref::Tag(n) | git::Ref::Commit(n) => n,
            };
            // Replace all directory separators with a dash `-`, so that we don't create
            // subfolders for tag or branch names that contain such characters.
            let mut ref_name = ref_name.replace(['/', '\\'], "-");

            // Sanitize:
            // Remove all chars that are not ASCII alphanumeric or almost all
            // punctuation, except the ones forbidden in paths (more information here
            // https://stackoverflow.com/questions/1976007/what-characters-are-forbidden-in-windows-and-linux-directory-names).
            ref_name.retain(|c| {
                c.is_ascii_alphanumeric()
                    || b"!#$%&'()+,-.;=@[]^_`{}~"
                        .iter()
                        .any(|delim| c == *delim as char)
            });
            ref_name
        }
    }
}
