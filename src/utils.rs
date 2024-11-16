pub mod ring_buffer;

#[cfg(debug_assertions)]
pub fn thread_name() -> String {
    std::thread::current().name().unwrap().to_string()
}
