use git2::{Repository, Signature};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn setup_repo_and_worktree(test_name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let test_root = env::current_dir().unwrap().join("target").join("tmp_test").join(test_name);
    if test_root.exists() {
        fs::remove_dir_all(&test_root).unwrap();
    }
    fs::create_dir_all(&test_root).unwrap();

    let main_repo_path = test_root.join("main_repo");
    let worktree_path = test_root.join("wt");

    // 1. Setup main repo
    let repo = Repository::init(&main_repo_path).unwrap();
    
    // config user
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "Your Name").unwrap();
    config.set_str("user.email", "you@example.com").unwrap();
    config.set_bool("commit.gpgsign", false).unwrap();

    // Initial commit
    let signature = Signature::now("Your Name", "you@example.com").unwrap();
    fs::write(main_repo_path.join("file.txt"), "initial").unwrap();
    
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("file.txt")).unwrap();
    let oid = index.write_tree().unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(oid).unwrap();
    repo.commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[]).unwrap();

    // 2. Create worktree
    // `repo.worktree` will create the branch if it doesn't exist.
    let _repo_wt = repo.worktree("feature", &worktree_path, None).unwrap();

    (test_root, main_repo_path, worktree_path)
}

fn run_git_promote(cwd: &Path, args: &[&str]) -> std::process::ExitStatus {
    let binary = env::current_dir().unwrap().join("target/debug/git_promote.exe");
    Command::new(&binary).args(args).current_dir(cwd).status().unwrap()
}

#[test]
fn test_git_promote_success() {
    let (_, main_repo_path, worktree_path) = setup_repo_and_worktree("success");

    // 3. Make changes in worktree
    let wt_repo = Repository::open(&worktree_path).unwrap();
    let signature = Signature::now("Your Name", "you@example.com").unwrap();
    
    fs::write(worktree_path.join("file.txt"), "feature change").unwrap();
    
    let mut wt_index = wt_repo.index().unwrap();
    wt_index.add_path(Path::new("file.txt")).unwrap();
    let wt_oid = wt_index.write_tree().unwrap();
    wt_index.write().unwrap();
    let wt_tree = wt_repo.find_tree(wt_oid).unwrap();
    let parent = wt_repo.head().unwrap().peel_to_commit().unwrap();
    wt_repo.commit(Some("HEAD"), &signature, &signature, "feature change", &wt_tree, &[&parent]).unwrap();

    let commit_hash = wt_repo.head().unwrap().peel_to_commit().unwrap().id();

    // 4. Run git_promote
    let status = run_git_promote(&worktree_path, &[]);
    assert!(status.success(), "git_promote failed");

    // 5. Verify main repo
    let main_repo_check = Repository::open(&main_repo_path).unwrap();
    let main_head_id = main_repo_check.head().unwrap().peel_to_commit().unwrap().id();

    assert_eq!(main_head_id, commit_hash, "Main repo HEAD should match worktree commit");
    assert!(main_repo_check.head_detached().unwrap(), "Main repo should be detached");
    
    let content = fs::read_to_string(main_repo_path.join("file.txt")).unwrap();
    assert_eq!(content, "feature change");
}

#[test]
fn test_not_in_worktree() {
    let (_, main_repo_path, _) = setup_repo_and_worktree("not_in_worktree");
    
    // Run in main repo (which is not a worktree of itself, usually? well git2 is_worktree checks if it is a linked worktree)
    // A standard repo is NOT a worktree in git2 terminology (is_worktree() returns false).
    
    let status = run_git_promote(&main_repo_path, &[]);
    assert!(!status.success(), "Should fail when run in main repo");
}

#[test]
fn test_dirty_worktree() {
    let (_, _, worktree_path) = setup_repo_and_worktree("dirty_worktree");
    
    // Modify file but don't commit
    fs::write(worktree_path.join("file.txt"), "dirty").unwrap();
    
    let status = run_git_promote(&worktree_path, &[]);
    assert!(!status.success(), "Should fail with dirty worktree");
}

