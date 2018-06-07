#![crate_name = "inotify"]
#![crate_type = "lib"]
#![deny(missing_docs)]
#![deny(warnings)]

//! Idiomatic inotify wrapper for the Rust programming language
//!
//! # About
//!
//! [inotify-rs] is an idiomatic wrapper around the Linux kernel's [inotify] API
//! for the Rust programming language. It can be used for monitoring changes to
//! files or directories.
//!
//! The [`Inotify`] struct is the main entry point into the API.
//!
//! # Example
//!
//! ```
//! use inotify::{
//!     Inotify,
//!     WatchMask,
//! };
//!
//! let mut inotify = Inotify::init()
//!     .expect("Error while initializing inotify instance");
//!
//! # // Create a temporary file, so `add_watch` won't return an error.
//! # use std::fs::File;
//! # let mut test_file = File::create("/tmp/inotify-rs-test-file")
//! #     .expect("Failed to create test file");
//! #
//! // Watch for modify and close events.
//! inotify
//!     .add_watch(
//!         "/tmp/inotify-rs-test-file",
//!         WatchMask::MODIFY | WatchMask::CLOSE,
//!     )
//!     .expect("Failed to add file watch");
//!
//! # // Modify file, so the following `read_events_blocking` won't block.
//! # use std::io::Write;
//! # write!(&mut test_file, "something\n")
//! #     .expect("Failed to write something to test file");
//! #
//! // Read events that were added with `add_watch` above.
//! let mut buffer = [0; 1024];
//! let events = inotify.read_events_blocking(&mut buffer)
//!     .expect("Error while reading events");
//!
//! for event in events {
//!     // Handle event
//! }
//! ```
//!
//! # Attention: inotify gotchas
//!
//! inotify (as in, the Linux API, not this wrapper) has many edge cases, making
//! it hard to use correctly. This can lead to weird and hard to find bugs in
//! applications that are based on it. inotify-rs does its best to fix these
//! issues, but sometimes this would require an amount of runtime overhead that
//! is just unacceptable for a low-level wrapper such as this.
//!
//! We've documented any issues that inotify-rs has inherited from inotify, as
//! far as we are aware of them. Please watch out for any further warnings
//! throughout this documentation. If you want to be on the safe side, in case
//! we have missed something, please read the [inotify man pages] carefully.
//!
//! [inotify-rs]: https://crates.io/crates/inotify
//! [inotify]: https://en.wikipedia.org/wiki/Inotify
//! [`Inotify`]: struct.Inotify.html
//! [inotify man pages]: http://man7.org/linux/man-pages/man7/inotify.7.html


#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate futures;

extern crate libc;
extern crate inotify_sys as ffi;
extern crate mio;
extern crate tokio_io;
extern crate tokio_reactor;

use std::mem;
use std::hash::{
    Hash,
    Hasher,
};
use std::io;
use std::io::ErrorKind;
use std::ops::Deref;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{
    AsRawFd,
    FromRawFd,
    IntoRawFd,
    RawFd,
};
use std::path::Path;
use std::sync::{
    Arc,
    Weak,
    atomic::{
        AtomicBool,
        Ordering,
    },
};
use std::slice;
use std::ffi::{
    OsStr,
    OsString,
    CString,
};

use futures::{Async, Poll, Stream};
use libc::{
    FILENAME_MAX,
    F_GETFL,
    F_SETFL,
    O_NONBLOCK,
    fcntl,
    c_void,
    size_t,
    c_int,
};
use mio::{
    event::Evented,
    unix::EventedFd,
};
use tokio_reactor::{Handle, PollEvented};
use tokio_io::AsyncRead;


/// Idiomatic Rust wrapper around Linux's inotify API
///
/// `Inotify` is a wrapper around an inotify instance. It generally tries to
/// adhere to the underlying inotify API closely, while making access to it
/// safe and convenient.
///
/// Please refer to the [top-level documentation] for further details and a
/// usage example.
///
/// [top-level documentation]: index.html
pub struct Inotify {
    fd: Arc<FdGuard>,
}

