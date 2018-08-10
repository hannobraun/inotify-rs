extern crate mio;
extern crate tokio_io;


use std::{
    io,
    ops::Deref,
    sync::Arc,
};

use self::{
    mio::{
        event::Evented,
        unix::EventedFd,
    },
    tokio_io::AsyncRead,
};
use futures::{
    Async,
    Poll,
    Stream,
};
use tokio_reactor::{
    Handle,
    PollEvented,
};

use events::{
    Event,
    EventOwned,
};
use fd_guard::FdGuard;
use util::read_into_buffer;


/// Stream of inotify events
///
/// Allows for streaming events returned by [`Inotify::event_stream`].
///
/// [`Inotify::event_stream`]: struct.Inotify.html#method.event_stream
pub struct EventStream<'buffer> {
    fd: PollEvented<EventedFdGuard>,
    buffer: &'buffer mut [u8],
    pos: usize,
    size: usize,
}

impl<'buffer> EventStream<'buffer> {
    /// Returns a new `EventStream` associated with the default reactor.
    pub(crate) fn new(fd: Arc<FdGuard>, buffer: &'buffer mut [u8]) -> Self {
        EventStream {
            fd: PollEvented::new(EventedFdGuard(fd)),
            buffer: buffer,
            pos: 0,
            size: 0,
        }
    }

    /// Returns a new `EventStream` associated with the specified reactor.
     pub(crate) fn new_with_handle(
        fd    : Arc<FdGuard>,
        handle: &Handle,
        buffer: &'buffer mut [u8],
    )
        -> io::Result<Self>
    {
        Ok(EventStream {
            fd: PollEvented::new_with_handle(EventedFdGuard(fd), handle)?,
            buffer: buffer,
            pos: 0,
            size: 0,
        })
    }
}

impl<'buffer> Stream for EventStream<'buffer> {
    type Item = EventOwned;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error>
    {
        if 0 < self.size {
            let (step, event) = Event::from_buffer(
                Arc::downgrade(self.fd.get_ref()),
                &self.buffer[self.pos..],
            );
            self.pos += step;
            self.size -= step;

            return Ok(Async::Ready(Some(event.into_owned())));
        }

        let bytes_read = try_ready!(self.fd.poll_read(&mut self.buffer));

        if bytes_read == 0 {
            return Ok(Async::Ready(None));
        }

        let (step, event) = Event::from_buffer(
            Arc::downgrade(self.fd.get_ref()),
            &self.buffer,
        );
        self.pos = step;
        self.size = bytes_read - step;

        Ok(Async::Ready(Some(event.into_owned())))
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
