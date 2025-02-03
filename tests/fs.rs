use std::{fs::OpenOptions, path::PathBuf};

use hooch::{
    fs::{file::HoochFile, traits::OpenHooch},
    runtime::{Handle, RuntimeBuilder},
};

const RESOURCES_DIR: &str = "./tests/resources";
const FILE_READ_NAME: &str = "./tests/resources/my_file.txt";

fn build_runtime() -> Handle {
    RuntimeBuilder::default().build()
}

#[test]
fn test_hooch_open_file_ok() {
    let runtime_handle = build_runtime();
    let actual = runtime_handle.run_blocking(async { HoochFile::open(FILE_READ_NAME).await });
    assert!(actual.is_ok());
}

#[test]
fn test_hooch_open_file_err() {
    let runtime_handle = build_runtime();
    let actual = runtime_handle.run_blocking(async { HoochFile::open("does not exist").await });
    assert!(actual.is_err());
}

#[test]
fn test_hooch_open_hooch_trait() {
    const TMP_FILE_NAME: &str = "open_hooch_trait.txt";
    let file_path = PathBuf::from(format!("{}/{}", RESOURCES_DIR, TMP_FILE_NAME));
    let file_path_clone = file_path.clone();

    if file_path.exists() {
        let _ = std::fs::remove_file(&file_path);
    }

    let runtime_handle = build_runtime();
    runtime_handle.run_blocking(async move {
        let _ = OpenOptions::new()
            .create(true)
            .append(true)
            .open_hooch(&file_path)
            .await;
    });

    assert!(file_path_clone.exists());
    let _ = std::fs::remove_file(&file_path_clone);
}

#[test]
fn test_hooch_read_to_string_file() {
    let runtime_handle = build_runtime();
    let actual = runtime_handle.run_blocking(async {
        let mut hooch_file = HoochFile::open(FILE_READ_NAME).await.unwrap();
        hooch_file.read_to_string().await
    });
    let expected = std::fs::read_to_string(FILE_READ_NAME).unwrap();
    assert_eq!(actual, expected);
}
