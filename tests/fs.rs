use hooch::{
    fs::file::HoochFile,
    runtime::{Handle, RuntimeBuilder},
};

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
fn test_hooch_read_file() {
    let runtime_handle = build_runtime();
    let actual = runtime_handle.run_blocking(async {
        let mut hooch_file = HoochFile::open(FILE_READ_NAME).await.unwrap();
        hooch_file.read_to_string().await
    });
    let expected = std::fs::read_to_string(FILE_READ_NAME).unwrap();
    assert_eq!(actual, expected);
}
