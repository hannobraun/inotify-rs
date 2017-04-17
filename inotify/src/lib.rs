#![crate_name = "inotify"]
#![crate_type = "lib"]
#![warn(missing_docs)]

//! Binding and wrapper for inotify.
//!
//! [Inotify][wiki] is a linux kernel mechanism for monitoring
//! changes to filesystems' contents.
//!
//! > The inotify API provides a mechanism for monitoring filesystem
//! > events. Inotify can be used to monitor individual files, or to
//! > monitor directories. When a directory is monitored, inotify will
//! > return events for the directory itself, and for files inside the
//! > directory.
//!
//! See the [man page][inotify7] for usage information
//! of the C version, which this package follows closely.
//!
//! [wiki]: https://en.wikipedia.org/wiki/Inotify
//! [inotify7]: http://man7.org/linux/man-pages/man7/inotify.7.html


#[macro_use]
extern crate bitflags;

extern crate libc;
extern crate inotify_sys as ffi;


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


/// Idiomatic Rust wrapper for Linux's inotify API
///
/// `Inotify` is a wrapper around an inotify instance. It generally tries to
/// adhere to the underlying inotify API as closely as possible, while at the
/// same time making access to it safe and convenient.
///
/// Please note that using inotify correctly is not always trivial, and while
/// this wrapper tries to alleviate that, it is not perfect. Please refer to the
/// inotify man pages for potential problems to watch out for.
///
/// # Examples
///
/// ```
/// use inotify::{
///     Inotify,
///     watch_mask,
/// };
///
/// let mut inotify = Inotify::init()
///     .expect("Error while initializing inotify instance");
///
/// // Watch for modify and close events.
/// // Ignore returned error, as this is an example, and the file we're trying
/// // to watch here doesn't actually exist.
/// let _ = inotify.add_watch(
///     "path/to/file",
///     watch_mask::MODIFY | watch_mask::CLOSE,
/// );
///
/// let events = inotify.available_events()
///     .expect("Error while reading events");
///
/// for event in events {
///     // Handle event
/// }
/// ```
pub struct Inotify {
    fd    : c_int,
    events: Vec<Event>,
}

impl Inotify {
    /// Creates an [`Inotify`] instance
    ///
    /// Initializes an inotify instance by calling [`inotify_init1`].
    ///
    /// This method passes both flags accepted by [`inotify_init1`], and doesn't
    /// allow the user any choice in the matter, as not passing any of the flags
    /// would be inappropriate in the context of this wrapper:
    ///
    /// - [`IN_CLOEXEC`] prevents leaking file descriptors to other processes.
    /// - [`IN_NONBLOCK`] controls the blocking behavior of the inotify API,
    ///   which is entirely managed by this wrapper.
    ///
    /// # Errors
    ///
    /// Directly returns the error from the call to [`inotify_init1`], without
    /// adding any error conditions of its own.
    ///
    /// # Examples
    ///
    /// ```
    /// use inotify::Inotify;
    ///
    /// let inotify = Inotify::init()
    ///     .expect("Failed to initialize an inotify instance");
    /// ```
    ///
    /// [`Inotify`]: struct.Inotify.html
    /// [`inotify_init1`]: ../inotify_sys/fn.inotify_init1.html
    /// [`IN_CLOEXEC`]: ../inotify_sys/constant.IN_CLOEXEC.html
    /// [`IN_NONBLOCK`]: ../inotify_sys/constant.IN_NONBLOCK.html
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

    /// Watches the file at the given path
    ///
    /// Adds a watch for the file at the given path by calling
    /// [`inotify_add_watch`]. Returns a watch descriptor that can be used to
    /// refer to this watch later.
    ///
    /// The `mask` argument defines what kind of changes the file should be
    /// watched for, and how to do that. See the documentation of [`WatchMask`]
    /// for details.
    ///
    /// # Errors
    ///
    /// Directly returns the error from the call to [`inotify_add_watch`],
    /// without adding any error conditions of its own.
    ///
    /// # Examples
    ///
    /// ```
    /// use inotify::{
    ///     Inotify,
    ///     watch_mask,
    /// };
    ///
    /// let mut inotify = Inotify::init()
    ///     .expect("Failed to initialize an inotify instance");
    ///
    /// // Ignore any errors, as this is an example and the file we're trying to
    /// // watch here doesn't actually exist.
    /// let _ = inotify.add_watch("path/to/file", watch_mask::MODIFY);
    ///
    /// // Handle events for the file here
    /// ```
    ///
    /// [`inotify_add_watch`]: ../inotify_sys/fn.inotify_add_watch.html
    /// [`WatchMask`]: watch_mask/struct.WatchMask.html
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