#[test]
fn test_dirty_main() {
    let (_, main_repo_path, worktree_path) = setup_repo_and_worktree("dirty_main");
    
    // Modify file in MAIN repo
    fs::write(main_repo_path.join("file.txt"), "dirty main").unwrap();
    
    // Attempt promote from worktree
    let status = run_git_promote(&worktree_path, &[]);
    assert!(!status.success(), "Should fail when main worktree is dirty");
}

#[test]
fn test_dirty_worktree_with_wip() {
    let (_, main_repo_path, worktree_path) = setup_repo_and_worktree("dirty_worktree_wip");
    
    // Modify file but don't commit
    fs::write(worktree_path.join("file.txt"), "dirty").unwrap();
    
    // Run with --wip
    let status = run_git_promote(&worktree_path, &["--wip"]);
    assert!(status.success(), "Should success with --wip");

    // Verify wip commit created
    let wt_repo = Repository::open(&worktree_path).unwrap();
    let head = wt_repo.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(head.message().unwrap(), "wip");

    // Verify promoted
     let main_repo_check = Repository::open(&main_repo_path).unwrap();
    let main_head_id = main_repo_check.head().unwrap().peel_to_commit().unwrap().id();

    assert_eq!(main_head_id, head.id(), "Main repo HEAD should match worktree commit");
    let content = fs::read_to_string(main_repo_path.join("file.txt")).unwrap();
    assert_eq!(content, "dirty");
}

#[test]
fn test_clean_worktree_with_wip() {
     let (_, _, worktree_path) = setup_repo_and_worktree("clean_worktree_wip");

     // Run with --wip on clean worktree
     let status = run_git_promote(&worktree_path, &["--wip"]);
     assert!(status.success(), "Should success with --wip on clean worktree");

     // Verify no new commit
     let wt_repo = Repository::open(&worktree_path).unwrap();
     let head = wt_repo.head().unwrap().peel_to_commit().unwrap();
    assert_ne!(head.message().unwrap(), "wip"); // Should be initial commit or whatever was before
}

#[test]
fn test_dirty_main_with_force() {
    let (_, main_repo_path, worktree_path) = setup_repo_and_worktree("dirty_main_force");
    
    // Modify file in MAIN repo
    fs::write(main_repo_path.join("file.txt"), "dirty main").unwrap();
    
    // Attempt promote from worktree
    let status = run_git_promote(&worktree_path, &["--force"]);
    assert!(status.success(), "Should succeed when main worktree is dirty but --force is used");
    
    // Check that file.txt is restored
    let content = fs::read_to_string(main_repo_path.join("file.txt")).unwrap();
    // The main worktree is checking out the commit from worktree which is "initial" since we didn't modify it in worktree in this test.
    assert_eq!(content, "initial");
}

#[test]
fn test_promote_detached_branch_name() {
    let (_, main_repo_path, worktree_path) = setup_repo_and_worktree("detached_branch");

    let status = run_git_promote(&worktree_path, &[]);
    assert!(status.success(), "git_promote failed");

    // verify git status indicates detached at branch
    let output = Command::new("git")
        .arg("status")
        .current_dir(&main_repo_path)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("HEAD detached at feature") || stdout.contains("HEAD detached at refs/heads/feature"),
        "Git status did not show detached at branch: {}", stdout
    );
}

#[test]
fn test_promote_detached_commit() {
    let (_, main_repo_path, worktree_path) = setup_repo_and_worktree("detached_commit");

    // detach worktree so `git promote` sees no branch name
    let wt_repo = Repository::open(&worktree_path).unwrap();
    let commit_hash = wt_repo.head().unwrap().peel_to_commit().unwrap().id();
    wt_repo.set_head_detached(commit_hash).unwrap();

    let status = run_git_promote(&worktree_path, &[]);
    assert!(status.success(), "git_promote failed");

    let output = Command::new("git")
        .arg("status")
        .current_dir(&main_repo_path)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_hash = &commit_hash.to_string()[..7];
    assert!(
        stdout.contains(&format!("HEAD detached at {}", short_hash)),
        "Git status did not show detached at commit: {}", stdout
    );
}
