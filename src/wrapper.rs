#![allow(missing_docs)]

//! Idiomatic wrapper for inotify

use std::mem;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::slice;
use std::ffi::{
    OsStr,
    CString,
};
use std::vec;

use libc::{
    F_GETFL,
    F_SETFL,
    O_NONBLOCK,
    fcntl,
    c_int,
    c_void,
    size_t,
    ssize_t,
};
use ffi::{self, inotify_event};


pub struct Inotify {
    pub fd: c_int,
    events: Vec<Event>,
}

impl Inotify {

    pub fn init() -> io::Result<Inotify> {
        let fd = unsafe {
            // Initialize inotify and pass both `IN_CLOEXEC` and `IN_NONBLOCK`.
            //
            // `IN_NONBLOCK` is needed, because `Inotify` manages blocking
            // behavior for the API consumer, and the way we do that is to make
            // everything non-blocking by default and later override that as
            // required.
            //
            // Passing `IN_CLOEXEC` prevents leaking file descriptors to
            // processes executed by this process and seems to be a best
            // practice. I don't grasp this issue completely and failed to find
            // any authorative sources on the topic. There's some discussion in
            // the open(2) and fcntl(2) man pages, but I didn't find that
            // helpful in understanding the issue of leaked file scriptors.
            // For what it's worth, there's a Rust issue about this:
            // https://github.com/rust-lang/rust/issues/12148
            ffi::inotify_init1(ffi::IN_CLOEXEC | ffi::IN_NONBLOCK)
        };

        match fd {
            -1 => Err(io::Error::last_os_error()),
            _  => Ok(Inotify {
                fd    : fd,
                events: Vec::new(),
            })
        }
    }

    pub fn add_watch<P>(&mut self, path: P, mask: WatchMask)
        -> io::Result<WatchDescriptor>
        where P: AsRef<Path>
    {
        let path = CString::new(path.as_ref().as_os_str().as_bytes())?;

        let wd = unsafe {
            ffi::inotify_add_watch(
                self.fd,
                path.as_ptr() as *const _,
                mask.bits(),
            )
        };

        match wd {
            -1 => Err(io::Error::last_os_error()),
            _  => Ok(WatchDescriptor(wd)),
        }
    }

    pub fn rm_watch(&mut self, wd: WatchDescriptor) -> io::Result<()> {
        let result = unsafe { ffi::inotify_rm_watch(self.fd, wd.0) };
        match result {
            0  => Ok(()),
            -1 => Err(io::Error::last_os_error()),
            _  => panic!(
                "unexpected return code from inotify_rm_watch ({})", result)
        }
    }

    /// Wait until events are available, then return them.
    /// This function will block until events are available. If you want it to
    /// return immediately, use `available_events`.
    pub fn wait_for_events(&mut self) -> io::Result<Events> {
        let fd = self.fd;

        unsafe {
            fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) & !O_NONBLOCK)
        };
        let result = self.available_events();
        unsafe {
            fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK)
        };

        result
    }

    /// Returns available inotify events.
    /// If no events are available, this method will simply return a slice with
    /// zero events. If you want to wait for events to become available, call
    /// `wait_for_events`.
    pub fn available_events(&mut self) -> io::Result<Events> {
        let mut buffer = [0u8; 1024];
        let len = unsafe {
            ffi::read(
                self.fd,
                buffer.as_mut_ptr() as *mut c_void,
                buffer.len() as size_t
            )
        };

        match len {
            0 => {
                panic!(
                    "Call to read returned 0. This should never happen and may \
                    indicate a bug in inotify-rs. For example, the buffer used \
                    to read into might be too small."
                );
            }
            -1 => {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::WouldBlock {
                    return Ok(Events(self.events.drain(..)));
                }
                else {
                    return Err(error);
                }
            },
            _ =>
                ()
        }

        let event_size = mem::size_of::<inotify_event>();

        let mut i = 0;
        while i < len {
            unsafe {
                let slice = &buffer[i as usize..];

                let event = slice.as_ptr() as *const inotify_event;

                let name = if (*event).len > 0 {
                    let name_ptr = slice
                        .as_ptr()
                        .offset(event_size as isize);

                    let name_slice_with_0 = slice::from_raw_parts(
                        name_ptr,
                        (*event).len as usize,
                    );

                    // This split ensures that the slice contains no \0 bytes, as CString
                    // doesn't like them. It will replace the slice with all bytes before the
                    // first \0 byte, or just leave the whole slice if the slice doesn't contain
                    // any \0 bytes. Using .unwrap() here is safe because .splitn() always returns
                    // at least 1 result, even if the original slice contains no instances of \0.
                    let name_slice = name_slice_with_0.splitn(2, |b| b == &0u8).next().unwrap();

                    Path::new(OsStr::from_bytes(name_slice)).to_path_buf()
                }
                else {
                    PathBuf::new()
                };

                self.events.push(Event::new(&*event, name));

                i += (event_size + (*event).len as usize) as ssize_t;
            }
        }

        Ok(Events(self.events.drain(..)))
    }

    pub fn close(mut self) -> io::Result<()> {
        let result = unsafe { ffi::close(self.fd) };
        self.fd = -1;
        match result {
            0 => Ok(()),
            _ => Err(io::Error::last_os_error()),
        }
    }
}

