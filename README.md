# inotify-rs [![Build Status](https://travis-ci.org/hannobraun/inotify-rs.svg?branch=master)](https://travis-ci.org/hannobraun/inotify-rs)

## What is inotify-rs?

It consists of two things:
- [inotify](http://en.wikipedia.org/wiki/Inotify) bindings for the
  [Rust programming language](http://rust-lang.org/)
- An idiomatic Rust wrapper for those bindings


## Is it any good?

Yes.

The bindings are complete (after all, inotify isn't that big of an API). The
idiomatic wrapper needs some work, but is already useful as it is.


## How do I use it?

Include it in your Cargo.toml:
```toml
[dependencies]
inotify = "*"
```

And here's a little example:
```Rust
extern crate inotify;

use inotify::INotify;
use inotify::ffi::*;
use std::path::Path;

fn main() {
    let mut ino = INotify::init().unwrap();

    ino.add_watch(Path::new("/home"), IN_MODIFY | IN_CREATE | IN_DELETE).unwrap();
    loop {
        let events = ino.wait_for_events().unwrap();

        for event in events.iter() {
            if event.is_create() {
                if event.is_dir() {
                    println!("The directory \"{}\" was created.", event.name);       
                } else {
                    println!("The file \"{}\" was created.", event.name);
                }
            } else if event.is_delete() {
                if event.is_dir() {
                    println!("The directory \"{}\" was deleted.", event.name);       
                } else {
                    println!("The file \"{}\" was deleted.", event.name);
                }
            } else if event.is_modify() {
                if event.is_dir() {
                    println!("The directory \"{}\" was modified.", event.name);
                } else {
                    println!("The file \"{}\" was modified.", event.name);
                }
            }
        }
    }
}
```

## Any documentation?

The binding is fully documented, but because inotify usage is subject to
various caveats, warnings, and recommendations to build a robust and
efficient application, programmers should read through the [inotify(7)]
man page.

The wrapper is not documented at this time. (But pull requests are appreciated!)

[inotify(7)]: http://man7.org/linux/man-pages/man7/inotify.7.html


## What's the license?

Copyright (c) 2014, Hanno Braun and contributors

Permission to use, copy, modify, and/or distribute this software for any purpose
with or without fee is hereby granted, provided that the above copyright notice
and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH
REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND
FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT,
INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS
OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER
TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF
THIS SOFTWARE.
