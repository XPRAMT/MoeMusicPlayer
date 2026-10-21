//! Opt-in live integration check for the production SMB share-listing path and NDR32 SRVSVC against Samba.
//! Credentials are read only from the process environment and are never printed.

#[test]
fn samba_accepts_ndr32_and_enumerates_share_names() {
    let (Ok(host), Ok(port), Ok(user), Ok(password), Ok(expected_share)) = (
        std::env::var("MOE_NDR32_SMB_HOST"),
        std::env::var("MOE_NDR32_SMB_PORT"),
        std::env::var("MOE_NDR32_SMB_USER"),
        std::env::var("MOE_NDR32_SMB_PASSWORD"),
        std::env::var("MOE_NDR32_SMB_EXPECT_SHARE"),
    ) else {
        eprintln!("skipped: MOE_NDR32_SMB_* is not set");
        return;
    };
    let Ok(port) = port.parse::<u16>() else {
        panic!("MOE_NDR32_SMB_PORT must be a valid TCP port");
    };
    let password = zeroize::Zeroizing::new(password);

    // Exercise the same entry point used by Tauri's `remote_smb_shares` command.
    // Guest access stays disabled for this authenticated-account integration test.
    let names = player_remote::smb::list_shares(&host, Some(port), &user, password.as_str())
        .expect("production MoeMusicPlayer list_shares should succeed over NDR32");
    eprintln!("production list_shares status=success shares={names:?}");
    assert!(names.iter().any(|name| name == &expected_share));

    let client = smb::Client::new(smb::ClientConfig {
        connection: smb::ConnectionConfig {
            port: Some(port),
            timeout: Some(std::time::Duration::from_secs(15)),
            allow_unsigned_guest_access: false,
            ..Default::default()
        },
        ..Default::default()
    });
    let ipc = smb::UncPath::ipc_share(&host).expect("valid SMB host");
    client
        .share_connect(&ipc, &user, password.to_string())
        .expect("authenticate to test IPC share");
    let pipe = client.open_pipe(&host, "srvsvc").expect("open SRVSVC pipe");
    let mut srvsvc: smb_rpc::interface::SrvSvc<_> = pipe.bind().expect("bind SRVSVC");
    assert_eq!(
        srvsvc.transfer_syntax(),
        smb_rpc::interface::RpcTransferSyntax::Ndr32,
        "test server must negotiate the NDR32 context"
    );
    eprintln!("selected syntax=NDR32");
    let shares = srvsvc
        .netr_share_enum(&host)
        .expect("decode NDR32 NetrShareEnum response");
    let share_names = shares
        .iter()
        .filter_map(|share| {
            share
                .netname
                .as_ref()
                .map(|name| name.to_string().trim_matches('\0').to_owned())
        })
        .collect::<Vec<_>>();
    eprintln!("NetrShareEnum status=success shares={share_names:?}");
    assert!(
        share_names.iter().any(|name| name == &expected_share),
        "share enumeration response did not contain the configured test share"
    );
    drop(srvsvc);
    let _ = client.close();
}
