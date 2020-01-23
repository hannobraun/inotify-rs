<a name="v0.8.1"></a>
### v0.8.1 (2020-01-23)

- Ensure file descriptor is closed on drop ([#140])

[#140]: https://github.com/inotify-rs/inotify/pull/140


<a name="v0.8.0"></a>
### v0.8.0 (2019-12-04)

- Update to tokio 0.2 and futures 0.3 ([#134])

[#134]: https://github.com/inotify-rs/inotify/pull/134


<a name="v0.7.0"></a>
### v0.7.0 (2019-02-09)

#### Features

* Make stream API more flexible in regards to buffers ([ea3e7a394bf34a6ccce4f2136c0991fe7e8f1f42](ea3e7a394bf34a6ccce4f2136c0991fe7e8f1f42)) (breaking change)

<a name="v0.6.1"></a>
### v0.6.1 (2018-08-28)


#### Bug Fixes

*   Don't return spurious filenames ([2f37560f](2f37560f))



<a name="v0.6.0"></a>
## v0.6.0 (2018-08-16)


#### Features

*   Handle closing of inotify instance better ([824160fe](824160fe))
*   Implement `EventStream` using `mio` ([ba4cb8c7](ba4cb8c7))



<a name="v0.5.1"></a>
### v0.5.1 (2018-02-27)

#### Features

*   Add future-based async API ([569e65a7](569e65a7), closes [#49](49))



