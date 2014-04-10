use libc::{
	c_char,
	c_int,
	c_void,
	size_t,
	ssize_t,
	uint32_t };


// Flags for inotify_init1()
pub static IN_CLOEXEC : c_int = 0o2000000;
pub static IN_NONBLOCK: c_int = 0o4000;


// Events, used in the mask for inotify_add_watch and the inotify_event struct
pub static IN_ACCESS       : uint32_t = 0x00000001;
pub static IN_MODIFY       : uint32_t = 0x00000002;
pub static IN_ATTRIB       : uint32_t = 0x00000004;
pub static IN_CLOSE_WRITE  : uint32_t = 0x00000008;
pub static IN_CLOSE_NOWRITE: uint32_t = 0x00000010;
pub static IN_OPEN         : uint32_t = 0x00000020;
pub static IN_MOVED_FROM   : uint32_t = 0x00000040;
pub static IN_MOVED_TO     : uint32_t = 0x00000080;
pub static IN_CREATE       : uint32_t = 0x00000100;
pub static IN_DELETE       : uint32_t = 0x00000200;
pub static IN_DELETE_SELF  : uint32_t = 0x00000400;
pub static IN_MOVE_SELF    : uint32_t = 0x00000800;

pub static IN_MOVE : uint32_t = (IN_MOVED_FROM | IN_MOVED_TO);
pub static IN_CLOSE: uint32_t = (IN_CLOSE_WRITE | IN_CLOSE_NOWRITE);

pub static IN_ALL_EVENTS: uint32_t = (
	IN_ACCESS | IN_MODIFY | IN_ATTRIB | IN_CLOSE_WRITE | IN_CLOSE_NOWRITE
	| IN_OPEN | IN_MOVED_FROM | IN_MOVED_TO | IN_CREATE | IN_DELETE
	| IN_DELETE_SELF | IN_MOVE_SELF);


// Additional options that can be part of the mask for inotify_add_watch
pub static IN_ONLYDIR    : uint32_t = 0x01000000;
pub static IN_DONT_FOLLOW: uint32_t = 0x02000000;
pub static IN_EXCL_UNLINK: uint32_t = 0x04000000;
pub static IN_MASK_ADD   : uint32_t = 0x20000000;
pub static IN_ONESHOT    : uint32_t = 0x80000000;


// Additional events that can be part of the mask of inotify_event
pub static IN_ISDIR     : uint32_t = 0x40000000;
pub static IN_UNMOUNT   : uint32_t = 0x00002000;
pub static IN_Q_OVERFLOW: uint32_t = 0x00004000;
pub static IN_IGNORED   : uint32_t = 0x00008000;


#[allow(non_camel_case_types)]
#[allow(raw_pointer_deriving)]
#[deriving(Show)]
pub struct inotify_event {
	pub wd    : c_int,
	pub mask  : uint32_t,
	pub cookie: uint32_t,
	pub len   : uint32_t,
	pub name  : *c_char
}


extern {
	pub fn inotify_init() -> c_int;
	pub fn inotify_init1(flags: c_int) -> c_int;
	pub fn inotify_add_watch(fd: c_int, pathname: *c_char, mask: uint32_t) -> c_int;
	pub fn inotify_rm_watch(fd: c_int, wd: c_int) -> c_int;
	pub fn read(fd: c_int, buf: *c_void, count: size_t) -> ssize_t;
	pub fn close(fd: c_int) -> c_int;
}
