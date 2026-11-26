use super::*;
use std::fs;

fn put(root: &Path, path: &str, contents: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn markdown(name: &str, description: &str) -> String {
    format!("---\nname: '{name}'\ndescription: '{description}'\n---\nPRIVATE PROMPT")
}

#[test]
fn parses_bounded_metadata_not_the_agent_prompt() {
    let parsed = frontmatter(
        "\u{feff}---\r\ndescription: >-\r\n  Look for changes.\r\n  Don't make edits.\r\n\
         name: \"reviewer\" # comment\r\ntools: Read, Grep\r\n---\r\nPRIVATE PROMPT"
            .as_bytes(),
    )
    .unwrap();
    assert_eq!(parsed, Agent { name: "reviewer".into(), description: "Look for changes. Don't make edits.".into() });
    assert_eq!(scalar(" 'doesn''t include # as a comment'  # comment").unwrap(), "doesn't include # as a comment");
    for invalid in [
        "# Missing frontmatter\nname: agent",
        "---\nname: abc",
        "---\nname: --help\n---",
        "---\nname: sci:team\n---",
        "---\nname: first\nname: second\n---",
        "---\nname: *alias\n---",
        "---\nname: 'looks-valid' extra-data\n---",
    ] {
        assert!(frontmatter(invalid.as_bytes()).is_none(), "{invalid}");
    }
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cs-agents-{}", crate::compact_open::session_id().unwrap()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn account(&self, slug: &str, primary: bool) -> Account {
        Account {
            slug: slug.into(),
            name: slug.into(),
            color: [0; 3],
            cli_dir: if primary { self.0.join(".claude") } else { self.0.join(slug) },
            desktop_dir: self.0.join("unused-desktop"),
            primary,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn project_overrides_the_target_account_and_plugin_catalog_is_scoped_and_read_only() {
    let fixture = Fixture::new();
    let dir = &fixture.0;
    let roots = Roots { home: dir.clone(), appdata: dir.clone() };
    let main = fixture.account("main", true);
    let target = fixture.account("target", false);
    let repo = dir.join("repo");
    let cwd = repo.join("sub");
    fs::create_dir_all(&cwd).unwrap();
    let registry = target.cli_dir.join("plugins/installed_plugins.json");
    put(&main.cli_dir, "agents/main.md", &markdown("main", "Main only"));
    put(&target.cli_dir, "agents/account.md", &markdown("account", "Target only"));
    put(&target.cli_dir, "agents/reviewer.md", &markdown("reviewer", "User reviewer"));
    put(&repo, ".git/HEAD", "ref: refs/heads/test");
    put(&repo, ".claude/agents/nested/reviewer.md", &markdown("reviewer", "Project reviewer"));
    put(
        &target.cli_dir,
        "settings.json",
        r#"{"enabledPlugins":{"tools@market":true,"disabled@market":true,"elsewhere@market":true,"remote@market":true}}"#,
    );
    put(&repo, ".claude/settings.local.json", r#"{"enabledPlugins":{"disabled@market":false}}"#);
    let registry_text = serde_json::json!({
        "version": 2,
        "plugins": {
            "tools@market": [
                {"scope":"user","installPath":target.cli_dir.join("plugins/cache/market/tools/1")},
                {"scope":"local","projectPath":repo,"installPath":target.cli_dir.join("plugins/cache/market/tools/2")}
            ],
            "disabled@market": [{"scope":"user","installPath":target.cli_dir.join("plugins/cache/market/disabled/1")}],
            "elsewhere@market": [{"scope":"project","projectPath":dir.join("elsewhere"),
                                   "installPath":target.cli_dir.join("plugins/cache/market/elsewhere/1")}],
            "remote@market": [{"scope":"user","installPath":r"\\must-not-resolve.invalid\share\remote"}]
        }
    })
    .to_string();
    put(&target.cli_dir, "plugins/installed_plugins.json", &registry_text);
    for (plugin, version, description) in [
        ("tools", "1", "Version one"),
        ("tools", "2", "Version two"),
        ("disabled", "1", "Disabled"),
        ("elsewhere", "1", "Not in scope"),
    ] {
        put(
            &target.cli_dir,
            &format!("plugins/cache/market/{plugin}/{version}/agents/core/check.md"),
            &markdown("check", description),
        );
    }
    let found = discover(&roots, &target, &cwd, || false);
    assert_eq!(
        found.iter().map(|a| (a.name.as_str(), a.description.as_str())).collect::<Vec<_>>(),
        [("account", "Target only"), ("reviewer", "Project reviewer"), ("tools:core:check", "Version two")]
    );
    assert_eq!(fs::read_to_string(registry).unwrap(), registry_text);
    assert_eq!(
        discover(&roots, &main, &cwd, || false).into_iter().map(|a| a.name).collect::<Vec<_>>(),
        ["main", "reviewer"]
    );
}

#[test]
fn secondary_missing_agents_can_see_the_main_folder_without_provisioning_it() {
    let fixture = Fixture::new();
    let account = fixture.account("further", false);
    let roots = Roots { home: fixture.0.clone(), appdata: fixture.0.clone() };
    put(&roots.primary_cli_dir(), "agents/main.md", &markdown("inherited", "Shared on launch"));
    assert_eq!(discover(&roots, &account, &fixture.0, || false)[0].name, "inherited");
    assert!(!account.cli_dir.exists());
    claude_switch_core::share::apply(&roots, &account).unwrap();
    assert!(Volume::at(&account.cli_dir).unwrap().folder(&account.cli_dir.join("agents")).is_none());
    assert_eq!(discover(&roots, &account, &fixture.0, || false)[0].name, "inherited");
}
