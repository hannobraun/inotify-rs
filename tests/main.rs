// This test suite is incomplete and doesn't cover all available functionality.
// Contributions to improve test coverage would be highly appreciated!

extern crate inotify;


use std::io::{
	File,
	TempDir,
};

use inotify::INotify;
use inotify::ffi::IN_MODIFY;


#[test]
fn test_watch() {
	let temp_dir = TempDir::new("inotify-test").unwrap_or_else(|error|
		panic!("Failed to create temporary directory: {}", error)
	);
	let     temp_file_path = temp_dir.path().join("test-file");
	let mut temp_file      = File::create(&temp_file_path);


	let mut inotify = INotify::init().unwrap_or_else(|error|
		panic!("Failed to initialize inotify: {}", error)
	);
	let watch = inotify.add_watch(&temp_file_path, IN_MODIFY).unwrap_or_else(|error|
		panic!("Failed to add watch: {}", error)
	);

	temp_file
		.write_line("This should trigger an inotify event.")
		.unwrap_or_else(|error|
			panic!("Failed to write to file: {}", error)
		);

	let event = inotify.event().unwrap_or_else(|error|
		panic!("Failed to retrieve event: {}", error)
	);

	assert_eq!(watch, event.wd);
}