impl Inotify {
    /// Creates an [`Inotify`] instance
    ///
    /// Initializes an inotify instance by calling [`inotify_init1`].
    ///
    /// This method passes both flags accepted by [`inotify_init1`], not giving
    /// the user any choice in the matter, as not passing the flags would be
    /// inappropriate in the context of this wrapper:
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
            // any authoritative sources on the topic. There's some discussion in
            // the open(2) and fcntl(2) man pages, but I didn't find that
            // helpful in understanding the issue of leaked file descriptors.
            // For what it's worth, there's a Rust issue about this:
            // https://github.com/rust-lang/rust/issues/12148
            ffi::inotify_init1(ffi::IN_CLOEXEC | ffi::IN_NONBLOCK)
        };

        match fd {
            -1 => Err(io::Error::last_os_error()),
            _  =>
                Ok(Inotify {
                    fd: Arc::new(FdGuard {
                        fd,
                        close_on_drop: AtomicBool::new(false),
                    }),
                }),
        }
    }

    /// Adds or updates a watch for the given path
    ///
    /// Adds a new watch or updates an existing one for the file referred to by
    /// `path`. Returns a watch descriptor that can be used to refer to this
    /// watch later.
    ///
    /// The `mask` argument defines what kind of changes the file should be
    /// watched for, and how to do that. See the documentation of [`WatchMask`]
    /// for details.
    ///
    /// If this method is used to add a new watch, a new [`WatchDescriptor`] is
    /// returned. If it is used to update an existing watch, a
    /// [`WatchDescriptor`] that equals the previously returned
    /// [`WatchDescriptor`] for that watch is returned instead.
    ///
    /// Under the hood, this method just calls [`inotify_add_watch`] and does
    /// some trivial translation between the types on the Rust side and the C
    /// side.
    ///
    /// # Attention: Updating watches and hardlinks
    ///
    /// As mentioned above, this method can be used to update an existing watch.
    /// This is usually done by calling this method with the same `path`
    /// argument that it has been called with before. But less obviously, it can
    /// also happen if the method is called with a different path that happens
    /// to link to the same inode.
    ///
    /// You can detect this by keeping track of [`WatchDescriptor`]s and the
    /// paths they have been returned for. If the same [`WatchDescriptor`] is
    /// returned for a different path (and you haven't freed the
    /// [`WatchDescriptor`] by removing the watch), you know you have two paths
    /// pointing to the same inode, being watched by the same watch.
    ///
    /// # Errors
    ///
    /// Directly returns the error from the call to
    /// [`inotify_add_watch`][`inotify_add_watch`] (translated into an
    /// `io::Error`), without adding any error conditions of
    /// its own.
    ///
    /// # Examples
    ///
    /// ```
    /// use inotify::{
    ///     Inotify,
    ///     WatchMask,
    /// };
    ///
    /// let mut inotify = Inotify::init()
    ///     .expect("Failed to initialize an inotify instance");
    ///
    /// # // Create a temporary file, so `add_watch` won't return an error.
    /// # use std::fs::File;
    /// # File::create("/tmp/inotify-rs-test-file")
    /// #     .expect("Failed to create test file");
    /// #
    /// inotify.add_watch("/tmp/inotify-rs-test-file", WatchMask::MODIFY)
    ///     .expect("Failed to add file watch");
    ///
    /// // Handle events for the file here
    /// ```
    ///
    /// [`inotify_add_watch`]: ../inotify_sys/fn.inotify_add_watch.html
    /// [`WatchMask`]: struct.WatchMask.html
    /// [`WatchDescriptor`]: struct.WatchDescriptor.html
    pub fn add_watch<P>(&mut self, path: P, mask: WatchMask)
        -> io::Result<WatchDescriptor>
        where P: AsRef<Path>
    {
        let path = CString::new(path.as_ref().as_os_str().as_bytes())?;

        let wd = unsafe {
            ffi::inotify_add_watch(
                **self.fd,
                path.as_ptr() as *const _,
                mask.bits(),
            )
        };

        match wd {
            -1 => Err(io::Error::last_os_error()),
            _  => Ok(WatchDescriptor{ id: wd, fd: Arc::downgrade(&self.fd) }),
        }
    }

    /// Stops watching a file
    ///
    /// Removes the watch represented by the provided [`WatchDescriptor`] by
    /// calling [`inotify_rm_watch`]. [`WatchDescriptor`]s can be obtained via
    /// [`Inotify::add_watch`], or from the `wd` field of [`Event`].
    ///
    /// # Errors
    ///
    /// Directly returns the error from the call to [`inotify_rm_watch`].
    /// Returns an [`io::Error`] with [`ErrorKind`]`::InvalidInput`, if the given
    /// [`WatchDescriptor`] did not originate from this [`Inotify`] instance.
    ///
    /// # Examples
    ///
    /// ```
    /// use inotify::Inotify;
    ///
    /// let mut inotify = Inotify::init()
    ///     .expect("Failed to initialize an inotify instance");
    ///
    /// # // Create a temporary file, so `add_watch` won't return an error.
    /// # use std::fs::File;
    /// # let mut test_file = File::create("/tmp/inotify-rs-test-file")
    /// #     .expect("Failed to create test file");
    /// #
    /// # // Add a watch and modify the file, so the code below doesn't block
    /// # // forever.
    /// # use inotify::WatchMask;
    /// # inotify.add_watch("/tmp/inotify-rs-test-file", WatchMask::MODIFY)
    /// #     .expect("Failed to add file watch");
    /// # use std::io::Write;
    /// # write!(&mut test_file, "something\n")
    /// #     .expect("Failed to write something to test file");
    /// #
    /// let mut buffer = [0; 1024];
    /// let events = inotify
    ///     .read_events_blocking(&mut buffer)
    ///     .expect("Error while waiting for events");
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
    /// [`Inotify`]: struct.Inotify.html
    /// [`io::Error`]: https://doc.rust-lang.org/std/io/struct.Error.html
    /// [`ErrorKind`]: https://doc.rust-lang.org/std/io/enum.ErrorKind.html
    pub fn rm_watch(&mut self, wd: WatchDescriptor) -> io::Result<()> {
        if wd.fd.upgrade().as_ref() != Some(&self.fd) {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "Invalid WatchDescriptor",
            ));
        }

        let result = unsafe { ffi::inotify_rm_watch(**self.fd, wd.id) };
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
    /// available. If this is not desirable, please consider [`read_events`].
    ///
    /// The documentation of [`read_events`] has additional about this call.
    ///
    /// # Errors
    ///
    /// Directly returns the error from the call to [`read`], without adding any
    /// error conditions of its own.
    ///
    /// [`read_events`]: struct.Inotify.html#method.read_events
    /// [`read`]: ../libc/fn.read.html
    pub fn read_events_blocking<'a>(&mut self, buffer: &'a mut [u8])
        -> io::Result<Events<'a>>
    {
        unsafe {
            fcntl(**self.fd, F_SETFL, fcntl(**self.fd, F_GETFL) & !O_NONBLOCK)
        };
        let result = self.read_events(buffer);
        unsafe {
            fcntl(**self.fd, F_SETFL, fcntl(**self.fd, F_GETFL) | O_NONBLOCK)
        };

        result
    }

    /// Returns any available events
    ///
    /// Returns an iterator over all events that are currently available. If no
    /// events are available, an iterator is still returned. If you need a
    /// method that will block until at least one event is available, please
    /// consider [`read_events_blocking`].
    ///
    /// Please note that inotify will merge identical unread events into a
    /// single event. This means this method can not be used to count the number
    /// of file system events.
    ///
    /// The `buffer` argument, as the name indicates, is used as a buffer for
    /// the inotify events. Its contents may be overwritten.
    ///
    /// # Errors
    ///
    /// This function directly returns all errors from the call to [`read`]. In
    /// addition, [`ErrorKind`]`::UnexpectedEof` is returned, if the call to
    /// [`read`] returns `0`, signaling end-of-file.
    ///
    /// If `buffer` is too small, this will result in an error with
    /// [`ErrorKind`]`::InvalidInput`. On very old Linux kernels,
    /// [`ErrorKind`]`::UnexpectedEof` will be returned instead.
    ///
    /// # Examples
    ///
    /// ```
    /// use inotify::Inotify;
    ///
    /// let mut inotify = Inotify::init()
    ///     .expect("Failed to initialize an inotify instance");
    ///
    /// let mut buffer = [0; 1024];
    /// let events = inotify.read_events(&mut buffer)
    ///     .expect("Error while reading events");
    ///
    /// for event in events {
    ///     // Handle event
    /// }
    /// ```
    ///
    /// [`read_events_blocking`]: struct.Inotify.html#method.read_events_blocking
    /// [`read`]: ../libc/fn.read.html
    /// [`ErrorKind`]: https://doc.rust-lang.org/std/io/enum.ErrorKind.html
    pub fn read_events<'a>(&mut self, buffer: &'a mut [u8])
        -> io::Result<Events<'a>>
    {
        let num_bytes = read_into_buffer(**self.fd, buffer);

        let num_bytes = match num_bytes {
            0 => {
                return Err(
                    io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "`read` return `0`, signaling end-of-file"
                    )
                );
            }
            -1 => {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::WouldBlock {
                    return Ok(Events::new(Arc::downgrade(&self.fd), buffer, 0));
                }
                else {
                    return Err(error);
                }
            },
            _ if num_bytes < 0 => {
                panic!("{} {} {} {} {} {}",
                    "Unexpected return value from `read`. Received a negative",
                    "value that was not `-1`. According to the `read` man page",
                    "this shouldn't happen, as either `-1` is returned on",
                    "error, `0` on end-of-file, or a positive value for the",
                    "number of bytes read. Returned value:",
                    num_bytes,
                );
            }
            _ => {
                // The value returned by `read` should be `isize`. Let's quickly
                // verify this with the following assignment, so we can be sure
                // our cast below is valid.
                let num_bytes: isize = num_bytes;

                // The type returned by `read` is `isize`, and we've ruled out
                // all negative values with the match arms above. This means we
                // can safely cast to `usize`.
                debug_assert!(num_bytes > 0);
                num_bytes as usize
            }
        };

        Ok(Events::new(Arc::downgrade(&self.fd), buffer, num_bytes))
    }

    /// Create a stream which collects events
    ///
    /// Returns a `Stream` over all events that are available. This stream is an
    /// infinite source of events.
    ///
    /// Note that this stream is not optimal and always reschedules itself if a
    /// read would block.
    ///
    /// An internal buffer which can hold the maximum possible size is used.
    ///
    /// The event stream will be associated with the default reactor.
    pub fn event_stream(&mut self) -> EventStream {
        EventStream::new(self.fd.clone())
    }

    /// Create a stream which collects events, associated with the given
    /// reactor.
    ///
    /// This functions identically to [`Inotify::event_stream`], except that
    /// the returned stream will be associated with the given reactor, rather
    /// than the default.
    ///
    /// [`Inotify::event_stream`]: struct.Inotify.html#method.event_stream
    pub fn event_stream_with_handle(&mut self, handle: &Handle)
                                    -> io::Result<EventStream>
    {
        EventStream::new_with_handle(self.fd.clone(), handle)
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
    pub fn close(self) -> io::Result<()> {
        // `self` will be dropped when this method returns. If this is the only
        // owner of `fd`, the `Arc` will also be dropped. The `Drop`
        // implementation for `FdGuard` will attempt to close the file descriptor
        // again, unless this flag here is cleared.
        self.fd.should_not_close();

        match unsafe { ffi::close(**self.fd) } {
            0 => Ok(()),
            _ => Err(io::Error::last_os_error()),
        }
    }
}

