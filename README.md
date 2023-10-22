# Dotfiles Manager

A small program to manage my dotfiles and keep them in sync between all my computers.
- must be _very_ simple to install and deploy:
    - Shipped as a single static binary (installable anywhere by a simple `curl`)
    - Small binary (less than 2MB)
    - single command to configure the program
    - single command to pull the files on a new computer
    - single command to add files/sync to/from the remote
- Storage engine: S3. _Ideally from a public bucket so that no credentials are required to fetch files_. Authentication is however possible (at least for uploads)
- Versioning is outside of the scope of this tool (can be handled by S3)
- Basic conflict reconciliation by printing diffs
- CI pipeline to autogenerate windows / mac / linux binaries

It is currently a prototype that I use to sketch the necessary features:
- Need to revamp error handling (handle them at the top of the stack instead of using expect everywhere)
- Need to improve support for AWS credentials
- Need to move to async to handle more files (will currently check them sequentially)
- See if I can avoid downloading the files to check for diffs
- Major refactoring to clean it up
- Unit tests

## How to build statically

```
$ cargo build --release --target x86_64-unknown-linux-musl
$ upx --brute --no-lzma target/x86_64-unknown-linux-musl/release/dotfile
$ ls -lh target/x86_64-unknown-linux-musl/release/dotfile
-rwxr-xr-x 1 simon simon 1.3M Oct 22 21:10 target/x86_64-unknown-linux-musl/release/dotfile*
$ ldd target/x86_64-unknown-linux-musl/release/dotfile
        not a dynamic executable
```
