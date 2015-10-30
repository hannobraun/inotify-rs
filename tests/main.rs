// This test suite is incomplete and doesn't cover all available functionality.
// Contributions to improve test coverage would be highly appreciated!

extern crate inotify;
extern crate tempdir;

use std::fs::File;
use std::io::Write;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use tempdir::TempDir;

use inotify::INotify;
use inotify::ffi::IN_MODIFY;


#[test]
fn it_should_watch_a_file() {
    let mut testdir = TestDir::new();
    let (path, mut file) = testdir.new_file();

	let mut inotify = INotify::init().unwrap();
	let watch = inotify.add_watch(&path, IN_MODIFY).unwrap();

	write_to(&mut file);

	let events = inotify.wait_for_events().unwrap();
	assert!(events.len() > 0);
	for event in events.iter() {
		assert_eq!(watch, event.wd);
	}
}

#[test]
fn it_should_return_immediately_if_no_events_are_available() {
	let mut inotify = INotify::init().unwrap();

	assert_eq!(0, inotify.available_events().unwrap().len());
}

#[test]
fn it_should_wait_for_events() {
    let mut testdir = TestDir::new();
    let (path, mut file) = testdir.new_file();

    let mut inotify = INotify::init().unwrap();
    inotify.add_watch(&path, IN_MODIFY).unwrap();

    thread::spawn(move|| {
        thread::sleep_ms(50);
        write_to(&mut file);
    });

    let events = inotify.wait_for_events().unwrap();
    assert!(events.len() > 0);
}

#[test]
fn it_should_be_asynchronously_closable() {
    let mut inotify = INotify::init().unwrap();
    let closer = inotify.closer();

    thread::spawn(move|| {
        thread::sleep_ms(50);
        closer.close_sync();
    });

    inotify.wait_for_events().unwrap();
}

#[test]
fn it_should_wait_for_the_inotify_descriptor_to_be_closed() {
    let mut inotify = INotify::init().unwrap();
    let closer = inotify.closer();
    let waited = Arc::new(AtomicBool::new(false));
    let waited_clone = waited.clone();

    thread::spawn(move|| {
        thread::sleep_ms(50);
        waited_clone.store(true, Ordering::Relaxed);
        inotify.wait_for_events().unwrap();
    });

    closer.close_sync();
    assert!(waited.load(Ordering::Relaxed));
}

#[test]
fn it_should_not_return_duplicate_events() {
    let mut testdir = TestDir::new();
    let (path, mut file) = testdir.new_file();

	let mut inotify = INotify::init().unwrap();
	inotify.add_watch(&path, IN_MODIFY).unwrap();

	write_to(&mut file);
	inotify.wait_for_events().unwrap();

	assert_eq!(0, inotify.available_events().unwrap().len());
}

#[test]
fn it_should_handle_file_names_correctly() {
    let mut testdir = TestDir::new();
    let (mut path, mut file) = testdir.new_file();
	let file_name = path
        .file_name().unwrap()
        .to_str().unwrap()
        .to_string();
	path.pop(); // Get path to the directory the file is in

	let mut inotify = INotify::init().unwrap();
	inotify.add_watch(&path, IN_MODIFY).unwrap();

	write_to(&mut file);

	let events = inotify.wait_for_events().unwrap();
	assert!(events.len() > 0);
	for event in events {
		assert_eq!(file_name, event.name);
	}
}

fn write_to(file: &mut File) {
	file
		.write(b"This should trigger an inotify event.")
		.unwrap_or_else(|error|
			panic!("Failed to write to file: {}", error)
		);
}

struct TestDir {
    dir: TempDir,
    counter: u32,
}

impl TestDir {
    fn new() -> TestDir {
        TestDir {
            dir: TempDir::new("inotify-rs-test").unwrap(),
            counter: 0,
        }
    }

    fn new_file(&mut self) -> (PathBuf, File) {
        let id = self.counter;
        self.counter += 1;

        let path = self.dir.path().join("file-".to_string() + &id.to_string());
        let file = File::create(&path)
            .unwrap_or_else(|error| panic!("Failed to create temporary file: {}", error));

        (path, file)
    }
}

