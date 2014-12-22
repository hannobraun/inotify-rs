// This test suite is incomplete and doesn't cover all available functionality.
// Contributions to improve test coverage would be highly appreciated!

extern crate inotify;


use std::io::File;
use std::os::tmpdir;

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


fn temp_file() -> (Path, File) {
	let path = tmpdir().join("test-file");
	let file = File::create(&path).unwrap_or_else(|error|
		panic!("Failed to create temporary file: {}", error)
	);

	(path, file)
}

fn write_to(file: &mut File) {
	file
		.write_line("This should trigger an inotify event.")
		.unwrap_or_else(|error|
			panic!("Failed to write to file: {}", error)
		);
}
