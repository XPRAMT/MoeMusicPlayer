#![cfg(all(windows, feature = "test-fixtures"))]
use moe_update::{ApplyPlan, FileIdentity, PendingUpdate, ReleaseManifest};
use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

struct Fixture {
    root: PathBuf,
    plan: ApplyPlan,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "moe-update-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let stage = root.join("UserData/updates/stage");
        std::fs::create_dir_all(&stage).unwrap();
        let target = root.join("moemusicplayer.exe");
        let original = std::fs::read(env!("CARGO_BIN_EXE_updater-test-fixture")).unwrap();
        let mut old = original.clone();
        old.extend_from_slice(b"OLD BUILD");
        let mut new = original;
        new.extend_from_slice(b"NEW BUILD");
        std::fs::write(&target, old).unwrap();
        std::fs::write(stage.join("moemusicplayer.exe"), new).unwrap();
        std::fs::copy(
            env!("CARGO_BIN_EXE_moemusicplayer-updater"),
            stage.join("moemusicplayer-updater.exe"),
        )
        .unwrap();
        std::fs::copy(
            env!("CARGO_BIN_EXE_moemusicplayer-updater"),
            stage.join("running-helper.exe"),
        )
        .unwrap();
        std::fs::write(root.join("moemusicplayer-updater.exe"), b"OLD HELPER").unwrap();
        std::fs::write(root.join("UserData/sentinel.txt"), b"KEEP USER DATA").unwrap();
        let identity = |name: &str| {
            let path = stage.join(name);
            FileIdentity {
                name: name.into(),
                sha256: moe_update::file_sha256(&path).unwrap(),
                size: std::fs::metadata(path).unwrap().len(),
            }
        };
        let manifest = ReleaseManifest {
            schema_version: 1,
            version: "test".into(),
            build_timestamp_utc: "2026-10-09T01:02:03.456Z".into(),
            git_commit: "a".repeat(40),
            architecture: "windows-x86_64".into(),
            executable: identity("moemusicplayer.exe"),
            helper: identity("moemusicplayer-updater.exe"),
        };
        let pending = PendingUpdate {
            manifest,
            stage_directory: stage,
            target: target.clone(),
        };
        moe_update::write_json(&root.join("UserData/updates/pending.json"), &pending).unwrap();
        let plan = ApplyPlan {
            pending,
            parent_pid: 0,
            parent_started: 0,
            original_sha256: moe_update::file_sha256(&target).unwrap(),
            nonce: "abcdef0123456789abcdef0123456789".into(),
            wait_seconds: 2,
        };
        Self { root, plan }
    }
    fn assert_userdata(&self) {
        assert_eq!(
            std::fs::read(self.root.join("UserData/sentinel.txt")).unwrap(),
            b"KEEP USER DATA"
        );
    }
    fn parent(&self, delay: u64, permit: bool) -> std::process::Child {
        let template = self.plan.pending.stage_directory.join("template.json");
        moe_update::write_json(&template, &self.plan).unwrap();
        Command::new(&self.plan.pending.target)
            .arg("--apply")
            .arg(template)
            .arg(self.plan.pending.stage_directory.join("running-helper.exe"))
            .arg(delay.to_string())
            .arg(if permit { "permit" } else { "no-permit" })
            .spawn()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "child-process deadline expired");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn helper_waits_for_exact_parent_then_replaces_both_and_relaunches() {
    let fixture = Fixture::new();
    let mut parent = fixture.parent(700, true);
    until(|| {
        fixture
            .plan
            .pending
            .stage_directory
            .join("parent-ready")
            .exists()
    });
    assert_eq!(
        moe_update::file_sha256(&fixture.plan.pending.target).unwrap(),
        fixture.plan.original_sha256
    );
    assert!(parent.wait().unwrap().success());
    until(|| {
        fixture
            .plan
            .pending
            .stage_directory
            .join("launch-ready")
            .exists()
    });
    assert_eq!(
        moe_update::file_sha256(&fixture.plan.pending.target).unwrap(),
        fixture.plan.pending.manifest.executable.sha256
    );
    assert_eq!(
        moe_update::file_sha256(&fixture.root.join("moemusicplayer-updater.exe")).unwrap(),
        fixture.plan.pending.manifest.helper.sha256
    );
    assert!(!fixture.root.join("UserData/updates/pending.json").exists());
    fixture.assert_userdata();
}

#[test]
fn parent_timeout_leaves_original_and_does_not_kill_parent() {
    let mut fixture = Fixture::new();
    fixture.plan.wait_seconds = 1;
    let mut parent = fixture.parent(2000, true);
    until(|| {
        fixture
            .plan
            .pending
            .stage_directory
            .join("parent-ready")
            .exists()
    });
    std::thread::sleep(Duration::from_millis(1200));
    assert!(parent.try_wait().unwrap().is_none());
    assert_eq!(
        moe_update::file_sha256(&fixture.plan.pending.target).unwrap(),
        fixture.plan.original_sha256
    );
    parent.wait().unwrap();
    fixture.assert_userdata();
}

#[test]
fn missing_strict_save_permit_prevents_replacement_after_parent_exit() {
    let fixture = Fixture::new();
    fixture.parent(0, false).wait().unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        moe_update::file_sha256(&fixture.plan.pending.target).unwrap(),
        fixture.plan.original_sha256
    );
    fixture.assert_userdata();
}

#[test]
fn failed_launch_rolls_back_both_executables_and_keeps_userdata() {
    let fixture = Fixture::new();
    let result =
        moe_update::windows::apply_files(&fixture.plan, |_| Err("injected launch failure".into()));
    assert!(result.unwrap_err().contains("已還原舊版"));
    assert_eq!(
        moe_update::file_sha256(&fixture.plan.pending.target).unwrap(),
        fixture.plan.original_sha256
    );
    assert_eq!(
        std::fs::read(fixture.root.join("moemusicplayer-updater.exe")).unwrap(),
        b"OLD HELPER"
    );
    fixture.assert_userdata();
}

#[test]
fn unconfirmed_live_child_preserves_new_files_and_never_rolls_back() {
    let fixture = Fixture::new();
    let result = moe_update::windows::apply_files(&fixture.plan, |_| {
        Ok(moe_update::windows::LaunchOutcome::StillRunning)
    });
    assert!(result.unwrap_err().contains("不還原或啟動另一個程式"));
    assert_eq!(
        moe_update::file_sha256(&fixture.plan.pending.target).unwrap(),
        fixture.plan.pending.manifest.executable.sha256
    );
    assert!(fixture
        .plan
        .pending
        .stage_directory
        .join("original-main.exe")
        .exists());
    fixture.assert_userdata();
}

#[test]
fn pid_creation_identity_mismatch_is_rejected_before_ready() {
    let fixture = Fixture::new();
    let mut parent = Command::new(&fixture.plan.pending.target)
        .arg("--hold")
        .spawn()
        .unwrap();
    let result = moe_update::windows::wait_for_parent(
        parent.id(),
        0,
        &fixture.plan.pending.target,
        Duration::from_millis(10),
        || panic!("must not grant READY"),
    );
    assert!(result.is_err());
    parent.wait().unwrap();
    fixture.assert_userdata();
}
