mod watcher;

fn main() {
    env_logger::init();
    println!("Hello, world!");
    if let Err(error) = watcher::watch("/home/lukas/Downloads") {
        log::error!("Error: {error:?}");
    }
}
