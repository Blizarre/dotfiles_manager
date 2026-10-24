# Dotfiles Manager

A small program to manage my dotfiles and keep them in sync between all my computers.
- must be _very_ simple to install and deploy:
    - Shipped as a single static binary (installable anywhere by a simple `curl`)
    - Small-ish binary (less than 2MB)
    - single command to configure the program (optional, can also use environment variables)
    - single command to pull the files on a new computer
    - single command to add files/sync to/from the remote
- Storage engine: WebDAV
- Versioning is outside of the scope of this tool
- Basic conflict reconciliation by printing diffs
- CI pipeline to autogenerate Linux binaries (x86_64 and aarch64)
- Designed for a moderate (<100) number of small (<1MB) text files.

**It is currently a beta that I built to define the APIs and figure out how to deal with the corner cases**.
It has all the basic features. I plan to use it for a while and polish it.

## Storage backends

The remote is configured with a WebDAV collection URL, for example
`https://user:password@server.com/prefix/`. Credentials are optional and must be
URL-encoded; the trailing slash indicates the collection that contains the
tracked files. The URL can be supplied either in the config file (`~/.dots` by
default), created with `dotfile configure`, or with the `DOT_URL` environment
variable, which overrides the configured URL.

The config file is TOML and supports these fields:

```toml
# WebDAV collection URL
url = "https://user:password@server.com/prefix/"
# Directory matched with the remote. Defaults to the home directory.
root_dir = "/home/test"

# Optional extra headers sent with every WebDAV request
[[extra_headers]]
key = "X-Custom-Header"
value = "value"
```

`dotfile configure` also accepts an optional root directory:
`dotfile configure <url> <root_dir>`.

## Examples

### Fetch config on a new computer (no config, public endpoint, raspberry pi)
```
$ curl -L https://github.com/Blizarre/dotfiles_manager/releases/download/0.3.1/dotfile.aarch64 -o /tmp/dotfile
$ chmod +x /tmp/dotfile
$ DOT_URL=https://server.com/prefix/ /tmp/dotfile sync
INFO  [dotfile] Listing files
INFO  [dotfile]     Identical content, skipping: .config/fish/config.fish
INFO  [dotfile]     Identical content, skipping: .config/fish/functions/c.fish
INFO  [dotfile]     Identical content, skipping: .config/fish/functions/ff.fish
...
```

### Sync on a new computer (with config, and URL authentication)

```
$ curl -L https://github.com/Blizarre/dotfiles_manager/releases/download/0.3.1/dotfile -o ~/bin/dotfile
$ chmod +x ~/bin/dotfile
$ dotfile configure https://user:password@server.com/prefix/
INFO  [dotfile] New configuration saved in /home/simon/.dots
$ dotfile sync
[INFO ] Listing files
[INFO ]     Identical content, skipping: .config/fish/conf.d/rustup.fish
[INFO ]     Identical content, skipping: .config/fish/functions/c.fish
[INFO ]     Identical content, skipping: .config/fish/functions/ff.fish
[INFO ]     Identical content, skipping: .config/fish/functions/fish_prompt.fish
...
```

NOTE: The `DOT_URL` environment variable can be provided during a sync as well instead of calling `configure`.

### Track/forget a file (require authentication)
```
$ curl -L https://github.com/Blizarre/dotfiles_manager/releases/download/0.3.1/dotfile -o ~/bin/dotfile
$ chmod +x ~/bin/dotfile
$ dotfile configure https://user:password@server.com/prefix/
INFO  [dotfile] New configuration saved in /home/simon/.dots
$ dotfile track ~/.bashrc
INFO  [dotfile] Uploading /home/simon/.bashrc to .bashrc
$ dotfile forget ~/.bashrc
INFO  [dotfile] The file .bashrc has been removed
```