impl AsRawFd for Inotify {
    #[inline]
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl FromRawFd for Inotify {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Inotify {
            fd: Arc::new(FdGuard::from_raw_fd(fd))
        }
    }
}

impl IntoRawFd for Inotify {
    #[inline]
    fn into_raw_fd(self) -> RawFd {
        self.fd.should_not_close();
        self.fd.fd
    }
}

fn read_into_buffer(fd: RawFd, buffer: &mut [u8]) -> isize {
    unsafe {
        ffi::read(
            fd,
            buffer.as_mut_ptr() as *mut c_void,
            buffer.len() as size_t
        )
    }
}

/// A RAII guard around a `RawFd` that closes it automatically on drop.
#[derive(Debug)]
struct FdGuard {
    fd: RawFd,
    close_on_drop: AtomicBool,
}

impl FdGuard {

    /// Indicate that the wrapped file descriptor should _not_ be closed
    /// when the guard is dropped.
    ///
    /// This should be called in cases where ownership of the wrapped file
    /// descriptor has been "moved" out of the guard.
    ///
    /// This is factored out into a separate function to ensure that it's
    /// always used consistently.
    #[inline]
    fn should_not_close(&self) {
        self.close_on_drop.store(false, Ordering::Release);
    }
}

impl Deref for FdGuard {
    type Target = RawFd;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.fd
    }
}

