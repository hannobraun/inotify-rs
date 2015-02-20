// This test suite is incomplete and doesn't cover all available functionality.
// Contributions to improve test coverage would be highly appreciated!

#![feature(io, os, fs, env, path, old_path, std_misc)]

extern crate inotify;


use std::fs::File;
use std::io::Write;

use std::env::temp_dir;
use std::path::PathBuf;
use std::ffi::AsOsStr;

use inotify::INotify;
use inotify::ffi::IN_MODIFY;


#[test]
fn it_should_watch_a_file() {
	let (path, mut file) = temp_file();

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
fn it_should_not_return_duplicate_events() {
	// Usually, the write in this test seems to generate only one event, but
	// sometimes it generates multiple events. If that happens, the assertion at
	// the end will likely fail.
	//
	// I'm not sure why that happens, and since the test works as intended most
	// of the time, I don't think it's that big of a deal. If you happen to know
	// more about the subject, please consider to fix the test accordingly. It
	// would be greatly appreciated!

	let (path, mut file) = temp_file();

	let mut inotify = INotify::init().unwrap();
	inotify.add_watch(&path, IN_MODIFY).unwrap();

	write_to(&mut file);
	inotify.wait_for_events().unwrap();

	assert_eq!(0, inotify.available_events().unwrap().len());
}

#[test]
fn it_should_handle_file_names_correctly() {
	let (mut path, mut file) = temp_file();
	let file_name = file.path().unwrap()
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


fn temp_file() -> (PathBuf, File) {
	let path = temp_dir().join("test-file");
	let file = File::create(&path).unwrap_or_else(|error|
		panic!("Failed to create temporary file: {}", error)
	);
	let pathbuf = PathBuf::new(path.as_os_str());

	(pathbuf, file)
}

fn write_to(file: &mut File) {
	file
		.write(b"This should trigger an inotify event.")
		.unwrap_or_else(|error|
			panic!("Failed to write to file: {}", error)
		);
}
