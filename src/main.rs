fn main() {
    if let Err(err) = wgpu_cube::run() {
        eprintln!("Application error: {err}");
    }
}
