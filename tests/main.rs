// This test suite is incomplete and doesn't cover all available functionality.
// Contributions to improve test coverage would be highly appreciated!

extern crate inotify;


use std::io::File;
use std::os::tmpdir;

use inotify::INotify;
use inotify::ffi::IN_MODIFY;


#[test]
fn test_watch() {
	let (path, mut file) = temp_file();

	let mut inotify = INotify::init().unwrap_or_else(|error|
		panic!("Failed to initialize inotify: {}", error)
	);
	let watch = inotify.add_watch(&path, IN_MODIFY).unwrap_or_else(|error|
		panic!("Failed to add watch: {}", error)
	);

	write_to(&mut file);

	let event = inotify.event().unwrap_or_else(|error|
		panic!("Failed to retrieve event: {}", error)
	);

	assert_eq!(watch, event.wd);
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
