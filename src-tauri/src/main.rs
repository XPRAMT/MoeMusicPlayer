#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "windows")]
    if std::env::args().any(|argument| argument == "--diagnostics-json") {
        match moemusicplayer_lib::diagnostics_json() {
            Ok(report) => println!("{report}"),
            Err(error) => {
                eprintln!("diagnostics failed: {error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if std::env::args().any(|argument| argument == "--update-build-info") {
        println!(
            "{}",
            serde_json::json!({ "buildVersion": env!("MOE_BUILD_VERSION"), "buildTimestampUtc": env!("MOE_BUILD_TIMESTAMP_UTC"), "gitCommit": env!("MOE_GIT_COMMIT"), "architecture": std::env::consts::ARCH })
        );
        return;
    }
    #[cfg(target_os = "windows")]
    {
        let diagnostic = std::env::args().find(|argument| {
            matches!(
                argument.as_str(),
                "--update-check-json" | "--update-stage-next-launch"
            )
        });
        if let Some(diagnostic) = diagnostic {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("diagnostic runtime");
            match runtime.block_on(moemusicplayer_lib::updater::diagnostic_check(
                diagnostic == "--update-stage-next-launch",
            )) {
                Ok(state) => println!(
                    "{}",
                    serde_json::to_string(&state).expect("update state serializes")
                ),
                Err(error) => {
                    eprintln!("{error}");
                    std::process::exit(1);
                }
            }
            return;
        }
        // A freshly installed version started by the old one waits for it to exit before it starts up.
        let mut arguments = std::env::args();
        arguments.next();
        if arguments.next().as_deref() == Some("--update-relaunch") {
            if let Some(pid) = arguments.next() {
                moemusicplayer_lib::updater::wait_for_relaunch_parent(&pid);
            }
        }
    }
    #[cfg(target_os = "windows")]
    moemusicplayer_lib::initialize_process_diagnostics();
    moemusicplayer_lib::run();
}
