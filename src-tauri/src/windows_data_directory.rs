use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub(crate) fn user_data_dir_for_executable(executable: &Path) -> io::Result<PathBuf> {
    if !executable.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "current executable path must be absolute",
        ));
    }

    let executable_directory = executable.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "current executable path has no parent directory",
        )
    })?;
    let user_data_directory = executable_directory.join("UserData");

    if user_data_directory.exists() {
        let metadata = fs::metadata(&user_data_directory)?;
        if !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "portable data location is not a directory: {}",
                    user_data_directory.display()
                ),
            ));
        }

        let physical_executable_directory = fs::canonicalize(executable_directory)?;
        let physical_user_data_directory = fs::canonicalize(&user_data_directory)?;
        if physical_user_data_directory.parent() != Some(physical_executable_directory.as_path()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "portable data directory resolves outside the executable directory: {}",
                    physical_user_data_directory.display()
                ),
            ));
        }
    }

    Ok(user_data_directory)
}

#[cfg(test)]
mod tests {
    use super::user_data_dir_for_executable;
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("test clock after epoch")
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("moe-user-data-dir-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).expect("create isolated test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn data_directory_is_sibling_of_the_executable_and_ignores_cwd() {
        let directory = TestDirectory::new();
        let executable = directory.0.join("release").join("moemusicplayer.exe");
        fs::create_dir_all(executable.parent().expect("executable parent"))
            .expect("create fake release directory");
        fs::write(&executable, b"isolated test marker").expect("create fake executable");

        let expected = executable.parent().unwrap().join("UserData");
        let first = user_data_dir_for_executable(&executable).expect("resolve sibling data path");
        let second = user_data_dir_for_executable(&executable).expect("resolve again");

        assert_eq!(first, expected);
        assert_eq!(second, expected);
    }

    #[test]
    fn existing_user_data_must_physically_remain_under_executable_directory() {
        let directory = TestDirectory::new();
        let executable_directory = directory.0.join("release");
        fs::create_dir_all(executable_directory.join("UserData"))
            .expect("create isolated portable data directory");
        let executable = executable_directory.join("moemusicplayer.exe");
        fs::write(&executable, b"isolated test marker").expect("create fake executable");

        let resolved =
            user_data_dir_for_executable(&executable).expect("validate physical sibling");
        let physical_executable_directory = fs::canonicalize(&executable_directory)
            .expect("resolve executable directory through filesystem");
        let physical_user_data_directory =
            fs::canonicalize(&resolved).expect("resolve UserData through filesystem");
        assert_eq!(
            physical_user_data_directory.parent(),
            Some(physical_executable_directory.as_path())
        );
    }

    #[test]
    fn relative_executable_paths_are_rejected() {
        assert!(user_data_dir_for_executable(Path::new("moemusicplayer.exe")).is_err());
    }
}