impl Drop for Inotify {
    fn drop(&mut self) {
        if self.fd != -1 {
            unsafe { ffi::close(self.fd); }
        }
    }
}


/// Contains the [`WatchMask`](struct.WatchMask.html) flags
///
/// Contains constants for all valid
/// [`WatchMask`](struct.WatchMask.html) flags, which can be used to
/// compare against a [`WatchMask`](struct.WatchMask.html) instance
/// using its [`contains`](struct.WatchMask.html#method.contains) method.
pub mod watch_mask {
    use ffi;

    bitflags! {
        /// Mask for a file watch
        ///
        /// Passed to
        /// [`Inotify::add_watch`](../struct.Inotify.html#method.add_watch), to
        /// describe what file system events to watch for and how to do that.
        pub flags WatchMask: u32 {
            /// File was accessed.
            const ACCESS        = ffi::IN_ACCESS,

            /// Metadata changed.
            const ATTRIB        = ffi::IN_ATTRIB,

            /// File opened for writing was closed.
            const CLOSE_WRITE   = ffi::IN_CLOSE_WRITE,

            /// File or directory not opened for writing was closed.
            const CLOSE_NOWRITE = ffi::IN_CLOSE_NOWRITE,

            /// File/directory created in watched directory.
            const CREATE        = ffi::IN_CREATE,

            /// File/directory deleted from watched directory.
            const DELETE        = ffi::IN_DELETE,

            /// Watched file/directory was itself deleted.
            const DELETE_SELF   = ffi::IN_DELETE_SELF,

            /// File was modified.
            const MODIFY        = ffi::IN_MODIFY,

            /// Watched file/directory was itself moved.
            const MOVE_SELF     = ffi::IN_MOVE_SELF,

            /// Generated for the directory containing the old filename when a
            /// file is renamend.
            const MOVED_FROM    = ffi::IN_MOVED_FROM,

            /// Generated for the directory containing the new filename when a
            /// file is renamed.
            const MOVED_TO      = ffi::IN_MOVED_TO,

            /// File or directory was opened.
            const OPEN          = ffi::IN_OPEN,

            /// Watch for all events.
            const ALL_EVENTS    = ffi::IN_ALL_EVENTS,

            /// Watch for both `MOVED_FROM` and `MOVED_TO`.
            const MOVE          = ffi::IN_MOVE,

            /// Watch for both `IN_CLOSE_WRITE` and `IN_CLOSE_NOWRITE`.
            const CLOSE         = ffi::IN_CLOSE,

            /// Don't dereference the path if it is a symbolic link
            const DONT_FOLLOW   = ffi::IN_DONT_FOLLOW,

            /// Don't watch events for children that have been unlinked from
            /// watched directory.
            const EXCL_UNLINK   = ffi::IN_EXCL_UNLINK,

            /// If a watch instance already exists for the inode corresponding
            /// to the given path, amend the existing watch mask instead of
            /// replacing it.
            const MASK_ADD      = ffi::IN_MASK_ADD,

            /// Only monitor for one event, then remove the watch
            const ONESHOT       = ffi::IN_ONESHOT,

            /// Only watch path, if it is a directory.
            const ONLYDIR       = ffi::IN_ONLYDIR,
        }
    }
}