    /// Stops watching a file
    ///
    /// Removes the watch represented by the provided [`WatchDescriptor`] by
    /// calling [`inotify_rm_watch`]. You can obtain a [`WatchDescriptor`] by
    /// saving one returned by [`Inotify::add_watch`] or from the `wd` field of
    /// [`Event`].
    ///
    /// # Errors
    ///
    /// Directly returns the error from the call to [`inotify_rm_watch`],
    /// without adding any error conditions of its own.
    ///
    /// # Examples
    ///
    /// ```
    /// use inotify::Inotify;
    ///
    /// let mut inotify = Inotify::init()
    ///     .expect("Failed to initialize an inotify instance");
    ///
    /// // Move the events into a buffer of our own. If we don't do this, we'll
    /// // have a mutable borrow on `inotify`, which prevents us from calling
    /// // `rm_watch` in the event handling loop below.
    /// let mut events = Vec::new();
    /// events.extend(
    ///     inotify
    ///         .available_events()
    ///         .expect("Error while waiting for events")
    /// );
    ///
    /// for event in events {
    ///     inotify.rm_watch(event.wd);
    /// }
    /// ```
    ///
    /// [`WatchDescriptor`]: struct.WatchDescriptor.html
    /// [`inotify_rm_watch`]: ../inotify_sys/fn.inotify_rm_watch.html
    /// [`Inotify::add_watch`]: struct.Inotify.html#method.add_watch
    /// [`Event`]: struct.Event.html
    pub fn rm_watch(&mut self, wd: WatchDescriptor) -> io::Result<()> {
        let result = unsafe { ffi::inotify_rm_watch(self.fd, wd.0) };
        match result {
            0  => Ok(()),
            -1 => Err(io::Error::last_os_error()),
            _  => panic!(
                "unexpected return code from inotify_rm_watch ({})", result)
        }
    }

    /// Waits until events are available, then returns them
    ///
    /// This method will block the current thread until at least one event is
    /// available. If this is not desirable, please take a look at
    /// [`available_events`].
    ///
    /// # Errors
    ///
    /// Directly returns the error from the call to [`read`], without adding any
    /// error conditions of its own.
    ///
    /// [`available_events`]: struct.Inotify.html#method.available_events
    /// [`read`]: ../libc/fn.read.html
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

    /// Returns any available events
    ///
    /// Returns an iterator over all events that are currently available. If no
    /// events are available, an iterator is still returned.
    ///
    /// If you need a method that will block until at least one event is
    /// available, please call [`wait_for_events`].
    ///
    /// # Errors
    ///
    /// Directly returns the error from the call to [`read`], without adding any
    /// error conditions of its own.
    ///
    /// # Examples
    ///
    /// ```
    /// use inotify::Inotify;
    ///
    /// let mut inotify = Inotify::init()
    ///     .expect("Failed to initialize an inotify instance");
    ///
    /// let events = inotify.available_events()
    ///     .expect("Error while reading events");
    ///
    /// for event in events {
    ///     // Handle event
    /// }
    /// ```
    ///
    /// [`wait_for_events`]: struct.Inotify.html#method.wait_for_events
    /// [`read`]: ../libc/fn.read.html
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

        let event_size = mem::size_of::<ffi::inotify_event>();

