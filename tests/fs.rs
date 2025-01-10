use hooch::{fs::file::HoochFile, runtime::RuntimeBuilder};

const FILE_READ_NAME: &str = "./tests/resources/my_file.txt";

#[test]
fn test_hooch_read_file() {
    let runtime_handle = RuntimeBuilder::default().build();
    let actual = runtime_handle.run_blocking(async {
        let mut hooch_file = HoochFile::try_new(FILE_READ_NAME).unwrap();
        hooch_file.read_to_string().await
    });
    let expected = std::fs::read_to_string(FILE_READ_NAME).unwrap();
    assert_eq!(actual, expected);
}
