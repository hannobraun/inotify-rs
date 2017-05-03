extern crate inotify;
extern crate tempdir;


use std::fs::File;

use inotify::{
    watch_mask,
    Inotify,
};
use tempdir::TempDir;


fn main() {
    let mut inotify = Inotify::init()
        .expect("Failed to initialize inotify");

    let temp_dir = TempDir::new("inotify-rs-modify-example")
        .expect("Failed to create temporary directory");
    let path = temp_dir.path().join("file");

    File::create(&path)
        .expect("Failed to create temporary file");

    print!("Watching {}\n", path.display());
    print!("If you modify this file, this program should print a message.\n\n");

    inotify
        .add_watch(path, watch_mask::MODIFY)
        .expect("Failed to add inotify watch");

    let mut buffer = [0u8; 4096];
    loop {
        let events = inotify
            .read_events_blocking(&mut buffer)
            .expect("Failed to read inotify events");

        for _ in events {
            print!("MODIFIED\n");
        }
    }
}
