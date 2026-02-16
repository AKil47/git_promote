use anyhow::{bail, Context, Result};
use git2::{build::CheckoutBuilder, Repository, StatusOptions};
use std::env;
use std::path::PathBuf;

fn main() -> Result<()> {
    let current_dir = env::current_dir().context("Failed to get current directory")?;
    let repo = Repository::open(&current_dir).context("Failed to open repository")?;

    validate_current_worktree(&repo)?;

    let main_worktree_path = find_main_worktree(&repo)?;
    println!("Found main worktree at: {:?}", main_worktree_path);

    let main_repo = Repository::open(&main_worktree_path).context("Failed to open main repository")?;
    validate_main_repo(&main_repo)?;

    let head_commit = repo.head().context("Failed to get HEAD")?.peel_to_commit().context("Failed to resolve HEAD to commit")?;
    let commit_id = head_commit.id();

    promote_to_main(&main_repo, commit_id)?;

    println!("Done.");
    Ok(())
}

fn validate_current_worktree(repo: &Repository) -> Result<()> {
    if !repo.is_worktree() {
        bail!("Current repository is not a worktree. 'git promote' must be run from a worktree.");
    }
    check_clean_status(repo, "Current worktree")?;
    Ok(())
}

fn validate_main_repo(repo: &Repository) -> Result<()> {
    // Check if main repo is bare, just in case
    if repo.is_bare() {
        bail!("Main repository is bare. Cannot checkout.");
    }
    check_clean_status(repo, "Main worktree")?;
    Ok(())
}

/// Promotes a commit to the main worktree.
///
/// This function performs a safe checkout of the target commit in the main repository
/// and then updates the HEAD to point to that commit in a detached state.
///
/// flow:
/// 1. Resolve the target commit and tree in the main repository context.
/// 2. Perform a safe `checkout_tree` to update the working directory.
///    - This ensures that if there are uncommitted changes in the main worktree that would be overwritten, the operation fails safely.
/// 3. Update HEAD to the target commit (detached).
fn promote_to_main(main_repo: &Repository, commit_id: git2::Oid) -> Result<()> {
    // We need to look up the commit/tree IN the main repo to ensure it belongs to that Repository instance.
    let main_target_commit = main_repo.find_commit(commit_id).context("Failed to find target commit in main repo")?;
    let main_target_tree = main_target_commit.tree().context("Failed to get tree of target commit in main repo")?;
    
    let mut checkout_builder = CheckoutBuilder::new();
    // Default is Safe.
    main_repo.checkout_tree(main_target_tree.as_object(), Some(&mut checkout_builder))
        .context("Failed to checkout target tree in main repo")?;
    
    main_repo.set_head_detached(commit_id).context("Failed to set detached HEAD in main repo")?;

    Ok(())
}

fn check_clean_status(repo: &Repository, name: &str) -> Result<()> {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true);
    let statuses = repo.statuses(Some(&mut opts)).context(format!("Failed to get statuses for {}", name))?;
    
    // Filter out ignored files just in case
    let dirty_count = statuses.iter().filter(|s| !s.status().is_ignored()).count();

    if dirty_count > 0 {
        bail!("{} has {} unstaged/uncommitted changes. Please clean up before promoting.", name, dirty_count);
    }
    Ok(())
}

fn find_main_worktree(repo: &Repository) -> Result<PathBuf> {
    // repo.path() in worktree -> .../.git/worktrees/name/
    // repo.commondir() -> .../.git/
    
    let commondir = repo.commondir();
    // The main worktree root is the parent of the common .git directory
    let main_root = commondir.parent().context("Failed to determine main worktree root from common dir")?;
    
    Ok(main_root.to_path_buf())
}