        let mut i = 0;
        while i < len {
            unsafe {
                let slice = &buffer[i as usize..];

                let event = slice.as_ptr() as *const ffi::inotify_event;

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

    /// Closes the inotify instance
    ///
    /// Closes the file descriptor referring to the inotify instance. The user
    /// usually doesn't have to call this function, as the underlying inotify
    /// instance is closed automatically, when [`Inotify`] is dropped.
    ///
    /// # Errors
    ///
    /// Directly returns the error from the call to [`close`], without adding any
    /// error conditions of its own.
    ///
    /// # Examples
    ///
    /// ```
    /// use inotify::Inotify;
    ///
    /// let mut inotify = Inotify::init()
    ///     .expect("Failed to initialize an inotify instance");
    ///
    /// inotify.close()
    ///     .expect("Failed to close inotify instance");
    /// ```
    ///
    /// [`Inotify`]: struct.Inotify.html
    /// [`close`]: ../libc/fn.close.html
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


/// Contains the [`WatchMask`] flags
///
/// Contains constants for all valid [`WatchMask`] flags, which can be used to
/// compare against a [`WatchMask`] instance using [`WatchMask::contains`].
///
/// [`WatchMask`]: struct.WatchMask.html
/// [`WatchMask::contains`]: struct.WatchMask.html#method.contains
pub mod watch_mask {
    use ffi;

    bitflags! {
        /// Mask for a file watch
        ///
        /// Passed to [`Inotify::add_watch`], to describe what file system
        /// events to watch for and how to do that.
        ///
        /// [`Inotify::add_watch`]: ../struct.Inotify.html#method.add_watch
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
/// Can be obtained from [`Inotify::add_watch`] or from an [`Event`]. A watch
/// descriptor can be used to get inotify to stop watching a file by passing it
/// to [`Inotify::rm_watch`].
///
/// [`Inotify::add_watch`]: struct.Inotify.html#method.add_watch
/// [`Inotify::rm_watch`]: struct.Inotify.html#method.rm_watch
/// [`Event`]: struct.Event.html
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WatchDescriptor(c_int);


/// Iterates over inotify events
///
/// Iterates over the events returned by [`Inotify::wait_for_events`] or
/// [`Inotify::available_events`].
///
/// [`Inotify::wait_for_events`]: struct.Inotify.html#method.wait_for_events
/// [`Inotify::available_events`]: struct.Inotify.html#method.available_events
pub struct Events<'a>(vec::Drain<'a, Event>);

impl<'a> Iterator for Events<'a> {
    type Item = Event;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }

}


/// An inotify event
///
/// A file system event that describes a change that the user previously
/// registered interest in. To watch for events, call [`Inotify::add_watch`]. To
/// retrieve events, call [`Inotify::wait_for_events`] or
/// [`Inotify::available_events`].
///
/// [`Inotify::add_watch`]: struct.Inotify.html#method.add_watch
/// [`Inotify::wait_for_events`]: struct.Inotify.html#method.wait_for_events
/// [`Inotify::available_events`]: struct.Inotify.html#method.available_events
#[derive(Clone, Debug)]
pub struct Event {
    /// Identifies the watch this event originates from
    ///
    /// This is the same [`WatchDescriptor`] that [`Inotify::add_watch`]
    /// returned when interest for this event was registered. The
    /// [`WatchDescriptor`] can be used to remove the watch using
    /// [`Inotify::rm_watch`], thereby preventing future events of this type
    /// from being created.
    ///
    /// [`WatchDescriptor`]: struct.WatchDescriptor.html
    /// [`Inotify::add_watch`]: struct.Inotify.html#method.add_watch
    /// [`Inotify::rm_watch`]: struct.Inotify.html#method.rm_watch
    pub wd    : WatchDescriptor,

    /// Shows what kind of event this is
    ///
    /// The various flags that can be set on this mask are defined in the
    /// [`event_mask`] module. You can check against any flags that are of
    /// interest to you by using [`EventMask::contains`].
    ///
    /// [`event_mask`]: event_mask/index.html
    /// [`EventMask::contains`]: event_mask/struct.EventMask.html#contains
    pub mask  : EventMask,

    /// Connects related events to each other
    ///
    /// When a file is renamed, this results two events: [`MOVED_FROM`] and
    /// [`MOVED_TO`]. The `cookie` field will be the same for both of them,
    /// thereby making is possible to connect the event pair.
    ///
    /// [`MOVED_FROM`]: event_mask/constant.MOVED_FROM.html
    /// [`MOVED_TO`]: event_mask/constant.MOVED_TO.html
    pub cookie: u32,

    /// The name of the file the event originates from
    pub name  : PathBuf,
}

impl Event {
    fn new(event: &ffi::inotify_event, name: PathBuf) -> Event {
        let mask = EventMask::from_bits(event.mask)
            .expect("Failed to convert event mask. This indicates a bug.");

        Event {
            wd    : WatchDescriptor(event.wd),
            mask  : mask,
            cookie: event.cookie,
            name  : name,
        }
    }
}


/// Contains the [`EventMask`] flags
///
/// Contains constants for all valid [`EventMask`] flags, which can be used to
/// compare against a [`EventMask`] instance using [`EventMask::contains`].
///
/// [`EventMask`]: struct.EventMask.html
/// [`EventMask::contains`]: struct.EventMask.html#method.contains
pub mod event_mask {
    use ffi;

    bitflags! {
        /// Mask for an event
        ///
        /// This struct can be retrieved from an [`Event`] via its `mask` field.
        /// You can determine the [`Event`]'s type by comparing it to the
        /// constants in [this module] module using [`EventMask::contains`].
        ///
        /// [`Event`]: ../struct.Event.html
        /// [this module]: index.html
        /// [`EventMask::contains`]: struct.EventMask.html#method.contains
        pub flags EventMask: u32 {
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

            /// Watch was removed.
            const IGNORED       = ffi::IN_IGNORED,

            /// Subject of this event is a directory.
            const ISDIR         = ffi::IN_ISDIR,

            /// Event queue overflowed.
            const Q_OVERFLOW    = ffi::IN_Q_OVERFLOW,

            /// File system containing watched object was unmounted.
            const UNMOUNT       = ffi::IN_UNMOUNT,
        }
    }
}

pub use self::event_mask::EventMask;