impl Drop for FdGuard {
    fn drop(&mut self) {
        if self.close_on_drop.load(Ordering::Acquire) {
            unsafe { ffi::close(self.fd); }
        }
    }
}

impl FromRawFd for FdGuard {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        FdGuard {
            fd,
            close_on_drop: AtomicBool::new(true),
        }
    }
}

impl IntoRawFd for FdGuard {
    fn into_raw_fd(self) -> RawFd {
        self.should_not_close();
        self.fd
    }
}

impl AsRawFd for FdGuard {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}


impl PartialEq for FdGuard {
    fn eq(&self, other: &FdGuard) -> bool {
        self.fd == other.fd
    }
}

#[derive(Clone, Debug, PartialEq)]
struct EventedFdGuard(Arc<FdGuard>);

impl Evented for EventedFdGuard {
    #[inline]
    fn register(&self,
                poll: &mio::Poll,
                token: mio::Token,
                interest: mio::Ready,
                opts: mio::PollOpt)
                -> io::Result<()>
    {
        EventedFd(&(self.fd)).register(poll, token, interest, opts)
    }

    #[inline]
    fn reregister(&self,
                  poll: &mio::Poll,
                  token: mio::Token,
                  interest: mio::Ready,
                  opts: mio::PollOpt)
                  -> io::Result<()>
    {
        EventedFd(&(self.fd)).reregister(poll, token, interest, opts)
    }

    #[inline]
    fn deregister(&self, poll: &mio::Poll) -> io::Result<()> {
        EventedFd(&self.fd).deregister(poll)
    }
}

impl io::Read for EventedFdGuard {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match read_into_buffer(self.fd, buf) {
            i if i >= 0 => Ok(i as usize),
            _ => Err(io::Error::last_os_error()),
        }
    }
}

impl Deref for EventedFdGuard {
    type Target = Arc<FdGuard>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }

}

// Use this when 1.24 is available for use. We can use the hard-coded 16 due to Linux's ABI
// guarantees.
// const EVENT_MAX_SIZE: usize = mem::size_of::<ffi::inotify_event>() + (FILENAME_MAX as usize) + 1;
const EVENT_MAX_SIZE: usize = 16 + (FILENAME_MAX as usize) + 1;

/// Stream of inotify events
///
/// Allows for streaming events returned by [`Inotify::event_stream`].
///
/// [`Inotify::event_stream`]: struct.Inotify.html#method.event_stream
pub struct EventStream {
    fd: PollEvented<EventedFdGuard>,
    buffer: [u8; EVENT_MAX_SIZE],
    pos: usize,
    size: usize,
}

impl EventStream {
    /// Returns a new `EventStream` associated with the default reactor.
    fn new(fd: Arc<FdGuard>) -> Self {
        EventStream {
            fd: PollEvented::new(EventedFdGuard(fd)),
            buffer: [0; EVENT_MAX_SIZE],
            pos: 0,
            size: 0,
        }
    }

    /// Returns a new `EventStream` associated with the specified reactor.
    fn new_with_handle(fd: Arc<FdGuard>, handle: &Handle) -> io::Result<Self> {
        Ok(EventStream {
            fd: PollEvented::new_with_handle(EventedFdGuard(fd), handle)?,
            buffer: [0; EVENT_MAX_SIZE],
            pos: 0,
            size: 0,
        })
    }
}

impl Stream for EventStream {
    type Item = EventOwned;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error>
    {
        if 0 < self.size {
            let (step, event) = Event::from_buffer(
                Arc::downgrade(self.fd.get_ref()),
                &self.buffer[self.pos..],
                self.size,
            );
            self.pos += step;
            self.size -= step;

            return Ok(Async::Ready(Some(event.into_owned())));
        }

        let num_bytes = try_ready!(self.fd.poll_read(&mut self.buffer)) as usize;

        if num_bytes == 0 {
            return Ok(Async::Ready(None));
        }

        let (step, event) = Event::from_buffer(
            Arc::downgrade(self.fd.get_ref()),
            &self.buffer,
            num_bytes,
        );
        self.pos = step;
        self.size = num_bytes - step;

        Ok(Async::Ready(Some(event.into_owned())))
    }
}

