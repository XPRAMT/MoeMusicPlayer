#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
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
        let mut arguments = std::env::args_os().skip(1);
        if arguments.next().as_deref() == Some(std::ffi::OsStr::new("--update-launched")) {
            let result = arguments
                .next()
                .ok_or_else(|| "缺少更新交接目錄。".to_owned())
                .and_then(|stage| {
                    moemusicplayer_lib::updater::acknowledge_updated_launch(std::path::Path::new(
                        &stage,
                    ))
                });
            if let Err(error) = result {
                eprintln!("{error}");
                return;
            }
        } else if moemusicplayer_lib::updater::finish_pending_before_app() {
            return;
        }
    }
    moemusicplayer_lib::run();
}
