# git-async

An async-first Rust library for reading git repositories

## Usage

The main entry point is the [`Repo`] object, which represents a git
repository. Refs and objects are looked up via methods on [`Repo`].

The library is agnostic as to the async runtime in use, so consumers must
implement a couple of traits that provide filesystem operations. See the
[`file_system`] module for further details.

For example, these could use Tokio, or the web filesystem API using
wasm-bindgen's support for transforming JS promises to Rust futures. A dummy
implementation could use the Rust standard library's synchronous filesystem
operations.

A future goal is to provide some standard implementations for commonly-used
async runtimes.

## Example

```rust
async fn example() -> GResult<()> {
    let repo = Repo::<MyFS>::open(MyDirectory::new("a-repository")).await?;
    let head = repo.head().await?;
    let commit = head.peel_to_commit(&repo).await?.unwrap();
    let message = commit.message();
    println!("{}", str::from_utf8(message).unwrap());
    Ok(())
}
```

## Caveats

There are a few things this crate cannot (yet) do. They are in scope, so
future versions may support them, but for now they are not implemented.

- It only supports read operations on git repositories
- It ignores the working tree. Only operations involving the actual
  repository structure are supported
- The diff algorithm is naive and quite slow. I have not looked into how
  `git diff` manages to be so fast, but I imagine it uses the packfile delta
  encoding somehow to optimize diffing.

[`file_system`]: https://docs.rs/git-async/latest/git_async/file_system/index.html
[`Repo`]: https://docs.rs/git-async/latest/git_async/struct.Repo.html