pub use self::watch_mask::WatchMask;


/// Represents a file that inotify is watching
///
/// Can be obtained from
/// [`Inotify::add_watch`](struct.Inotify.html#method.add_watch) or from an
/// [`Event`](struct.Event.html). A watch descriptor can be used to get inotify
/// to stop watching a file by passing it to
/// [`Inotify::rm_watch`](struct.Inotify.html#method.rm_watch).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WatchDescriptor(c_int);


/// Iterates over inotify events
///
/// Iterates over the events returned by
/// [`Inotify::wait_for_events`](struct.Inotify.html#method.wait_for_events) or
/// [`Inotify::available_events`](struct.Inotify.html#method.available_events).
pub struct Events<'a>(vec::Drain<'a, Event>);

impl<'a> Iterator for Events<'a> {
    type Item = Event;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }

}


#[derive(Clone, Debug)]
pub struct Event {
    pub wd    : WatchDescriptor,
    pub mask  : u32,
    pub cookie: u32,
    pub name  : PathBuf,
}

impl Event {
    fn new(event: &inotify_event, name: PathBuf) -> Event {
        Event {
            wd    : WatchDescriptor(event.wd),
            mask  : event.mask,
            cookie: event.cookie,
            name  : name,
        }
    }

    pub fn is_access(&self) -> bool {
        return self.mask & ffi::IN_ACCESS > 0;
    }

    pub fn is_modify(&self) -> bool {
        return self.mask & ffi::IN_MODIFY > 0;
    }

    pub fn is_attrib(&self) -> bool {
        return self.mask & ffi::IN_ATTRIB > 0;
    }

    pub fn is_close_write(&self) -> bool {
        return self.mask & ffi::IN_CLOSE_WRITE > 0;
    }

    pub fn is_close_nowrite(&self) -> bool {
        return self.mask & ffi::IN_CLOSE_NOWRITE > 0;
    }

    pub fn is_open(&self) -> bool {
        return self.mask & ffi::IN_OPEN > 0;
    }

    pub fn is_moved_from(&self) -> bool {
        return self.mask & ffi::IN_MOVED_FROM > 0;
    }

    pub fn is_moved_to(&self) -> bool {
        return self.mask & ffi::IN_MOVED_TO > 0;
    }

    pub fn is_create(&self) -> bool {
        return self.mask & ffi::IN_CREATE > 0;
    }

    pub fn is_delete(&self) -> bool {
        return self.mask & ffi::IN_DELETE > 0;
    }

    pub fn is_delete_self(&self) -> bool {
        return self.mask & ffi::IN_DELETE_SELF > 0;
    }

    pub fn is_move_self(&self) -> bool {
        return self.mask & ffi::IN_MOVE_SELF > 0;
    }

    pub fn is_move(&self) -> bool {
        return self.mask & ffi::IN_MOVE > 0;
    }

    pub fn is_close(&self) -> bool {
        return self.mask & ffi::IN_CLOSE > 0;
    }

    pub fn is_dir(&self) -> bool {
        return self.mask & ffi::IN_ISDIR > 0;
    }

    pub fn is_unmount(&self) -> bool {
        return self.mask & ffi::IN_UNMOUNT > 0;
    }

    pub fn is_queue_overflow(&self) -> bool {
        return self.mask & ffi::IN_Q_OVERFLOW > 0;
    }

    pub fn is_ignored(&self) -> bool {
        return self.mask & ffi::IN_IGNORED > 0;
    }
}
