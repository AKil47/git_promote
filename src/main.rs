use anyhow::{bail, Context, Result};
use clap::Parser;
use git2::{build::CheckoutBuilder, Repository, StatusOptions};
use std::env;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Automatically commit uncommitted changes with message "wip"
    #[arg(long)]
    wip: bool,

    /// Overwrite uncommitted changes in the main worktree instead of failing
    #[arg(long)]
    force: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let current_dir = env::current_dir().context("Failed to get current directory")?;
    let repo = Repository::open(&current_dir).context("Failed to open repository")?;

    validate_current_worktree(&repo, args.wip)?;

    let main_worktree_path = find_main_worktree(&repo)?;
    println!("Found main worktree at: {:?}", main_worktree_path);

    let main_repo = Repository::open(&main_worktree_path).context("Failed to open main repository")?;
    validate_main_repo(&main_repo, args.force)?;

    let head = repo.head().context("Failed to get HEAD")?;
    let branch_name = if head.is_branch() {
        head.shorthand().map(|s| s.to_string())
    } else {
        None
    };

    let head_commit = head.peel_to_commit().context("Failed to resolve HEAD to commit")?;
    let commit_id = head_commit.id();

    promote_to_main(&main_repo, commit_id, branch_name.as_deref(), args.force)?;

    println!("Done.");
    Ok(())
}

fn validate_current_worktree(repo: &Repository, wip: bool) -> Result<()> {
    if !repo.is_worktree() {
        bail!("Current repository is not a worktree. 'git promote' must be run from a worktree.");
    }

    let dirty_count = count_dirty_items(repo, "Current worktree")?;

    if dirty_count > 0 {
        if wip {
            commit_wip(repo)?;
        } else {
            bail!("Current worktree has {} unstaged/uncommitted changes. Use --wip to auto-commit them as 'wip', or clean up before promoting.", dirty_count);
        }
    }
    
    Ok(())
}

fn commit_wip(repo: &Repository) -> Result<()> {
    // Add all changes to index
    let mut index = repo.index().context("Failed to get index")?;
    
    // items to add are determined by status (modified, deleted, etc.)
    // simpler to just add all tracked files that are modified/deleted + untracked?
    // The requirement says "uncommitted changes in the worktree".
    // "git commit -am" usually handles modified and deleted tracked files.
    // Let's emulate `git add -u` (update tracked files) or `git add .`?
    // User said: equivalent of `git commit -am wip`. `git commit -a` automatically stages files that have been modified and deleted, but new files you have not told Git about are not affected.
    
    index.update_all(vec!["*"].iter(), None).context("Failed to update index")?;
    index.write().context("Failed to write index")?;

    let oid = index.write_tree().context("Failed to write tree")?;
    let tree = repo.find_tree(oid).context("Failed to find tree")?;

    let signature = repo.signature().context("Failed to get signature")?;
    let parent_commit = repo.head().context("Failed to get HEAD")?.peel_to_commit().context("Failed to resolve HEAD to commit")?;

    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        "wip",
        &tree,
        &[&parent_commit],
    ).context("Failed to create wip commit")?;
    
    println!("Created WIP commit.");

    Ok(())
}

fn validate_main_repo(repo: &Repository, force: bool) -> Result<()> {
    // Check if main repo is bare, just in case
    if repo.is_bare() {
        bail!("Main repository is bare. Cannot checkout.");
    }
    
    let dirty_count = count_dirty_items(repo, "Main worktree")?;
    if dirty_count > 0 {
        if force {
            println!("Main worktree has {} unstaged/uncommitted changes, but --force was used. Overwriting.", dirty_count);
        } else {
            bail!("Main worktree has {} unstaged/uncommitted changes. Please clean up before promoting.", dirty_count);
        }
    }
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
fn promote_to_main(main_repo: &Repository, commit_id: git2::Oid, branch_name: Option<&str>, force: bool) -> Result<()> {
    // We need to look up the commit/tree IN the main repo to ensure it belongs to that Repository instance.
    let main_target_commit = main_repo.find_commit(commit_id).context("Failed to find target commit in main repo")?;
    let main_target_tree = main_target_commit.tree().context("Failed to get tree of target commit in main repo")?;
    
    let mut checkout_builder = CheckoutBuilder::new();
    if force {
        checkout_builder.force();
    }
    // Default is Safe.
    main_repo.checkout_tree(main_target_tree.as_object(), Some(&mut checkout_builder))
        .context("Failed to checkout target tree in main repo")?;
    
    let detached_from_branch = branch_name
        .and_then(|name| main_repo.find_branch(name, git2::BranchType::Local).ok())
        .and_then(|branch| {
            let reference = branch.into_reference();
            main_repo.reference_to_annotated_commit(&reference).ok()
        })
        .and_then(|annotated| main_repo.set_head_detached_from_annotated(annotated).ok())
        .is_some();
    
    if !detached_from_branch {
        main_repo.set_head_detached(commit_id).context("Failed to set detached HEAD in main repo")?;
    }

    Ok(())
}

fn count_dirty_items(repo: &Repository, name: &str) -> Result<usize> {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true);
    let statuses = repo.statuses(Some(&mut opts)).context(format!("Failed to get statuses for {}", name))?;
    
    // Filter out ignored files just in case
    let dirty_count = statuses.iter().filter(|s| !s.status().is_ignored()).count();

    Ok(dirty_count)
}

fn find_main_worktree(repo: &Repository) -> Result<PathBuf> {
    // repo.path() in worktree -> .../.git/worktrees/name/
    // repo.commondir() -> .../.git/
    
    let commondir = repo.commondir();
    // The main worktree root is the parent of the common .git directory
    let main_root = commondir.parent().context("Failed to determine main worktree root from common dir")?;
    
    Ok(main_root.to_path_buf())
}
