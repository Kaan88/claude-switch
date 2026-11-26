use super::*;
use std::time::Duration;

#[derive(Default)]
pub(super) struct PendingOwners(VecDeque<PendingOwner>);

struct PendingOwner {
    id: String,
    account: String,
    cwd: String,
    expires: Instant,
}

impl PendingOwner {
    fn matches(&self, records: &[Running]) -> bool {
        let mut matches = records.iter().filter(|r| {
            r.app == App::Cli
                && r.session_id.eq_ignore_ascii_case(&self.id)
                && r.account == self.account
                && folder_key(&r.cwd) == self.cwd
        });
        matches.next().is_some() && matches.next().is_none()
    }
}

impl PendingOwners {
    pub(super) fn launched(&mut self, launch: &Launch, account: Option<&str>, now: Instant) {
        if launch.place != claude_switch_core::Place::Terminal {
            return;
        }
        let Some((account, cwd)) = account.zip(launch.cwd.as_ref()) else { return };
        let Some(id) = launch.args.windows(2).find(|args| args[0] == "--session-id").map(|args| &args[1]) else {
            return;
        };
        if !valid_id(id) {
            return;
        }
        if self.0.len() == 64 {
            self.0.pop_front();
        }
        self.0.push_back(PendingOwner {
            id: id.clone(),
            account: account.into(),
            cwd: folder_key(cwd),
            expires: now + Duration::from_secs(120),
        });
    }
}

impl AppState {
    pub fn pending_external_ownership(&self) -> bool {
        self.restore.external_owners.0.iter().any(|owner| owner.expires > Instant::now())
    }

    pub fn verify_external_owners(&mut self) -> bool {
        let Source::Real(roots) = &self.source else { return false };
        let now = Instant::now();
        let mut changed = false;
        self.restore.external_owners.0.retain(|owner| {
            if owner.expires <= now {
                return false;
            }
            if !owner.matches(&self.running)
                || claude_switch_core::launch::record_verified_owner(roots, &owner.id, &owner.account).is_err()
            {
                return true;
            }
            if self.selected.as_deref() == Some(&owner.id) {
                self.last_account = Some((owner.id.clone(), owner.account.clone()));
                changed = true;
            }
            false
        });
        changed
    }
}

#[test]
fn external_ownership_needs_one_live_matching_preallocated_identity() {
    let cwd = std::env::temp_dir();
    let id = "11111111-0000-0000-0000-000000000000";
    let mut launch = claude_switch_core::launch::shell(&cwd);
    launch.args.extend(["--session-id".into(), id.into()]);
    let mut pending = PendingOwners::default();
    pending.launched(&launch, Some("work"), Instant::now());
    let owner = &pending.0[0];
    let mut live = Running { session_id: id.into(), pid: 1, app: App::Cli, account: "main".into(), cwd };
    assert!(!owner.matches(&[]));
    assert!(!owner.matches(std::slice::from_ref(&live)));
    live.account = "work".into();
    assert!(owner.matches(std::slice::from_ref(&live)));
    assert!(!owner.matches(&[live.clone(), Running { pid: 2, ..live }]));
}
