use super::*;
use crate::util::testing::{TempDir, put};

#[cfg(windows)]
#[test]
fn project_falls_back_instead_of_following_a_junction_in_cwd_or_git_metadata() {
    let t = TempDir::new("project-junction");
    let real = t.0.join("real");
    put(&real.join(".git").join("HEAD"), "ref: refs/heads/main\n");
    std::fs::create_dir(real.join("src")).unwrap();
    assert_eq!(project(&real).branch.as_deref(), Some("main"));
    let alias = t.0.join("alias");
    crate::share::junction::create(&alias, &real).unwrap();
    let child = alias.join("src");
    assert_eq!(project(&child), Project::folder(&child));
    let linked_git = t.0.join("linked");
    std::fs::create_dir(&linked_git).unwrap();
    crate::share::junction::create(&linked_git.join(".git"), &real.join(".git")).unwrap();
    assert_eq!(project(&linked_git), Project::folder(&linked_git));
    std::fs::remove_dir(linked_git.join(".git")).unwrap();
    std::fs::remove_dir(alias).unwrap();
}

#[test]
fn projects_resolve_subfolders_worktrees_and_submodules_without_a_git_program() {
    let t = TempDir::new("project-roots");
    let main = t.0.join("main repo");
    let worktree = t.0.join("sidebar");
    let submodule = main.join("vendor").join("lib");
    let gitdir = main.join(".git").join("worktrees").join("sidebar");
    put(&gitdir.join("commondir"), "../..\n");
    put(&main.join(".git").join("HEAD"), "ref: refs/heads/main\n");
    put(&worktree.join(".git"), "gitdir: ../main repo/.git/worktrees/sidebar\n");
    let sub_git = main.join(".git").join("modules").join("lib");
    put(&sub_git.join("HEAD"), "ref: refs/heads/lib\n");
    put(&submodule.join(".git"), &format!("gitdir: {}\n", sub_git.display()));
    let inside = main.join("src");
    let work_inside = worktree.join("ui");
    for p in [&inside, &work_inside] {
        std::fs::create_dir_all(p).unwrap();
    }
    let base = project(&main);
    assert_eq!(base.name, "main repo");
    assert_eq!(base.branch.as_deref(), Some("main"));
    assert_eq!(project(&inside).key, base.key);
    assert_eq!(project(&inside).location.as_deref(), Some("src"));
    assert_eq!(project(&work_inside).key, base.key);
    assert_eq!(project(&work_inside).location.as_deref(), Some("Worktree: sidebar / ui"));
    assert_eq!(project(&submodule).name, "lib");
    assert_ne!(project(&submodule).key, base.key);
    let missing = main.join("gone");
    assert_eq!(project(&missing), Project::folder(&missing));
    let folders = [work_inside.as_path()];
    let key = folder_key(&work_inside);
    assert_eq!(projects_for_scan(folders)[&key].key, base.key);
    put(&gitdir.join("commondir"), "../missing");
    assert_eq!(projects_for_scan(folders)[&key], Project::folder(&work_inside));
}

#[test]
fn project_refresh_rechecks_a_normal_comp_folder_after_git_init() {
    let t = TempDir::new("project-refresh");
    let comp = t.0.join("comp");
    std::fs::create_dir(&comp).unwrap();
    let folders = [comp.as_path()];
    let key = folder_key(&comp);
    assert_eq!(projects_for_scan(folders)[&key], Project::folder(&comp));
    put(&comp.join(".git/HEAD"), "ref: refs/heads/main\n");
    assert_eq!(projects_for_scan(folders)[&key].branch.as_deref(), Some("main"));
    put(&comp.join(".git/HEAD"), "ref: refs/heads/next\n");
    assert_eq!(projects_for_scan(folders)[&key].branch.as_deref(), Some("next"));
    assert!(projects_for_scan([]).is_empty());
}

#[test]
fn chains_branch_missing_parents_and_cycles_are_not_confused() {
    assert_eq!(thread_roots(&[Some(1), Some(2), None, Some(1), Some(20), Some(5)]), [2, 2, 2, 2, 4, 5]);
    assert_eq!(thread_roots(&[Some(2), Some(2), Some(1), Some(1)]), [2, 1, 2, 1]);
    let long: Vec<_> = (1..=10_000).map(Some).collect();
    assert!(thread_roots(&long).iter().all(|&root| root == 9999));
}

#[test]
fn sittings_join_overlap_and_exact_grace_but_never_cross_the_local_day() {
    let hour = 3_600_000;
    let end = 86_400_000 - hour;
    let a = |project, first, last| Activity { project, first: Some(first), last };
    let samples = [
        a("main", end - 3 * hour, end - hour),
        a("main", end - hour / 2, end - hour / 4),
        a("main", end - 2 * hour, end - hour / 2),
        a("work", end - 2 * hour, end - hour / 2),
        a("main", end + 1, end + 10),
        a("main", end + hour / 2 + 11, end + hour),
        a("main", 0, 0),
    ];
    assert_eq!(sittings(&samples, 60), [0, 0, 0, 3, 4, 5, 6]);
    assert_eq!(sittings(&samples, 0)[4], 0);
}

#[test]
fn similarity_is_case_insensitive_overlap_without_repetitions_or_common_words() {
    let a = words("Can you fix RENDERER?", "The Renderer and sidebar");
    let b = words("Sidebar, please", "renderer layout");
    assert_eq!(a, HashSet::from(["fix".into(), "renderer".into(), "sidebar".into()]));
    assert_eq!(similarity(&a, &b), 500);
    assert_eq!(similarity(&a, &words("Could you please", "and the")), 0);
}

#[test]
fn provenance_and_topic_round_trip_without_turning_a_display_title_into_a_link() {
    let t = TempDir::new("project-marks");
    let roots = t.roots();
    let file = t.0.join("mock.jsonl");
    let mut marks = crate::marks::Marks::default();
    let id = "11111111-2222-4333-8444-555555555555";
    let source = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
    assert!(marks.set_handover_from(id, "Original title"));
    assert_eq!(marks.parent(id), None);
    assert!(!marks.set_parent(id, id));
    assert!(marks.set_parent(id, source));
    assert!(!marks.set_parent(id, "99999999-aaaa-4aaa-8aaa-bbbbbbbbbbbb"));
    assert!(marks.set_topic(id, &file, "  UI  layout  "));
    assert!(!marks.set_topic(id, &file, "One two three four"));
    marks.save(&roots).unwrap();
    let mut marks = crate::marks::Marks::load(&roots).unwrap();
    assert_eq!(marks.parent(id), Some(source));
    assert_eq!(marks.topic(id), Some("UI layout"));
    assert!(marks.set_topic(id, &file, ""));
    marks.clear_parent(id);
    assert_eq!((marks.topic(id), marks.parent(id), marks.handover_from(id)), (None, None, Some("Original title")));
    assert!(!file.exists());
}
