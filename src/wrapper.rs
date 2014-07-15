use libc::{
	c_char,
	c_int,
	c_void,
};
use std::c_str::CString;
use std::mem;
use std::io::{
	EndOfFile,
	IoError,
	IoResult
};

use ffi;
use ffi::inotify_event;


pub type Watch = c_int;


pub struct INotify {
	pub fd: c_int,
	events: Vec<Event>,
}

impl INotify {
	pub fn init() -> IoResult<INotify> {
		INotify::init_with_flags(0)
	}

	pub fn init_with_flags(flags: int) -> IoResult<INotify> {
		let fd = unsafe { ffi::inotify_init1(flags as c_int) };

		match fd {
			-1 => Err(IoError::last_error()),
			_  => Ok(INotify {
				fd    : fd,
				events: Vec::new(),
			})
		}
	}

	pub fn add_watch(&self, path: &Path, mask: u32) -> IoResult<Watch> {
		let wd = unsafe {
			ffi::inotify_add_watch(
				self.fd,
				path.to_c_str().unwrap(),
				mask)
		};

		match wd {
			-1 => Err(IoError::last_error()),
			_  => Ok(wd)
		}
	}

	pub fn rm_watch(&self, watch: Watch) -> IoResult<()> {
		let result = unsafe { ffi::inotify_rm_watch(self.fd, watch) };
		match result {
			0  => Ok(()),
			-1 => Err(IoError::last_error()),
			_  => fail!(
				"unexpected return code from inotify_rm_watch ({})", result)
		}
	}

	pub fn event(&mut self) -> IoResult<Event> {
		match self.events.pop() {
			Some(event) =>
				return Ok(event),
			None =>
				()
		};

		let mut buffer = [0u8, ..1024];
		let len = unsafe {
			ffi::read(
				self.fd,
				buffer.as_mut_ptr() as *mut c_void,
				buffer.len() as u64)
		};

		match len {
			0  => return Err(IoError {
				kind  : EndOfFile,
				desc  : "end of file",
				detail: None
			}),
			-1 => return Err(IoError::last_error()),

			_ => ()
		}

		let event_size = mem::size_of::<inotify_event>();

		let mut i = 0;
		while i < len {
			unsafe {
				let slice = buffer.slice_from(i as uint);

				let event = slice.as_ptr() as *const inotify_event;

				let name = if (*event).len > 0 {
					let c_str = CString::new(
						event.offset(1) as *const c_char,
						false);

					match c_str.as_str() {
						Some(string)
							=> string.to_string(),
						None =>
							fail!("Failed to convert C string into Rust string")
					}
				}
				else {
					"".to_string()
				};

				self.events.push(Event::new(&*event, name));

				i += (event_size + (*event).len as uint) as i64;
			}
		}

		Ok(self.events.pop().expect("expected event"))
	}

	pub fn close(&self) -> IoResult<()> {
		let result = unsafe { ffi::close(self.fd) };
		match result {
			0 => Ok(()),
			_ => Err(IoError::last_error())
		}
	}
}


pub struct Event {
	pub wd    : i32,
	pub mask  : u32,
	pub cookie: u32,
	pub name  : String,
}

impl Event {
	fn new(event: &inotify_event, name: String) -> Event {
		Event {
			wd    : event.wd,
			mask  : event.mask,
			cookie: event.cookie,
			name  : name,
		}
	}

	pub fn access(&self) -> bool {
		return self.mask & ffi::IN_ACCESS > 0;
	}

	pub fn modify(&self) -> bool {
		return self.mask & ffi::IN_MODIFY > 0;
	}

	pub fn attrib(&self) -> bool {
		return self.mask & ffi::IN_ATTRIB > 0;
	}

	pub fn close_write(&self) -> bool {
		return self.mask & ffi::IN_CLOSE_WRITE > 0;
	}

	pub fn close_nowrite(&self) -> bool {
		return self.mask & ffi::IN_CLOSE_NOWRITE > 0;
	}

	pub fn open(&self) -> bool {
		return self.mask & ffi::IN_OPEN > 0;
	}

	pub fn moved_from(&self) -> bool {
		return self.mask & ffi::IN_MOVED_FROM > 0;
	}

	pub fn moved_to(&self) -> bool {
		return self.mask & ffi::IN_MOVED_TO > 0;
	}

	pub fn create(&self) -> bool {
		return self.mask & ffi::IN_CREATE > 0;
	}

	pub fn delete(&self) -> bool {
		return self.mask & ffi::IN_DELETE > 0;
	}

	pub fn delete_self(&self) -> bool {
		return self.mask & ffi::IN_DELETE_SELF > 0;
	}

	pub fn move_self(&self) -> bool {
		return self.mask & ffi::IN_MOVE_SELF > 0;
	}

	pub fn move(&self) -> bool {
		return self.mask & ffi::IN_MOVE > 0;
	}

	pub fn close(&self) -> bool {
		return self.mask & ffi::IN_CLOSE > 0;
	}

	pub fn is_dir(&self) -> bool {
		return self.mask & ffi::IN_ISDIR > 0;
	}

	pub fn unmount(&self) -> bool {
		return self.mask & ffi::IN_UNMOUNT > 0;
	}

	pub fn queue_overflow(&self) -> bool {
		return self.mask & ffi::IN_Q_OVERFLOW > 0;
	}

	pub fn ignored(&self) -> bool {
		return self.mask & ffi::IN_IGNORED > 0;
	}
}
