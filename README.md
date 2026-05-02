# git-future

An async-first library for reading git repositories

## Usage

The library is agnostic as to the async runtime in use, so consumers must
implement a couple of traits that provide filesystem operations. See the
[`file_system`] module for further details.

For example, these could use Tokio, or the web filesystem API using
wasm-bindgen's support for transforming JS promises to Rust futures. A dummy
implementation could use the Rust standard library's synchronous filesystem
operations.

A future goal is to provide some standard implementations for commonly-used
async runtimes.

The main entry point is the [`Repo`] object, which represents a git
repository. Refs and objects are looked up via methods on [`Repo`].

## Example
```rust
let foo: u8 = 0;
```

## Caveats

- Read only
- Diff is slow

[`file_system`]: https://docs.rs/git-future/latest/git_future/file_system/index.html
[`Repo`]: https://docs.rs/git-future/latest/git_future/struct.Repo.html
