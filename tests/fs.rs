use hooch::fs::file::HoochFile;

#[test]
fn test_hooch_file() {
    let mut hooch_file = HoochFile::try_new("./my_file.txt").unwrap();
    hooch_file.read_to_string();
    dbg!(hooch_file);
}