bitflags! {
    /// Describes a file system watch
    ///
    /// Passed to [`Inotify::add_watch`], to describe what file system events
    /// to watch for, and how to do that.
    ///
    /// # Examples
    ///
    /// `WatchMask` constants can be passed to [`Inotify::add_watch`] as is. For
    /// example, here's how to create a watch that triggers an event when a file
    /// is accessed:
    ///
    /// ``` rust
    /// # use inotify::{
    /// #     Inotify,
    /// #     WatchMask,
    /// # };
    /// #
    /// # let mut inotify = Inotify::init().unwrap();
    /// #
    /// # // Create a temporary file, so `add_watch` won't return an error.
    /// # use std::fs::File;
    /// # File::create("/tmp/inotify-rs-test-file")
    /// #     .expect("Failed to create test file");
    /// #
    /// inotify.add_watch("/tmp/inotify-rs-test-file", WatchMask::ACCESS)
    ///    .expect("Error adding watch");
    /// ```
    ///
    /// You can also combine multiple `WatchMask` constants. Here we add a watch
    /// this is triggered both when files are created or deleted in a directory:
    ///
    /// ``` rust
    /// # use inotify::{
    /// #     Inotify,
    /// #     WatchMask,
    /// # };
    /// #
    /// # let mut inotify = Inotify::init().unwrap();
    /// inotify.add_watch("/tmp/", WatchMask::CREATE | WatchMask::DELETE)
    ///    .expect("Error adding watch");
    /// ```
    ///
    /// [`Inotify::add_watch`]: struct.Inotify.html#method.add_watch
    pub struct WatchMask: u32 {
        /// File was accessed
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_ACCESS`].
        ///
        /// [`inotify_sys::IN_ACCESS`]: ../inotify_sys/constant.IN_ACCESS.html
        const ACCESS = ffi::IN_ACCESS;

        /// Metadata (permissions, timestamps, ...) changed
        ///
        /// When watching a directory, this event can be triggered for the
        /// directory itself, as well as objects inside the directory.
        ///
        /// See [`inotify_sys::IN_ATTRIB`].
        ///
        /// [`inotify_sys::IN_ATTRIB`]: ../inotify_sys/constant.IN_ATTRIB.html
        const ATTRIB = ffi::IN_ATTRIB;

        /// File opened for writing was closed
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_CLOSE_WRITE`].
        ///
        /// [`inotify_sys::IN_CLOSE_WRITE`]: ../inotify_sys/constant.IN_CLOSE_WRITE.html
        const CLOSE_WRITE = ffi::IN_CLOSE_WRITE;

        /// File or directory not opened for writing was closed
        ///
        /// When watching a directory, this event can be triggered for the
        /// directory itself, as well as objects inside the directory.
        ///
        /// See [`inotify_sys::IN_CLOSE_NOWRITE`].
        ///
        /// [`inotify_sys::IN_CLOSE_NOWRITE`]: ../inotify_sys/constant.IN_CLOSE_NOWRITE.html
        const CLOSE_NOWRITE = ffi::IN_CLOSE_NOWRITE;

        /// File/directory created in watched directory
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_CREATE`].
        ///
        /// [`inotify_sys::IN_CREATE`]: ../inotify_sys/constant.IN_CREATE.html
        const CREATE = ffi::IN_CREATE;

        /// File/directory deleted from watched directory
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_DELETE`].
        ///
        /// [`inotify_sys::IN_DELETE`]: ../inotify_sys/constant.IN_DELETE.html
        const DELETE = ffi::IN_DELETE;

        /// Watched file/directory was deleted
        ///
        /// See [`inotify_sys::IN_DELETE_SELF`].
        ///
        /// [`inotify_sys::IN_DELETE_SELF`]: ../inotify_sys/constant.IN_DELETE_SELF.html
        const DELETE_SELF = ffi::IN_DELETE_SELF;

        /// File was modified
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_MODIFY`].
        ///
        /// [`inotify_sys::IN_MODIFY`]: ../inotify_sys/constant.IN_MODIFY.html
        const MODIFY = ffi::IN_MODIFY;

        /// Watched file/directory was moved
        ///
        /// See [`inotify_sys::IN_MOVE_SELF`].
        ///
        /// [`inotify_sys::IN_MOVE_SELF`]: ../inotify_sys/constant.IN_MOVE_SELF.html
        const MOVE_SELF = ffi::IN_MOVE_SELF;

        /// File was renamed/moved; watched directory contained old name
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_MOVED_FROM`].
        ///
        /// [`inotify_sys::IN_MOVED_FROM`]: ../inotify_sys/constant.IN_MOVED_FROM.html
        const MOVED_FROM = ffi::IN_MOVED_FROM;

        /// File was renamed/moved; watched directory contains new name
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_MOVED_TO`].
        ///
        /// [`inotify_sys::IN_MOVED_TO`]: ../inotify_sys/constant.IN_MOVED_TO.html
        const MOVED_TO = ffi::IN_MOVED_TO;

        /// File or directory was opened
        ///
        /// When watching a directory, this event can be triggered for the
        /// directory itself, as well as objects inside the directory.
        ///
        /// See [`inotify_sys::IN_OPEN`].
        ///
        /// [`inotify_sys::IN_OPEN`]: ../inotify_sys/constant.IN_OPEN.html
        const OPEN = ffi::IN_OPEN;

        /// Watch for all events
        ///
        /// This constant is simply a convenient combination of the following
        /// other constants:
        ///
        /// - [`ACCESS`]
        /// - [`ATTRIB`]
        /// - [`CLOSE_WRITE`]
        /// - [`CLOSE_NOWRITE`]
        /// - [`CREATE`]
        /// - [`DELETE`]
        /// - [`DELETE_SELF`]
        /// - [`MODIFY`]
        /// - [`MOVE_SELF`]
        /// - [`MOVED_FROM`]
        /// - [`MOVED_TO`]
        /// - [`OPEN`]
        ///
        /// See [`inotify_sys::IN_ALL_EVENTS`].
        ///
        /// [`ACCESS`]: #associatedconstant.ACCESS
        /// [`ATTRIB`]: #associatedconstant.ATTRIB
        /// [`CLOSE_WRITE`]: #associatedconstant.CLOSE_WRITE
        /// [`CLOSE_NOWRITE`]: #associatedconstant.CLOSE_NOWRITE
        /// [`CREATE`]: #associatedconstant.CREATE
        /// [`DELETE`]: #associatedconstant.DELETE
        /// [`DELETE_SELF`]: #associatedconstant.DELETE_SELF
        /// [`MODIFY`]: #associatedconstant.MODIFY
        /// [`MOVE_SELF`]: #associatedconstant.MOVE_SELF
        /// [`MOVED_FROM`]: #associatedconstant.MOVED_FROM
        /// [`MOVED_TO`]: #associatedconstant.MOVED_TO
        /// [`OPEN`]: #associatedconstant.OPEN
        /// [`inotify_sys::IN_ALL_EVENTS`]: ../inotify_sys/constant.IN_ALL_EVENTS.html
        const ALL_EVENTS = ffi::IN_ALL_EVENTS;

        /// Watch for all move events
        ///
        /// This constant is simply a convenient combination of the following
        /// other constants:
        ///
        /// - [`MOVED_FROM`]
        /// - [`MOVED_TO`]
        ///
        /// See [`inotify_sys::IN_MOVE`].
        ///
        /// [`MOVED_FROM`]: #associatedconstant.MOVED_FROM
        /// [`MOVED_TO`]: #associatedconstant.MOVED_TO
        /// [`inotify_sys::IN_MOVE`]: ../inotify_sys/constant.IN_MOVE.html
        const MOVE = ffi::IN_MOVE;

        /// Watch for all close events
        ///
        /// This constant is simply a convenient combination of the following
        /// other constants:
        ///
        /// - [`CLOSE_WRITE`]
        /// - [`CLOSE_NOWRITE`]
        ///
        /// See [`inotify_sys::IN_CLOSE`].
        ///
        /// [`CLOSE_WRITE`]: #associatedconstant.CLOSE_WRITE
        /// [`CLOSE_NOWRITE`]: #associatedconstant.CLOSE_NOWRITE
        /// [`inotify_sys::IN_CLOSE`]: ../inotify_sys/constant.IN_CLOSE.html
        const CLOSE = ffi::IN_CLOSE;

        /// Don't dereference the path if it is a symbolic link
        ///
        /// See [`inotify_sys::IN_DONT_FOLLOW`].
        ///
        /// [`inotify_sys::IN_DONT_FOLLOW`]: ../inotify_sys/constant.IN_DONT_FOLLOW.html
        const DONT_FOLLOW = ffi::IN_DONT_FOLLOW;

        /// Filter events for directory entries that have been unlinked
        ///
        /// See [`inotify_sys::IN_EXCL_UNLINK`].
        ///
        /// [`inotify_sys::IN_EXCL_UNLINK`]: ../inotify_sys/constant.IN_EXCL_UNLINK.html
        const EXCL_UNLINK = ffi::IN_EXCL_UNLINK;

        /// If a watch for the inode exists, amend it instead of replacing it
        ///
        /// See [`inotify_sys::IN_MASK_ADD`].
        ///
        /// [`inotify_sys::IN_MASK_ADD`]: ../inotify_sys/constant.IN_MASK_ADD.html
        const MASK_ADD = ffi::IN_MASK_ADD;

        /// Only receive one event, then remove the watch
        ///
        /// See [`inotify_sys::IN_ONESHOT`].
        ///
        /// [`inotify_sys::IN_ONESHOT`]: ../inotify_sys/constant.IN_ONESHOT.html
        const ONESHOT = ffi::IN_ONESHOT;

        /// Only watch path, if it is a directory
        ///
        /// See [`inotify_sys::IN_ONLYDIR`].
        ///
        /// [`inotify_sys::IN_ONLYDIR`]: ../inotify_sys/constant.IN_ONLYDIR.html
        const ONLYDIR = ffi::IN_ONLYDIR;
    }
}


/// Represents a watch on an inode
///
/// Can be obtained from [`Inotify::add_watch`] or from an [`Event`]. A watch
/// descriptor can be used to get inotify to stop watching an inode by passing
/// it to [`Inotify::rm_watch`].
///
/// [`Inotify::add_watch`]: struct.Inotify.html#method.add_watch
/// [`Inotify::rm_watch`]: struct.Inotify.html#method.rm_watch
/// [`Event`]: struct.Event.html
#[derive(Clone, Debug)]
pub struct WatchDescriptor{
    id: c_int,
    fd: Weak<FdGuard>,
}

impl Eq for WatchDescriptor {}

impl PartialEq for WatchDescriptor {
    fn eq(&self, other: &Self) -> bool {
        let self_fd  = self.fd.upgrade();
        let other_fd = other.fd.upgrade();

        self.id == other.id && self_fd.is_some() && self_fd == other_fd
    }
}

impl Hash for WatchDescriptor {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // This function only takes `self.id` into account, as `self.fd` is a
        // weak pointer that might no longer be available. Since neither
        // panicking nor changing the hash depending on whether it's available
        // is acceptable, we just don't look at it at all.
        // I don't think that this influences storage in a `HashMap` or
        // `HashSet` negatively, as storing `WatchDescriptor`s from different
        // `Inotify` instances seems like something of an anti-pattern anyway.
        self.id.hash(state);
    }
}


/// Iterator over inotify events
///
/// Allows for iteration over the events returned by
/// [`Inotify::read_events_blocking`] or [`Inotify::read_events`].
///
/// [`Inotify::read_events_blocking`]: struct.Inotify.html#method.read_events_blocking
/// [`Inotify::read_events`]: struct.Inotify.html#method.read_events
pub struct Events<'a> {
    fd       : Weak<FdGuard>,
    buffer   : &'a [u8],
    num_bytes: usize,
    pos      : usize,
}

impl<'a> Events<'a> {
    fn new(fd: Weak<FdGuard>, buffer: &'a [u8], num_bytes: usize) -> Self {
        Events {
            fd       : fd,
            buffer   : buffer,
            num_bytes: num_bytes,
            pos      : 0,
        }
    }
}

impl<'a> Iterator for Events<'a> {
    type Item = Event<&'a OsStr>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos < self.num_bytes {
            let (step, event) = Event::from_buffer(self.fd.clone(), &self.buffer[self.pos..], self.num_bytes - self.pos);
            self.pos += step;

            Some(event)
        }
        else {
            None
        }
    }
}


/// An inotify event
///
/// A file system event that describes a change that the user previously
/// registered interest in. To watch for events, call [`Inotify::add_watch`]. To
/// retrieve events, call [`Inotify::read_events_blocking`] or
/// [`Inotify::read_events`].
///
/// [`Inotify::add_watch`]: struct.Inotify.html#method.add_watch
/// [`Inotify::read_events_blocking`]: struct.Inotify.html#method.read_events_blocking
/// [`Inotify::read_events`]: struct.Inotify.html#method.read_events
#[derive(Clone, Debug)]
pub struct Event<S> {
    /// Identifies the watch this event originates from
    ///
    /// This [`WatchDescriptor`] is equal to the one that [`Inotify::add_watch`]
    /// returned when interest for this event was registered. The
    /// [`WatchDescriptor`] can be used to remove the watch using
    /// [`Inotify::rm_watch`], thereby preventing future events of this type
    /// from being created.
    ///
    /// [`WatchDescriptor`]: struct.WatchDescriptor.html
    /// [`Inotify::add_watch`]: struct.Inotify.html#method.add_watch
    /// [`Inotify::rm_watch`]: struct.Inotify.html#method.rm_watch
    pub wd: WatchDescriptor,

    /// Indicates what kind of event this is
    pub mask: EventMask,

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
    ///
    /// This field is set only if the subject of the event is a file in a
    /// watched directory. If the event concerns a file or directory that is
    /// watched directly, `name` will be `None`.
    pub name: Option<S>,
}

impl<'a> Event<&'a OsStr> {
    fn new(fd: Weak<FdGuard>, event: &ffi::inotify_event, name: &'a OsStr)
        -> Self
    {
        let mask = EventMask::from_bits(event.mask)
            .expect("Failed to convert event mask. This indicates a bug.");

        let wd = ::WatchDescriptor {
            id: event.wd,
            fd,
        };

        let name = if name == "" {
            None
        }
        else {
            Some(name)
        };

        Event {
            wd,
            mask,
            cookie: event.cookie,
            name,
        }
    }

    fn from_buffer(fd: Weak<FdGuard>, buffer: &'a [u8], num_bytes: usize) -> (usize, Self) {
        let event_size = mem::size_of::<ffi::inotify_event>();

        // `self.buffer` contains the data that was read from the inotify
        // instance. `self.num_bytes` is the number of bytes that were read.
        // And as per the if condition above, `self.pos < self.num_bytes`,
        // so our current position is still within the bounds of the buffer.
        // This means, unless inotify lied to us, or we did something
        // horribly wrong, there should be at least another event worth of
        // bytes in the buffer.
        debug_assert!(num_bytes >= event_size);

        let slice = &buffer[..];
        let event = slice.as_ptr() as *const ffi::inotify_event;

        // We have a pointer to an `inotify_event` that points into the
        // buffer at offset `self.pos`. Since we know, as per the assertion
        // above, that there are enough bytes for at least one more event in
        // the buffer, dereferencing that pointer is safe.
        let event = unsafe { *event };

        // The call to `offset` is safe, as long as the starting and the
        // resulting pointer are either in bounds or one byte past the end
        // of an allocated object. As we've established above, there are
        // enough bytes for the `inotify_event` left in the buffer. If there
        // is anything else in the buffer, the new pointer will be within an
        // allocated object. If there is nothing else, it will be exactly
        // one byte past it.
        let name = unsafe {
            slice
                .as_ptr()
                .offset(event_size as isize)
        };

        // Right behind the `inotify_event` struct is the event's name. The
        // name's length is given by `event.len`. There should always be
        // enough bytes left in the buffer to fit the name.
        let name_pos = event_size;
        debug_assert!(num_bytes - name_pos >= event.len as usize);

        // As we've established above, the name fits within the buffer. This
        // means that there's either an actual name in there, with enough
        // bytes to make the created slice valid, or `event.len` is `0`, in
        // which case the function call is safe in any case, as long as
        // `name` is not null. We know it's not, because we created it from
        // a slice right above.
        let name = unsafe {
            slice::from_raw_parts(
                name,
                event.len as usize,
            )
        };

        // Remove trailing \0 bytes
        //
        // The events in the buffer are aligned, and `name` is filled up
        // with '\0' up to the alignment boundary. Here we remove those
        // additional bytes.
        //
        // The `unwrap` here is safe, because `splitn` always returns at
        // least one result, even if the original slice contains no '\0'.
        let name = name
            .splitn(2, |b| b == &0u8)
            .next()
            .unwrap();

        let step = event_size + event.len as usize;
        let event = Event::new(
            fd,
            &event,
            OsStr::from_bytes(name),
        );

        (step, event)
    }

    fn into_owned(&self) -> EventOwned {
        Event {
            wd: self.wd.clone(),
            mask: self.mask,
            cookie: self.cookie,
            name: self.name.map(OsStr::to_os_string),
        }
    }
}

/// An owned version of `Event`
pub type EventOwned = Event<OsString>;


bitflags! {
    /// Indicates the type of an event
    ///
    /// This struct can be retrieved from an [`Event`] via its `mask` field.
    /// You can determine the [`Event`]'s type by comparing the `EventMask` to
    /// its associated constants.
    ///
    /// Please refer to the documentation of [`Event`] for a usage example.
    ///
    /// [`Event`]: struct.Event.html
    pub struct EventMask: u32 {
        /// File was accessed
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_ACCESS`].
        ///
        /// [`inotify_sys::IN_ACCESS`]: ../inotify_sys/constant.IN_ACCESS.html
        const ACCESS = ffi::IN_ACCESS;

        /// Metadata (permissions, timestamps, ...) changed
        ///
        /// When watching a directory, this event can be triggered for the
        /// directory itself, as well as objects inside the directory.
        ///
        /// See [`inotify_sys::IN_ATTRIB`].
        ///
        /// [`inotify_sys::IN_ATTRIB`]: ../inotify_sys/constant.IN_ATTRIB.html
        const ATTRIB = ffi::IN_ATTRIB;

        /// File opened for writing was closed
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_CLOSE_WRITE`].
        ///
        /// [`inotify_sys::IN_CLOSE_WRITE`]: ../inotify_sys/constant.IN_CLOSE_WRITE.html
        const CLOSE_WRITE = ffi::IN_CLOSE_WRITE;

        /// File or directory not opened for writing was closed
        ///
        /// When watching a directory, this event can be triggered for the
        /// directory itself, as well as objects inside the directory.
        ///
        /// See [`inotify_sys::IN_CLOSE_NOWRITE`].
        ///
        /// [`inotify_sys::IN_CLOSE_NOWRITE`]: ../inotify_sys/constant.IN_CLOSE_NOWRITE.html
        const CLOSE_NOWRITE = ffi::IN_CLOSE_NOWRITE;

        /// File/directory created in watched directory
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_CREATE`].
        ///
        /// [`inotify_sys::IN_CREATE`]: ../inotify_sys/constant.IN_CREATE.html
        const CREATE = ffi::IN_CREATE;

        /// File/directory deleted from watched directory
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_DELETE`].
        ///
        /// [`inotify_sys::IN_DELETE`]: ../inotify_sys/constant.IN_DELETE.html
        const DELETE = ffi::IN_DELETE;

        /// Watched file/directory was deleted
        ///
        /// See [`inotify_sys::IN_DELETE_SELF`].
        ///
        /// [`inotify_sys::IN_DELETE_SELF`]: ../inotify_sys/constant.IN_DELETE_SELF.html
        const DELETE_SELF = ffi::IN_DELETE_SELF;

        /// File was modified
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_MODIFY`].
        ///
        /// [`inotify_sys::IN_MODIFY`]: ../inotify_sys/constant.IN_MODIFY.html
        const MODIFY = ffi::IN_MODIFY;

        /// Watched file/directory was moved
        ///
        /// See [`inotify_sys::IN_MOVE_SELF`].
        ///
        /// [`inotify_sys::IN_MOVE_SELF`]: ../inotify_sys/constant.IN_MOVE_SELF.html
        const MOVE_SELF = ffi::IN_MOVE_SELF;

        /// File was renamed/moved; watched directory contained old name
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_MOVED_FROM`].
        ///
        /// [`inotify_sys::IN_MOVED_FROM`]: ../inotify_sys/constant.IN_MOVED_FROM.html
        const MOVED_FROM = ffi::IN_MOVED_FROM;

        /// File was renamed/moved; watched directory contains new name
        ///
        /// When watching a directory, this event is only triggered for objects
        /// inside the directory, not the directory itself.
        ///
        /// See [`inotify_sys::IN_MOVED_TO`].
        ///
        /// [`inotify_sys::IN_MOVED_TO`]: ../inotify_sys/constant.IN_MOVED_TO.html
        const MOVED_TO = ffi::IN_MOVED_TO;

        /// File or directory was opened
        ///
        /// When watching a directory, this event can be triggered for the
        /// directory itself, as well as objects inside the directory.
        ///
        /// See [`inotify_sys::IN_OPEN`].
        ///
        /// [`inotify_sys::IN_OPEN`]: ../inotify_sys/constant.IN_OPEN.html
        const OPEN = ffi::IN_OPEN;

        /// Watch was removed
        ///
        /// This event will be generated, if the watch was removed explicitly
        /// (via [`Inotify::rm_watch`]), or automatically (because the file was
        /// deleted or the file system was unmounted).
        ///
        /// See [`inotify_sys::IN_IGNORED`].
        ///
        /// [`inotify_sys::IN_IGNORED`]: ../inotify_sys/constant.IN_IGNORED.html
        const IGNORED = ffi::IN_IGNORED;

        /// Event related to a directory
        ///
        /// The subject of the event is a directory.
        ///
        /// See [`inotify_sys::IN_ISDIR`].
        ///
        /// [`inotify_sys::IN_ISDIR`]: ../inotify_sys/constant.IN_ISDIR.html
        const ISDIR = ffi::IN_ISDIR;

        /// Event queue overflowed
        ///
        /// The event queue has overflowed and events have presumably been lost.
        ///
        /// See [`inotify_sys::IN_Q_OVERFLOW`].
        ///
        /// [`inotify_sys::IN_Q_OVERFLOW`]: ../inotify_sys/constant.IN_Q_OVERFLOW.html
        const Q_OVERFLOW = ffi::IN_Q_OVERFLOW;

        /// File system containing watched object was unmounted.
        /// File system was unmounted
        ///
        /// The file system that contained the watched object has been
        /// unmounted. An event with [`WatchMask::IGNORED`] will subsequently be
        /// generated for the same watch descriptor.
        ///
        /// See [`inotify_sys::IN_UNMOUNT`].
        ///
        /// [`WatchMask::IGNORED`]: #associatedconstant.IGNORED
        /// [`inotify_sys::IN_UNMOUNT`]: ../inotify_sys/constant.IN_UNMOUNT.html
        const UNMOUNT = ffi::IN_UNMOUNT;
    }
}
