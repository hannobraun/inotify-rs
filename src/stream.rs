use std::{
    io,
    os::unix::io::AsRawFd,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures_core::{ready, Stream};
use tokio::io::unix::AsyncFd;

use crate::events::{Event, EventOwned};
use crate::fd_guard::FdGuard;
use crate::util::read_into_buffer;
use crate::watches::Watches;
use crate::Inotify;

/// Stream of inotify events
///
/// Allows for streaming events returned by [`Inotify::into_event_stream`].
#[derive(Debug)]
pub struct EventStream<T> {
    fd: AsyncFd<Arc<FdGuard>>,
    buffer: T,
    buffer_pos: usize,
    unused_bytes: usize,
}

impl<T> EventStream<T>
where
    T: AsMut<[u8]> + AsRef<[u8]>,
{
    /// Returns a new `EventStream` associated with the default reactor.
    pub(crate) fn new(fd: Arc<FdGuard>, buffer: T) -> io::Result<Self> {
        Ok(EventStream {
            fd: AsyncFd::new(fd)?,
            buffer,
            buffer_pos: 0,
            unused_bytes: 0,
        })
    }

    /// Returns an instance of `Watches` to add and remove watches.
    /// See [`Watches::add`] and [`Watches::remove`].
    pub fn watches(&self) -> Watches {
        Watches::new(self.fd.get_ref().clone())
    }

    /// Reads and returns a single available event
    ///
    /// Returns the next available event from the internal buffer. If the buffer
    /// is empty, it will attempt to read more events from the file descriptor.
    /// Returns `Ok(None)` if no events are available without blocking.
    ///
    /// Please note that inotify will merge identical successive unread events
    /// into a single event. This means this method can not be used to count the
    /// number of file system events.
    ///
    /// # Errors
    ///
    /// This function directly returns all errors from the call to [`read`],
    /// except for [`ErrorKind::WouldBlock`] which results in `Ok(None)`.
    /// [`ErrorKind::UnexpectedEof`] is returned if the call to [`read`]
    /// returns `0`, signaling end-of-file.
    ///
    /// [`read`]: libc::read
    /// [`ErrorKind::WouldBlock`]: std::io::ErrorKind::WouldBlock
    /// [`ErrorKind::UnexpectedEof`]: std::io::ErrorKind::UnexpectedEof
    pub fn read_events(&mut self) -> io::Result<Option<EventOwned>> {
        if self.unused_bytes == 0 {
            // Nothing usable in buffer. Need to reset and fill buffer.
            self.buffer_pos = 0;
            let num_bytes = read_into_buffer(self.fd.as_raw_fd(), self.buffer.as_mut());

            self.unused_bytes = match num_bytes {
                0 => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "`read` return `0`, signaling end-of-file",
                    ));
                }
                -1 => {
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::WouldBlock {
                        return Ok(None);
                    }
                    return Err(error);
                }
                _ if num_bytes < 0 => {
                    panic!(
                        "{} {} {} {} {} {}",
                        "Unexpected return value from `read`. Received a negative",
                        "value that was not `-1`. According to the `read` man page",
                        "this shouldn't happen, as either `-1` is returned on",
                        "error, `0` on end-of-file, or a positive value for the",
                        "number of bytes read. Returned value:",
                        num_bytes,
                    );
                }
                _ => num_bytes as usize,
            };
        }

        // We have bytes in the buffer. Extract one event.
        let (bytes_consumed, event) = Event::from_buffer(
            Arc::downgrade(self.fd.get_ref()),
            &self.buffer.as_ref()[self.buffer_pos..],
        );
        self.buffer_pos += bytes_consumed;
        self.unused_bytes -= bytes_consumed;

        Ok(Some(event.to_owned()))
    }

    /// Consumes the `EventStream` instance and returns an `Inotify` using the original
    /// file descriptor that was passed from `Inotify` to create the `EventStream`.
    pub fn into_inotify(self) -> Inotify {
        Inotify::from_file_descriptor(self.fd.into_inner())
    }
}

impl<T> Stream for EventStream<T>
where
    T: AsMut<[u8]> + AsRef<[u8]>,
{
    type Item = io::Result<EventOwned>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Safety: safe because we never move out of `self_`.
        let self_ = unsafe { self.get_unchecked_mut() };

        if self_.unused_bytes == 0 {
            // Nothing usable in buffer. Need to reset and fill buffer.
            self_.buffer_pos = 0;
            self_.unused_bytes = ready!(read(&self_.fd, self_.buffer.as_mut(), cx))?;
        }

        if self_.unused_bytes == 0 {
            // The previous read returned `0` signalling end-of-file. Let's
            // signal end-of-stream to the caller.
            return Poll::Ready(None);
        }

        // We have bytes in the buffer. inotify doesn't put partial events in
        // there, and we only take complete events out. That means we have at
        // least one event in there and can call `from_buffer` to take it out.
        let (bytes_consumed, event) = Event::from_buffer(
            Arc::downgrade(self_.fd.get_ref()),
            &self_.buffer.as_ref()[self_.buffer_pos..],
        );
        self_.buffer_pos += bytes_consumed;
        self_.unused_bytes -= bytes_consumed;

        Poll::Ready(Some(Ok(event.to_owned())))
    }
}

fn read(
    fd: &AsyncFd<Arc<FdGuard>>,
    buffer: &mut [u8],
    cx: &mut Context,
) -> Poll<io::Result<usize>> {
    let mut guard = ready!(fd.poll_read_ready(cx))?;
    let result = guard.try_io(|_| {
        let read = read_into_buffer(fd.as_raw_fd(), buffer);
        if read == -1 {
            return Err(io::Error::last_os_error());
        }

        Ok(read as usize)
    });

    match result {
        Ok(result) => Poll::Ready(result),
        Err(_would_block) => {
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}
