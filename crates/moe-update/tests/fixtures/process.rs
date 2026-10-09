#[cfg(windows)]
fn main() {
    use std::{
        io::{BufRead, Write},
        path::PathBuf,
    };
    let args: Vec<_> = std::env::args().collect();
    if args[1] == "--update-launched" {
        let stage = PathBuf::from(&args[2]);
        let plan: moe_update::ApplyPlan =
            moe_update::read_json(&stage.join("apply-plan.json")).unwrap();
        std::fs::write(stage.join("launch-ready"), plan.nonce).unwrap();
        return;
    }
    if args[1] == "--hold" {
        std::thread::sleep(std::time::Duration::from_secs(2));
        return;
    }
    let template = PathBuf::from(&args[2]);
    let helper = PathBuf::from(&args[3]);
    let delay: u64 = args[4].parse().unwrap();
    let permit = args[5] == "permit";
    let mut plan: moe_update::ApplyPlan = moe_update::read_json(&template).unwrap();
    plan.parent_pid = std::process::id();
    plan.parent_started = moe_update::windows::process_started(plan.parent_pid).unwrap();
    let plan_path = plan.pending.stage_directory.join("apply-plan.json");
    moe_update::write_json(&plan_path, &plan).unwrap();
    let mut child = std::process::Command::new(helper)
        .arg(plan_path)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut ready = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready.trim(), "READY");
    if permit {
        let mut file = std::fs::File::create(plan.pending.stage_directory.join("permit")).unwrap();
        file.write_all(plan.nonce.as_bytes()).unwrap();
        file.sync_all().unwrap();
    }
    std::fs::write(plan.pending.stage_directory.join("parent-ready"), b"ready").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(delay));
}
#[cfg(not(windows))]
fn main() {}
